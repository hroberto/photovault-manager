//! O que um destino consegue receber.
//!
//! Esta struct é a razão de a tela de restauração nunca mentir. Os avisos de "isto não será
//! restaurado" são derivados daqui, não escritos à mão — quando a Google mudar a API, a
//! interface muda junto. Ver ADR-010.

/// Capacidades declaradas por um destino de restauração.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkCapabilities {
    /// Aceita descrição por campo de API.
    pub description: bool,
    /// Permite criar álbuns e adicionar itens.
    pub albums: bool,
    /// Aceita marcação de pessoas.
    pub people: bool,
    /// Aceita marcação de favorito.
    pub favorites: bool,
    /// Aceita coordenada como campo explícito da API.
    pub explicit_geo: bool,
    /// Lê a geolocalização do EXIF embutido nos bytes enviados.
    pub reads_exif_geo: bool,
    /// Preserva a data de captura do EXIF embutido.
    pub reads_exif_date: bool,
    /// Tamanho máximo de uma foto, em bytes.
    pub max_photo_bytes: u64,
    /// Tamanho máximo de um vídeo, em bytes.
    pub max_video_bytes: u64,
    /// Requisições por dia, quando há cota.
    pub daily_request_quota: Option<u32>,
    /// Itens por chamada de criação em lote.
    pub batch_size: u16,
}

impl SinkCapabilities {
    /// Google Fotos via Library API com escopo `photoslibrary.appendonly`, em 2026.
    ///
    /// Os números vêm da documentação oficial e estão registrados em `RoadMap.md` seção 20.
    /// Não os altere sem verificar a fonte.
    pub const GOOGLE_PHOTOS: Self = Self {
        description: true,
        albums: true,
        // Não existe API para marcar pessoas nem favoritos.
        people: false,
        favorites: false,
        // Não há campo de localização no batchCreate — mas o Google lê o EXIF do arquivo.
        explicit_geo: false,
        reads_exif_geo: true,
        reads_exif_date: true,
        max_photo_bytes: 200 * 1024 * 1024,
        max_video_bytes: 20 * 1024 * 1024 * 1024,
        daily_request_quota: Some(10_000),
        batch_size: 50,
    };

    /// Pasta local: aceita tudo, porque os metadados vão dentro do arquivo.
    pub const LOCAL_FOLDER: Self = Self {
        description: true,
        albums: true,
        people: true,
        favorites: true,
        explicit_geo: true,
        reads_exif_geo: true,
        reads_exif_date: true,
        max_photo_bytes: u64::MAX,
        max_video_bytes: u64::MAX,
        daily_request_quota: None,
        batch_size: u16::MAX,
    };

    /// O que se perde ao enviar para este destino.
    ///
    /// A interface renderiza exatamente esta lista. Se ela estiver vazia, nada se perde.
    pub fn losses(&self) -> Vec<MetadataLoss> {
        let mut losses = Vec::new();
        if !self.people {
            losses.push(MetadataLoss::People);
        }
        if !self.favorites {
            losses.push(MetadataLoss::Favorites);
        }
        if !self.explicit_geo && !self.reads_exif_geo {
            losses.push(MetadataLoss::Location);
        }
        losses
    }

    /// Se a geolocalização chega ao destino de alguma forma.
    ///
    /// No Google a resposta é sim, mas só porque ele lê o EXIF — o que torna o passo de
    /// normalização obrigatório antes do envio.
    pub const fn preserves_location(&self) -> bool {
        self.explicit_geo || self.reads_exif_geo
    }

    /// Se a localização depende de o EXIF estar embutido no arquivo enviado.
    pub const fn requires_embedded_geo(&self) -> bool {
        !self.explicit_geo && self.reads_exif_geo
    }

    /// Quantos dias uma restauração deste tamanho leva, dada a cota.
    ///
    /// Devolve `None` quando não há cota. O resultado é arredondado para cima: seis dias e meio
    /// são sete dias de espera para quem está olhando a tela.
    pub fn estimated_days(&self, requests: u64) -> Option<u32> {
        let quota = u64::from(self.daily_request_quota?);
        if quota == 0 {
            return None;
        }
        Some(requests.div_ceil(quota).try_into().unwrap_or(u32::MAX))
    }

    /// Número de requisições de uma restauração, antes de conhecer os álbuns.
    ///
    /// Um upload por item, mais as criações em lote, mais a verificação de cada item.
    pub fn estimate_requests(&self, items: u64, album_memberships: u64, albums: u64) -> u64 {
        let batch = u64::from(self.batch_size).max(1);
        let uploads = items;
        let creates = items.div_ceil(batch);
        let album_adds = album_memberships.div_ceil(batch);
        let verifications = items.div_ceil(batch);
        uploads + creates + albums + album_adds + verifications
    }
}

/// Um metadado que não sobrevive ao envio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataLoss {
    /// Nomes de pessoas marcadas.
    People,
    /// Marcação de favorito.
    Favorites,
    /// Coordenada geográfica.
    Location,
}

impl MetadataLoss {
    /// Texto exibido ao usuário antes de confirmar a restauração.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::People => "nomes de pessoas marcadas",
            Self::Favorites => "marcações de favorito",
            Self::Location => "geolocalização",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_loses_people_and_favorites_only() {
        let losses = SinkCapabilities::GOOGLE_PHOTOS.losses();
        assert_eq!(losses, vec![MetadataLoss::People, MetadataLoss::Favorites]);
    }

    #[test]
    fn google_preserves_location_through_exif() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        assert!(google.preserves_location());
        assert!(!google.explicit_geo);
        // É isto que torna a normalização obrigatória antes de enviar.
        assert!(google.requires_embedded_geo());
        assert!(!google.losses().contains(&MetadataLoss::Location));
    }

    #[test]
    fn local_folder_loses_nothing() {
        assert!(SinkCapabilities::LOCAL_FOLDER.losses().is_empty());
        assert!(!SinkCapabilities::LOCAL_FOLDER.requires_embedded_geo());
    }

    #[test]
    fn full_library_restore_takes_about_six_days() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        // O acervo de referência do RoadMap: 48.231 fotos + 3.921 vídeos, 184 álbuns.
        let requests = google.estimate_requests(52_152, 60_000, 184);
        assert!(
            (54_000..57_000).contains(&requests),
            "estimativa fora do esperado: {requests}"
        );
        assert_eq!(google.estimated_days(requests), Some(6));
    }

    #[test]
    fn small_restore_finishes_same_day() {
        let google = SinkCapabilities::GOOGLE_PHOTOS;
        let requests = google.estimate_requests(1_284, 1_284, 1);
        assert_eq!(google.estimated_days(requests), Some(1));
    }

    #[test]
    fn local_folder_has_no_estimate() {
        assert_eq!(
            SinkCapabilities::LOCAL_FOLDER.estimated_days(1_000_000),
            None
        );
    }
}
