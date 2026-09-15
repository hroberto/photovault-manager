//! Tipos de entrada e saída do catálogo.

use photovault_core::{MediaKind, ObjectHash, SourceKind};

/// Identidade de uma importação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportRunId(pub(crate) i64);

impl ImportRunId {
    /// Valor bruto, para relatórios.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Identidade de uma pessoa no catálogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonId(pub(crate) i64);

impl PersonId {
    /// Valor bruto.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Identidade de um álbum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlbumId(pub(crate) i64);

impl AlbumId {
    /// Valor bruto.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Um item a ser catalogado.
#[derive(Debug, Clone)]
pub struct NewMedia {
    /// Hash dos bytes já guardados no CAS.
    pub object: ObjectHash,
    /// Nome do arquivo como veio da origem.
    pub filename: String,
    /// Natureza do item.
    pub kind: MediaKind,
    /// Fonte de entrada, que determina a fidelidade.
    pub source: SourceKind,
    /// Quando a foto foi tirada, em epoch UTC.
    pub captured_at: Option<i64>,
    /// Quando entrou no Google, em epoch UTC.
    pub uploaded_at: Option<i64>,
    /// Descrição escrita pelo usuário.
    pub description: Option<String>,
    /// Marcada como favorita no Google.
    pub favorited: bool,
    /// Link para o item no Google Fotos.
    pub google_url: Option<String>,
    /// Importação que trouxe este item.
    pub import_run: ImportRunId,
}

/// Contagens de uma importação.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportTally {
    /// Arquivos de mídia encontrados.
    pub media_seen: u64,
    /// Itens novos catalogados.
    pub media_imported: u64,
    /// Arquivos cujos bytes já estavam no cofre.
    pub media_deduplicated: u64,
    /// Sidecars associados com sucesso.
    pub sidecars_matched: u64,
    /// Sidecars sem dono, na fila de revisão.
    pub sidecars_orphan: u64,
    /// Arquivos de mídia sem sidecar.
    pub media_without_sidecar: u64,
    /// Bytes efetivamente escritos no CAS.
    pub bytes_stored: u64,
    /// Arquivos que falharam ao ser lidos ou guardados.
    pub failures: u64,
}

/// Um sidecar aguardando revisão humana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanRow {
    /// Identidade na fila.
    pub id: i64,
    /// Diretório onde o sidecar estava, relativo à raiz do archive.
    pub directory: String,
    /// Nome do arquivo.
    pub sidecar: String,
    /// Por que não foi associado.
    pub reason: String,
    /// Arquivos que poderiam ser o dono, quando o caso foi de ambiguidade.
    pub candidates: Vec<String>,
}

/// Números gerais do cofre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogStats {
    /// Itens lógicos vivos.
    pub media: u64,
    /// Objetos distintos no CAS.
    pub objects: u64,
    /// Soma dos tamanhos dos objetos.
    pub bytes: u64,
    /// Álbuns conhecidos.
    pub albums: u64,
    /// Pessoas conhecidas.
    pub people: u64,
    /// Itens com geolocalização.
    pub located: u64,
    /// Objetos verificados por releitura.
    pub verified: u64,
    /// Sidecars órfãos aguardando revisão.
    pub pending_orphans: u64,
}

impl CatalogStats {
    /// Tamanho legível, para relatórios de linha de comando.
    pub fn human_bytes(&self) -> String {
        human_bytes(self.bytes)
    }
}

/// Formata bytes em unidade legível.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(human_bytes(390 * 1024 * 1024 * 1024), "390.0 GB");
    }
}
