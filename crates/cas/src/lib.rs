//! Content Addressable Storage.
//!
//! Um objeto é identificado pelos seus bytes, não pelo seu nome. Disso saem três propriedades
//! que o projeto inteiro aproveita: a mesma foto em quatro álbuns ocupa espaço uma vez, a
//! deduplicação exata é consequência do armazenamento em vez de uma varredura posterior, e a
//! integridade é verificável a qualquer momento.
//!
//! A regra que sustenta tudo (ADR-004): **objetos nunca são modificados**. Escrever metadado
//! corrigido de volta no objeto mudaria o hash e destruiria a identidade. Por isso a
//! normalização produz cópias em `derived/`, e aqui os arquivos são gravados sem permissão de
//! escrita — a imutabilidade é física, não só documentada.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use photovault_core::ObjectHash;

/// Tamanho do buffer de streaming.
///
/// Um vídeo de 20 GB nunca passa inteiro pela memória: lemos em blocos e alimentamos o hash e o
/// arquivo de destino no mesmo passe, uma leitura só.
const BUFFER_SIZE: usize = 1024 * 1024;

/// Repositório de objetos em disco.
#[derive(Debug, Clone)]
pub struct ObjectStore {
    objects: PathBuf,
    staging: PathBuf,
}

/// O que aconteceu ao guardar um objeto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreOutcome {
    /// Identidade dos bytes.
    pub hash: ObjectHash,
    /// Tamanho em bytes.
    pub size: u64,
    /// Se o objeto já existia e nada foi escrito.
    ///
    /// É assim que a deduplicação exata sai de graça: o mesmo arquivo visto em quatro álbuns
    /// resulta em três `true`.
    pub deduplicated: bool,
}

/// Falha ao operar sobre o repositório.
#[derive(Debug, thiserror::Error)]
pub enum CasError {
    /// Erro de entrada e saída, com o caminho envolvido.
    #[error("{operation} em {path}: {source}")]
    Io {
        /// O que estava sendo feito.
        operation: &'static str,
        /// Caminho envolvido.
        path: PathBuf,
        /// Causa.
        source: io::Error,
    },
    /// O objeto pedido não existe no repositório.
    #[error("objeto {0} não está no repositório")]
    Missing(String),
    /// Os bytes em disco não correspondem mais ao hash que os nomeia.
    #[error("objeto {expected} está corrompido: os bytes agora resultam em {actual}")]
    Corrupted {
        /// Hash que dá nome ao arquivo.
        expected: String,
        /// Hash calculado agora.
        actual: String,
    },
}

type Result<T> = std::result::Result<T, CasError>;

