//! Planejamento de uma restauração.
//!
//! O plano existe para que nada comece sem que o usuário saiba o tamanho do que pediu: quantos
//! itens, quantos bytes, quantas requisições, quantos dias, o que se perde no caminho e quanto
//! de cota de armazenamento do Google será consumida.
//!
//! Esse último aviso é o mais importante e o mais fácil de esquecer: reenviar 390 GB consome
//! 390 GB da conta **de novo** se os originais ainda estiverem lá.

use photovault_core::SinkCapabilities;

use crate::PlannedItem;

/// O que restaurar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreSelection {
    /// O acervo inteiro.
    Everything,
    /// Um álbum, pelo título.
    Album(String),
    /// Uma amostra aleatória, para a verificação periódica do ADR-009.
    Sample(usize),
}

impl RestoreSelection {
    /// Descrição para a tela de confirmação.
    pub fn describe(&self) -> String {
        match self {
            Self::Everything => "todo o acervo".into(),
            Self::Album(title) => format!("o álbum \"{title}\""),
            Self::Sample(size) => format!("uma amostra de {size} itens"),
        }
    }
}

/// Um plano pronto para revisão.
#[derive(Debug, Clone)]
pub struct RestorePlan {
    /// O que foi pedido.
    pub selection: RestoreSelection,
    /// Destino.
    pub sink: String,
    /// Capacidades do destino.
    pub capabilities: SinkCapabilities,
    /// Itens que vão subir.
    pub items: Vec<PlannedItem>,
    /// Itens recusados antes de começar, com o motivo.
    pub rejected: Vec<(String, String)>,
    /// Quantos álbuns serão recriados.
    pub albums: u64,
    /// Quantas associações item-álbum serão feitas.
    pub album_memberships: u64,
}

impl RestorePlan {
    /// Soma dos bytes que vão trafegar.
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|item| item.size).sum()
    }

    /// Quantas requisições o trabalho consome.
    pub fn requests(&self) -> u64 {
        self.capabilities.estimate_requests(
            self.items.len() as u64,
            self.album_memberships,
            self.albums,
        )
    }

    /// Quantos dias, dada a cota do destino.
    pub fn days(&self) -> Option<u32> {
        self.capabilities.estimated_days(self.requests())
    }

    /// O que não será preservado neste destino.
    pub fn losses(&self) -> Vec<String> {
        crate::declared_losses(&self.capabilities)
    }

    /// Itens que precisam de normalização antes de subir.
    ///
    /// Quando o destino só entende geolocalização pelo EXIF embutido — que é o caso do Google —
    /// enviar o objeto cru perde a localização sem avisar ninguém.
    pub fn needing_normalization(&self) -> usize {
        if !self.capabilities.requires_embedded_geo() {
            return 0;
        }
        self.items.iter().filter(|item| item.has_location).count()
    }

    /// Se o plano pode ser executado como está.
    pub fn blocker(&self) -> Option<String> {
        if self.items.is_empty() {
            return Some("a seleção não resultou em nenhum item".into());
        }
        None
    }

    /// Aviso de consumo de armazenamento no destino.
    ///
    /// Reenviar não é de graça: o Google cobra a cota de novo se os originais ainda estiverem lá.
    pub fn storage_warning(&self) -> Option<String> {
        let bytes = self.bytes();
        if bytes == 0 {
            return None;
        }
        Some(format!(
            "Serão consumidos {} da cota de armazenamento do destino.",
            human_bytes(bytes)
        ))
    }
}

/// Formata bytes em unidade legível.
fn human_bytes(bytes: u64) -> String {
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
    use photovault_core::{MediaId, ObjectHash};

    fn item(index: u8, size: u64, has_location: bool) -> PlannedItem {
        PlannedItem {
            media: MediaId::new(i64::from(index)),
            object: ObjectHash::from_bytes([index; 32]),
            filename: format!("IMG_{index}.JPG"),
            description: None,
            size,
            has_location,
            idempotency_key: format!("k{index}"),
        }
    }

    fn plan(items: Vec<PlannedItem>, albums: u64, memberships: u64) -> RestorePlan {
        RestorePlan {
            selection: RestoreSelection::Everything,
            sink: "google_photos".into(),
            capabilities: SinkCapabilities::GOOGLE_PHOTOS,
            items,
            rejected: Vec::new(),
            albums,
            album_memberships: memberships,
        }
    }

    #[test]
    fn empty_selection_is_blocked() {
        assert!(plan(Vec::new(), 0, 0).blocker().is_some());
        assert!(plan(vec![item(1, 100, false)], 0, 0).blocker().is_none());
    }

    #[test]
    fn a_single_album_finishes_the_same_day() {
        let items: Vec<PlannedItem> = (0..200).map(|i| item(i as u8, 5_000_000, true)).collect();
        let plan = plan(items, 1, 200);
        assert_eq!(plan.days(), Some(1));
    }

    #[test]
    fn the_whole_library_takes_days_not_minutes() {
        // O acervo de referência do RoadMap.
        let plan = RestorePlan {
            selection: RestoreSelection::Everything,
            sink: "google_photos".into(),
            capabilities: SinkCapabilities::GOOGLE_PHOTOS,
            items: (0..u8::MAX).map(|i| item(i, 8_000_000, true)).collect(),
            rejected: Vec::new(),
            albums: 184,
            album_memberships: 60_000,
        };
        // 60 mil associações sozinhas já passam de um dia de cota.
        assert!(plan.days().unwrap_or(0) >= 1);
        assert!(plan.requests() > 1_000);
    }

    #[test]
    fn google_plan_declares_the_two_losses() {
        let losses = plan(vec![item(1, 100, true)], 0, 0).losses();
        assert_eq!(losses.len(), 2);
    }

    #[test]
    fn located_items_need_normalization_for_google() {
        let plan = plan(
            vec![item(1, 100, true), item(2, 100, false), item(3, 100, true)],
            0,
            0,
        );
        // Sem EXIF embutido, estes dois perderiam a localização em silêncio.
        assert_eq!(plan.needing_normalization(), 2);
    }

    #[test]
    fn a_local_folder_needs_no_normalization() {
        let mut plan = plan(vec![item(1, 100, true)], 0, 0);
        plan.capabilities = SinkCapabilities::LOCAL_FOLDER;
        assert_eq!(plan.needing_normalization(), 0);
        assert!(plan.losses().is_empty());
    }

    #[test]
    fn storage_warning_states_the_cost() {
        let plan = plan(vec![item(1, 14 * 1024 * 1024 * 1024, true)], 0, 0);
        let warning = plan.storage_warning().expect("há bytes a enviar");
        assert!(warning.contains("14.0 GB"));
        assert!(warning.contains("cota de armazenamento"));
    }

    #[test]
    fn no_warning_when_nothing_travels() {
        assert!(plan(Vec::new(), 0, 0).storage_warning().is_none());
    }

    #[test]
    fn selection_descriptions_read_naturally() {
        assert_eq!(RestoreSelection::Everything.describe(), "todo o acervo");
        assert_eq!(
            RestoreSelection::Album("Viagem Japão".into()).describe(),
            "o álbum \"Viagem Japão\""
        );
        assert_eq!(
            RestoreSelection::Sample(20).describe(),
            "uma amostra de 20 itens"
        );
    }

    #[test]
    fn bytes_are_summed_across_items() {
        let plan = plan(vec![item(1, 1000, false), item(2, 2000, false)], 0, 0);
        assert_eq!(plan.bytes(), 3000);
    }
}
