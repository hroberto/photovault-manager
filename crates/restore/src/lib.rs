//! Restauração do cofre para um destino.
//!
//! Implementa o ADR-009. A restauração não é recurso acessório: é o teste do produto inteiro.
//! Um backup que nunca foi restaurado não é um backup, é uma esperança.
//!
//! Três propriedades que o desenho precisa garantir, porque um trabalho de seis dias **vai** ser
//! interrompido:
//!
//! 1. **Idempotência.** Reiniciar não pode significar reenviar. A chave é consultada antes de
//!    gastar qualquer requisição.
//! 2. **Orçamento.** A cota de 10.000 requisições diárias é contada antes, não descoberta num
//!    429 no meio do caminho.
//! 3. **Honestidade.** O que não volta — nomes de pessoas, favoritos — é declarado antes de
//!    começar, a partir de `SinkCapabilities`, nunca escrito à mão.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use photovault_core::{MediaId, ObjectHash, SinkCapabilities};

pub mod plan;

pub use plan::{RestorePlan, RestoreSelection};

/// Calcula a chave de idempotência de um item.
///
/// Combina conta, objeto, destino e execução. Conta e objeto são o essencial: enviar os mesmos
/// bytes para a mesma conta duas vezes cria duas cópias na biblioteca do usuário, e **não existe
/// API para descobrir isso depois** — a Library API só enxerga o que o próprio aplicativo criou,
/// e nem isso lista a biblioteca inteira. A proteção é inteiramente local.
pub fn idempotency_key(account: &str, object: &ObjectHash, sink: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [account, &object.to_hex(), sink] {
        hasher.update(field.as_bytes());
        hasher.update(b"\x1f");
    }
    hasher.finalize().to_hex().to_string()
}

/// O que se perde ao enviar para este destino, em texto pronto para a tela.
///
/// Derivado de `SinkCapabilities` (ADR-010). Quando a Google mudar a API, isto muda sozinho.
pub fn declared_losses(capabilities: &SinkCapabilities) -> Vec<String> {
    capabilities
        .losses()
        .into_iter()
        .map(|loss| loss.describe().to_owned())
        .collect()
}

/// Um item pronto para subir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// Item no catálogo.
    pub media: MediaId,
    /// Bytes que serão enviados.
    pub object: ObjectHash,
    /// Nome do arquivo.
    pub filename: String,
    /// Descrição.
    pub description: Option<String>,
    /// Tamanho em bytes.
    pub size: u64,
    /// Se este item tem geolocalização a preservar.
    ///
    /// Quando verdadeiro e o destino exige EXIF embutido, o arquivo precisa ter passado pela
    /// normalização — senão a localização se perde em silêncio.
    pub has_location: bool,
    /// Chave de idempotência.
    pub idempotency_key: String,
}

impl PlannedItem {
    /// Se o item excede o tamanho que o destino aceita.
    pub fn exceeds_limit(&self, capabilities: &SinkCapabilities, is_video: bool) -> bool {
        let limit = if is_video {
            capabilities.max_video_bytes
        } else {
            capabilities.max_photo_bytes
        };
        self.size > limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(seed: u8) -> ObjectHash {
        ObjectHash::from_bytes([seed; 32])
    }

    fn item(size: u64) -> PlannedItem {
        PlannedItem {
            media: MediaId::new(1),
            object: hash(1),
            filename: "IMG_1002.JPG".into(),
            description: None,
            size,
            has_location: true,
            idempotency_key: "k".into(),
        }
    }

    #[test]
    fn the_same_object_to_the_same_account_has_the_same_key() {
        let a = idempotency_key("conta@exemplo.com", &hash(1), "google_photos");
        let b = idempotency_key("conta@exemplo.com", &hash(1), "google_photos");
        assert_eq!(a, b, "reiniciar não pode significar reenviar");
    }

    #[test]
    fn different_accounts_get_different_keys() {
        // Migrar de conta é um caso de uso legítimo: o mesmo item precisa poder subir na conta
        // nova sem que a chave da conta antiga o bloqueie.
        let a = idempotency_key("antiga@exemplo.com", &hash(1), "google_photos");
        let b = idempotency_key("nova@exemplo.com", &hash(1), "google_photos");
        assert_ne!(a, b);
    }

    #[test]
    fn different_objects_get_different_keys() {
        let a = idempotency_key("h@g.com", &hash(1), "google_photos");
        let b = idempotency_key("h@g.com", &hash(2), "google_photos");
        assert_ne!(a, b);
    }

    #[test]
    fn different_sinks_get_different_keys() {
        let a = idempotency_key("h@g.com", &hash(1), "google_photos");
        let b = idempotency_key("h@g.com", &hash(1), "local_folder");
        assert_ne!(a, b);
    }

    #[test]
    fn field_separator_prevents_collisions() {
        // Sem separador, ("ab", "c") e ("a", "bc") colidiriam.
        let a = idempotency_key("ab", &hash(1), "c");
        let b = idempotency_key("a", &hash(1), "bc");
        assert_ne!(a, b);
    }

    #[test]
    fn google_losses_are_people_and_favorites() {
        let losses = declared_losses(&SinkCapabilities::GOOGLE_PHOTOS);
        assert_eq!(losses.len(), 2);
        assert!(losses.iter().any(|l| l.contains("pessoas")));
        assert!(losses.iter().any(|l| l.contains("favorito")));
        // A geolocalização NÃO está na lista: o Google a lê do EXIF embutido.
        assert!(!losses.iter().any(|l| l.contains("geolocalização")));
    }

    #[test]
    fn local_folder_declares_no_losses() {
        assert!(declared_losses(&SinkCapabilities::LOCAL_FOLDER).is_empty());
    }

    #[test]
    fn photo_size_limit_is_enforced() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        assert!(!item(199 * 1024 * 1024).exceeds_limit(&google, false));
        assert!(item(201 * 1024 * 1024).exceeds_limit(&google, false));
    }

    #[test]
    fn videos_get_a_larger_limit() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        let big = item(19 * 1024 * 1024 * 1024);
        assert!(big.exceeds_limit(&google, false), "grande demais como foto");
        assert!(!big.exceeds_limit(&google, true), "cabe como vídeo");
    }

    #[test]
    fn local_folder_has_no_size_limit() {
        assert!(!item(u64::MAX - 1).exceeds_limit(&SinkCapabilities::LOCAL_FOLDER, false));
    }
}