fn io_err(operation: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> CasError {
    let path = path.into();
    move |source| CasError::Io {
        operation,
        path,
        source,
    }
}

impl ObjectStore {
    /// Abre (ou cria) um repositório na raiz indicada.
    ///
    /// A área de staging fica dentro da mesma raiz de propósito: `rename` só é atômico dentro
    /// do mesmo sistema de arquivos, e um temporário em `/tmp` costuma estar em outro.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let objects = root.join("objects");
        let staging = root.join("staging");
        for dir in [&objects, &staging] {
            fs::create_dir_all(dir).map_err(io_err("criar diretório", dir))?;
        }
        Ok(Self { objects, staging })
    }

    /// Caminho de um objeto, exista ele ou não.
    ///
    /// O fanout de dois caracteres evita dezenas de milhares de entradas em um único diretório.
    pub fn path_for(&self, hash: &ObjectHash) -> PathBuf {
        self.objects.join(hash.fanout()).join(hash.to_hex())
    }

    /// Se o objeto já está guardado.
    pub fn contains(&self, hash: &ObjectHash) -> bool {
        self.path_for(hash).is_file()
    }

    /// Guarda os bytes de um leitor.
    pub fn store_reader<R: Read>(&self, reader: R) -> Result<StoreOutcome> {
        let temp = tempfile::NamedTempFile::new_in(&self.staging)
            .map_err(io_err("criar arquivo temporário", &self.staging))?;
        let (hash, size) = stream_into(reader, temp.as_file())?;

        let destination = self.path_for(&hash);
        if destination.is_file() {
            // Já temos estes bytes. O temporário some ao sair de escopo.
            return Ok(StoreOutcome {
                hash,
                size,
                deduplicated: true,
            });
        }

        let parent = destination.parent().unwrap_or(&self.objects).to_owned();
        fs::create_dir_all(&parent).map_err(io_err("criar diretório de fanout", &parent))?;

        // Durabilidade antes de tornar visível: os dados precisam estar no disco antes de o
        // nome definitivo existir, ou uma queda de energia deixa um objeto vazio com nome de
        // objeto válido.
        temp.as_file()
            .sync_all()
            .map_err(io_err("sincronizar temporário", &self.staging))?;
        temp.persist(&destination).map_err(|error| CasError::Io {
            operation: "mover para o repositório",
            path: destination.clone(),
            source: error.error,
        })?;

        set_read_only(&destination)?;
        sync_dir(&parent)?;

        Ok(StoreOutcome {
            hash,
            size,
            deduplicated: false,
        })
    }

    /// Guarda um arquivo do disco.
    pub fn store_file(&self, source: impl AsRef<Path>) -> Result<StoreOutcome> {
        let source = source.as_ref();
        let file = File::open(source).map_err(io_err("abrir arquivo de origem", source))?;
        self.store_reader(file)
    }

    /// Abre um objeto para leitura.
    pub fn open_object(&self, hash: &ObjectHash) -> Result<File> {
        let path = self.path_for(hash);
        if !path.is_file() {
            return Err(CasError::Missing(hash.to_hex()));
        }
        File::open(&path).map_err(io_err("abrir objeto", path))
    }

    /// Tamanho de um objeto guardado.
    pub fn size_of(&self, hash: &ObjectHash) -> Result<u64> {
        let path = self.path_for(hash);
        let meta = fs::metadata(&path).map_err(io_err("ler metadados do objeto", path))?;
        Ok(meta.len())
    }

    /// Relê o objeto do disco e confere que os bytes ainda resultam no mesmo hash.
    ///
    /// É isto que separa `STORED` de `VERIFIED`, e é o passo do scrub periódico. Atenção ao que
    /// a verificação significa: integridade **local**. Não existe hash de referência do lado do
    /// Google para comparar.
    pub fn verify(&self, hash: &ObjectHash) -> Result<()> {
        let file = self.open_object(hash)?;
        let (actual, _) = stream_into(file, io::sink())?;
        if actual == *hash {
            Ok(())
        } else {
            Err(CasError::Corrupted {
                expected: hash.to_hex(),
                actual: actual.to_hex(),
            })
        }
    }

    /// Percorre todos os objetos guardados.
    pub fn iter_objects(&self) -> Result<Vec<ObjectHash>> {
        let mut found = Vec::new();
        let fanouts = match fs::read_dir(&self.objects) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
            Err(error) => return Err(io_err("listar repositório", &self.objects)(error)),
        };
        for fanout in fanouts {
            let fanout = fanout.map_err(io_err("listar repositório", &self.objects))?;
            if !fanout.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let dir = fanout.path();
            for entry in fs::read_dir(&dir).map_err(io_err("listar fanout", &dir))? {
                let entry = entry.map_err(io_err("listar fanout", &dir))?;
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(hash) = ObjectHash::from_hex(name) {
                        found.push(hash);
                    }
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

/// Copia do leitor para o escritor calculando o hash no mesmo passe.
fn stream_into<R: Read, W: Write>(mut reader: R, mut writer: W) -> Result<(ObjectHash, u64)> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut total = 0u64;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(io_err("ler bytes", PathBuf::from("<stream>")))?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        hasher.update(chunk);
        writer
            .write_all(chunk)
            .map_err(io_err("escrever bytes", PathBuf::from("<stream>")))?;
        total += read as u64;
    }
    writer
        .flush()
        .map_err(io_err("descarregar buffer", PathBuf::from("<stream>")))?;

    Ok((ObjectHash::from_hash(hasher.finalize()), total))
}

/// Remove a permissão de escrita do objeto recém-guardado.
#[cfg(unix)]
fn set_read_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = fs::Permissions::from_mode(0o444);
    fs::set_permissions(path, permissions).map_err(io_err("tornar objeto somente leitura", path))
}

#[cfg(not(unix))]
fn set_read_only(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .map_err(io_err("ler permissões", path))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(io_err("tornar objeto somente leitura", path))
}

