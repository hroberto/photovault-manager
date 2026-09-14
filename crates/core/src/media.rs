//! O item de mídia.
//!
//! Distinção que atravessa o projeto: um **objeto** são bytes identificados por hash; um
//! **item** é a coisa lógica que o usuário chama de foto. Um objeto sustenta vários itens; um
//! item pertence a vários álbuns. Colapsar as duas entidades por conveniência é o erro de
//! arquitetura que o `RoadMap.md` seção 10 evita.

use time::OffsetDateTime;

use crate::{Fidelity, GeoPoint, MediaId, ObjectHash, PersonName};

/// Natureza do item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// Imagem parada.
    Photo,
    /// Vídeo.
    Video,
    /// Foto com componente de movimento — Live Photo, Motion Photo.
    ///
    /// No Takeout chega partida em dois arquivos (`.HEIC` + `.MP4`) que são **um** item. Nunca
    /// os trate como duplicatas um do outro.
    MotionPhoto,
}

impl MediaKind {
    /// Rótulo estável para persistência.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::MotionPhoto => "motion_photo",
        }
    }

    /// Lê o rótulo persistido.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "photo" => Self::Photo,
            "video" => Self::Video,
            "motion_photo" => Self::MotionPhoto,
            _ => return None,
        })
    }
}

/// De onde o item entrou no cofre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Archive do Google Takeout. Fonte canônica.
    Takeout,
    /// Google Photos Picker API. Fonte degradada: os bytes vêm sem GPS.
    Picker,
    /// Pasta local ou disco do usuário.
    LocalFolder,
}

impl SourceKind {
    /// Fidelidade que esta fonte produz.
    ///
    /// É aqui que a assimetria entre Takeout e API vira regra executável em vez de nota de
    /// rodapé no documento.
    pub const fn fidelity(self) -> Fidelity {
        match self {
            Self::Takeout | Self::LocalFolder => Fidelity::Original,
            Self::Picker => Fidelity::ApiDerived,
        }
    }

    /// Rótulo estável para persistência.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Takeout => "takeout",
            Self::Picker => "picker",
            Self::LocalFolder => "local",
        }
    }

    /// Lê o rótulo persistido.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "takeout" => Self::Takeout,
            "picker" => Self::Picker,
            "local" => Self::LocalFolder,
            _ => return None,
        })
    }
}

/// Como um item se relaciona com outro.
///
/// Existe para que a deduplicação não elimine o que não é duplicata. `IMG_1002.JPG` e
/// `IMG_1002-edited.JPG` têm hash perceptual quase idêntico e são coisas diferentes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Versão editada de outro item.
    EditedFrom(MediaId),
    /// Componente de vídeo de uma foto com movimento.
    MotionPartOf(MediaId),
}

/// Um item de mídia no catálogo.
#[derive(Debug, Clone)]
pub struct MediaItem {
    /// Identidade no catálogo, ausente antes de persistir.
    pub id: Option<MediaId>,
    /// Hash dos bytes que sustentam este item.
    pub object: ObjectHash,
    /// Nome do arquivo como veio da origem.
    pub filename: String,
    /// Natureza do item.
    pub kind: MediaKind,
    /// Fonte de entrada.
    pub source: SourceKind,
    /// Quando a foto foi tirada, se conhecido.
    pub captured_at: Option<OffsetDateTime>,
    /// Quando entrou no Google, se conhecido.
    pub uploaded_at: Option<OffsetDateTime>,
    /// Descrição escrita pelo usuário.
    pub description: Option<String>,
    /// Onde foi tirada.
    pub location: Option<GeoPoint>,
    /// Pessoas marcadas.
    pub people: Vec<PersonName>,
    /// Marcada como favorita no Google.
    pub favorited: bool,
    /// Vínculo com outro item, quando este é derivado ou componente.
    pub relation: Option<Relation>,
}

impl MediaItem {
    /// Fidelidade implicada pela fonte.
    pub const fn fidelity(&self) -> Fidelity {
        self.source.fidelity()
    }

    /// Se o item pode participar de um grupo de duplicatas.
    ///
    /// Versões editadas e componentes de movimento ficam de fora: são parentes, não cópias.
    /// Sem esta regra, a deduplicação apaga a foto editada por ser "igual" à original.
    pub const fn is_dedupe_candidate(&self) -> bool {
        self.relation.is_none()
    }

    /// Se há geolocalização utilizável.
    pub fn has_location(&self) -> bool {
        self.location.is_some_and(|point| !point.is_null_island())
    }

    /// Se o item está pronto para ser enviado a um destino que exige EXIF embutido.
    ///
    /// Um item com coordenada no catálogo mas não no arquivo perde a localização no caminho —
    /// e o usuário só descobre quando abre o mapa no Google e ele está vazio.
    pub fn needs_normalization_for(&self, sink: &crate::SinkCapabilities) -> bool {
        sink.requires_embedded_geo() && self.has_location()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SinkCapabilities;

    fn item() -> MediaItem {
        MediaItem {
            id: None,
            object: ObjectHash::from_bytes([1; 32]),
            filename: "IMG_1002.JPG".into(),
            kind: MediaKind::Photo,
            source: SourceKind::Takeout,
            captured_at: None,
            uploaded_at: None,
            description: None,
            location: GeoPoint::new(35.0116, 135.7681, None).ok(),
            people: Vec::new(),
            favorited: false,
            relation: None,
        }
    }

    #[test]
    fn takeout_is_original_picker_is_derived() {
        assert_eq!(SourceKind::Takeout.fidelity(), Fidelity::Original);
        assert_eq!(SourceKind::LocalFolder.fidelity(), Fidelity::Original);
        assert_eq!(SourceKind::Picker.fidelity(), Fidelity::ApiDerived);
    }

    #[test]
    fn edited_versions_are_not_dedupe_candidates() {
        let mut edited = item();
        edited.relation = Some(Relation::EditedFrom(MediaId::new(1)));
        assert!(!edited.is_dedupe_candidate());
        assert!(item().is_dedupe_candidate());
    }

    #[test]
    fn motion_component_is_not_a_duplicate() {
        let mut component = item();
        component.kind = MediaKind::Video;
        component.relation = Some(Relation::MotionPartOf(MediaId::new(7)));
        assert!(!component.is_dedupe_candidate());
    }

    #[test]
    fn null_island_does_not_count_as_location() {
        let mut without = item();
        without.location = GeoPoint::new(0.0, 0.0, None).ok();
        assert!(!without.has_location());
        assert!(item().has_location());
    }

    #[test]
    fn located_item_must_be_normalized_before_google() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        assert!(item().needs_normalization_for(&google));

        let mut without = item();
        without.location = None;
        assert!(!without.needs_normalization_for(&google));
    }

    #[test]
    fn local_folder_never_requires_normalization() {
        assert!(!item().needs_normalization_for(&SinkCapabilities::LOCAL_FOLDER));
    }

    #[test]
    fn labels_roundtrip() {
        for kind in [MediaKind::Photo, MediaKind::Video, MediaKind::MotionPhoto] {
            assert_eq!(MediaKind::parse(kind.as_str()), Some(kind));
        }
        for source in [
            SourceKind::Takeout,
            SourceKind::Picker,
            SourceKind::LocalFolder,
        ] {
            assert_eq!(SourceKind::parse(source.as_str()), Some(source));
        }
    }
}