/// Garante que a entrada de diretório criada pelo `rename` chegou ao disco.
#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    let dir = File::open(path).map_err(io_err("abrir diretório", path))?;
    dir.sync_all()
        .map_err(io_err("sincronizar diretório", path))
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ObjectStore) {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let store = ObjectStore::open(dir.path()).expect("abre repositório");
        (dir, store)
    }

    #[test]
    fn stores_and_reads_back() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"uma foto"[..]).expect("guarda");
        assert!(!outcome.deduplicated);
        assert_eq!(outcome.size, 8);

        let mut content = Vec::new();
        store
            .open_object(&outcome.hash)
            .expect("abre")
            .read_to_end(&mut content)
            .expect("lê");
        assert_eq!(content, b"uma foto");
    }

    #[test]
    fn identical_bytes_are_stored_once() {
        let (_dir, store) = store();
        let first = store.store_reader(&b"mesma foto"[..]).expect("guarda");
        let second = store
            .store_reader(&b"mesma foto"[..])
            .expect("guarda de novo");

        assert_eq!(first.hash, second.hash);
        assert!(!first.deduplicated);
        // A mesma foto em quatro álbuns ocupa espaço uma vez.
        assert!(second.deduplicated);
        assert_eq!(store.iter_objects().expect("lista").len(), 1);
    }

    #[test]
    fn different_bytes_are_different_objects() {
        let (_dir, store) = store();
        let a = store.store_reader(&b"foto A"[..]).expect("guarda");
        let b = store.store_reader(&b"foto B"[..]).expect("guarda");
        assert_ne!(a.hash, b.hash);
        assert_eq!(store.iter_objects().expect("lista").len(), 2);
    }

    #[test]
    fn hash_matches_blake3_of_content() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"conteudo"[..]).expect("guarda");
        assert_eq!(
            outcome.hash,
            ObjectHash::from_hash(blake3::hash(b"conteudo"))
        );
    }

    #[test]
    fn layout_uses_fanout_directory() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"foto"[..]).expect("guarda");
        let path = store.path_for(&outcome.hash);
        let parent = path.parent().expect("tem fanout");
        assert_eq!(
            parent.file_name().expect("nome"),
            outcome.hash.fanout().as_str()
        );
        assert_eq!(
            path.file_name().expect("nome"),
            outcome.hash.to_hex().as_str()
        );
    }

    #[test]
    fn stored_objects_are_immutable() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"irrepetivel"[..]).expect("guarda");
        let path = store.path_for(&outcome.hash);

        // ADR-004 aplicado pelo sistema de arquivos, não só pela documentação.
        let attempt = fs::OpenOptions::new().write(true).open(&path);
        assert!(
            attempt.is_err(),
            "objeto no CAS não pode ser aberto para escrita"
        );
    }

    #[test]
    fn verify_passes_for_intact_object() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"integro"[..]).expect("guarda");
        assert!(store.verify(&outcome.hash).is_ok());
    }

    #[test]
    fn verify_detects_corruption() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b"sera corrompido"[..]).expect("guarda");
        let path = store.path_for(&outcome.hash);

        // Simula corrupção silenciosa de disco: reabre com permissão e troca os bytes.
        let mut permissions = fs::metadata(&path).expect("metadados").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o644);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).expect("libera escrita");
        fs::write(&path, b"bytes trocados").expect("corrompe");

        match store.verify(&outcome.hash) {
            Err(CasError::Corrupted { .. }) => {}
            other => panic!("corrupção deveria ser detectada, veio {other:?}"),
        }
    }

    #[test]
    fn missing_object_is_reported() {
        let (_dir, store) = store();
        let absent = ObjectHash::from_bytes([9; 32]);
        assert!(!store.contains(&absent));
        match store.verify(&absent) {
            Err(CasError::Missing(_)) => {}
            other => panic!("esperava ausência, veio {other:?}"),
        }
    }

    #[test]
    fn handles_content_larger_than_the_buffer() {
        let (_dir, store) = store();
        // Três blocos e um pedaço, para exercitar o laço de streaming.
        let big: Vec<u8> = (0..BUFFER_SIZE * 3 + 17).map(|i| (i % 251) as u8).collect();
        let outcome = store.store_reader(big.as_slice()).expect("guarda");
        assert_eq!(outcome.size, big.len() as u64);
        assert!(store.verify(&outcome.hash).is_ok());
        assert_eq!(
            store.size_of(&outcome.hash).expect("tamanho"),
            big.len() as u64
        );
    }

    #[test]
    fn empty_content_is_valid() {
        let (_dir, store) = store();
        let outcome = store.store_reader(&b""[..]).expect("guarda vazio");
        assert_eq!(outcome.size, 0);
        assert!(store.verify(&outcome.hash).is_ok());
    }

    #[test]
    fn stores_from_a_file_path() {
        let (dir, store) = store();
        let source = dir.path().join("origem.jpg");
        fs::write(&source, b"do disco").expect("escreve origem");

        let outcome = store.store_file(&source).expect("guarda do disco");
        assert_eq!(
            outcome.hash,
            ObjectHash::from_hash(blake3::hash(b"do disco"))
        );
        // O original permanece onde estava: guardar não é mover.
        assert!(source.is_file());
    }

    #[test]
    fn staging_is_left_clean() {
        let (dir, store) = store();
        store.store_reader(&b"foto"[..]).expect("guarda");
        store.store_reader(&b"foto"[..]).expect("deduplicado");

        let leftovers: Vec<_> = fs::read_dir(dir.path().join("staging"))
            .expect("lê staging")
            .collect();
        assert!(leftovers.is_empty(), "staging deveria ficar vazio");
    }

    #[test]
    fn reopening_sees_existing_objects() {
        let dir = tempfile::tempdir().expect("diretório");
        let hash = {
            let store = ObjectStore::open(dir.path()).expect("abre");
            store
                .store_reader(&b"persistente"[..])
                .expect("guarda")
                .hash
        };
        let reopened = ObjectStore::open(dir.path()).expect("reabre");
        assert!(reopened.contains(&hash));
        assert_eq!(reopened.iter_objects().expect("lista"), vec![hash]);
    }
}
