//! Localização geográfica.

use std::fmt;

/// Um ponto no mundo, com altitude opcional.
///
/// Construído sempre validado: latitude e longitude fora de faixa são recusadas na entrada, e
/// não descobertas depois quando a foto aparece no meio do oceano.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    latitude: f64,
    longitude: f64,
    altitude: Option<f64>,
}

impl GeoPoint {
    /// Constrói um ponto validando a faixa das coordenadas.
    pub fn new(latitude: f64, longitude: f64, altitude: Option<f64>) -> Result<Self, GeoError> {
        if !latitude.is_finite() || !longitude.is_finite() {
            return Err(GeoError::NotFinite);
        }
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(GeoError::Latitude(latitude));
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err(GeoError::Longitude(longitude));
        }
        Ok(Self {
            latitude,
            longitude,
            altitude,
        })
    }

    /// Latitude em graus decimais.
    pub const fn latitude(&self) -> f64 {
        self.latitude
    }

    /// Longitude em graus decimais.
    pub const fn longitude(&self) -> f64 {
        self.longitude
    }

    /// Altitude em metros, quando conhecida.
    pub const fn altitude(&self) -> Option<f64> {
        self.altitude
    }

    /// Se o ponto é a origem exata.
    ///
    /// O Takeout usa `0, 0` para "sem localização", e não para a ilha Null a 600 km da costa
    /// da Gana. Tratar zero como coordenada válida espalha fotos pelo Golfo da Guiné.
    pub fn is_null_island(&self) -> bool {
        self.latitude == 0.0 && self.longitude == 0.0
    }

    /// Hemisfério para `EXIF:GPSLatitudeRef`.
    ///
    /// Esquecer este campo é o erro clássico: sem ele a latitude negativa vira positiva e a
    /// foto do Brasil aparece na Ucrânia.
    pub const fn latitude_ref(&self) -> &'static str {
        if self.latitude < 0.0 {
            "S"
        } else {
            "N"
        }
    }

    /// Hemisfério para `EXIF:GPSLongitudeRef`.
    pub const fn longitude_ref(&self) -> &'static str {
        if self.longitude < 0.0 {
            "W"
        } else {
            "E"
        }
    }
}

impl fmt::Display for GeoPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.6},{:.6}", self.latitude, self.longitude)
    }
}

/// Coordenada recusada.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GeoError {
    /// Latitude fora de -90..=90.
    #[error("latitude fora de faixa: {0}")]
    Latitude(f64),
    /// Longitude fora de -180..=180.
    #[error("longitude fora de faixa: {0}")]
    Longitude(f64),
    /// NaN ou infinito.
    #[error("coordenada não finita")]
    NotFinite,
}

/// De onde veio a coordenada de um item.
///
/// O Takeout traz duas: a que o usuário vê no Google (`geoData`, que pode ter sido editada à
/// mão) e a que a câmera gravou (`geoDataExif`). Quando divergem, guardamos as duas e
/// registramos qual prevaleceu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoSource {
    /// Campo `geoData` do sidecar — o que o Google exibe.
    GoogleEdited,
    /// Campo `geoDataExif` do sidecar — o que a câmera gravou.
    CameraExif,
    /// Lido diretamente do EXIF do arquivo.
    FileExif,
    /// Informado pelo usuário no PhotoVault.
    User,
}

impl GeoSource {
    /// Rótulo estável para persistência.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GoogleEdited => "geo_data",
            Self::CameraExif => "geo_data_exif",
            Self::FileExif => "file_exif",
            Self::User => "user",
        }
    }
}

/// Resolve qual das duas coordenadas do sidecar deve valer.
///
/// Regra do `RoadMap.md` seção 11: `geoData` prevalece, exceto quando vem zerado e
/// `geoDataExif` não — aí vale o segundo.
pub fn resolve_geo(
    google_edited: Option<GeoPoint>,
    camera_exif: Option<GeoPoint>,
) -> Option<(GeoPoint, GeoSource)> {
    let usable = |point: Option<GeoPoint>| point.filter(|p| !p.is_null_island());

    match (usable(google_edited), usable(camera_exif)) {
        (Some(point), _) => Some((point, GeoSource::GoogleEdited)),
        (None, Some(point)) => Some((point, GeoSource::CameraExif)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(lat: f64, lon: f64) -> GeoPoint {
        GeoPoint::new(lat, lon, None).expect("coordenada de teste válida")
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(
            GeoPoint::new(91.0, 0.0, None),
            Err(GeoError::Latitude(91.0))
        );
        assert_eq!(
            GeoPoint::new(0.0, 181.0, None),
            Err(GeoError::Longitude(181.0))
        );
        assert_eq!(GeoPoint::new(f64::NAN, 0.0, None), Err(GeoError::NotFinite));
    }

    #[test]
    fn accepts_extremes() {
        assert!(GeoPoint::new(-90.0, -180.0, None).is_ok());
        assert!(GeoPoint::new(90.0, 180.0, None).is_ok());
    }

    #[test]
    fn hemisphere_refs_are_right() {
        // Kyoto: norte e leste.
        let kyoto = point(35.0116, 135.7681);
        assert_eq!(kyoto.latitude_ref(), "N");
        assert_eq!(kyoto.longitude_ref(), "E");

        // Porto Alegre: sul e oeste. Errar aqui manda a foto para a Ucrânia.
        let poa = point(-30.0346, -51.2177);
        assert_eq!(poa.latitude_ref(), "S");
        assert_eq!(poa.longitude_ref(), "W");
    }

    #[test]
    fn null_island_is_absence_not_location() {
        assert!(point(0.0, 0.0).is_null_island());
        assert!(!point(0.0001, 0.0).is_null_island());
    }

    #[test]
    fn google_edited_wins_when_present() {
        let edited = point(35.0, 135.0);
        let camera = point(36.0, 136.0);
        let (chosen, source) = resolve_geo(Some(edited), Some(camera)).expect("há coordenada");
        assert_eq!(chosen, edited);
        assert_eq!(source, GeoSource::GoogleEdited);
    }

    #[test]
    fn falls_back_to_camera_when_google_is_zeroed() {
        let camera = point(36.0, 136.0);
        let (chosen, source) =
            resolve_geo(Some(point(0.0, 0.0)), Some(camera)).expect("há coordenada");
        assert_eq!(chosen, camera);
        assert_eq!(source, GeoSource::CameraExif);
    }

    #[test]
    fn no_coordinate_when_both_are_zeroed() {
        assert!(resolve_geo(Some(point(0.0, 0.0)), Some(point(0.0, 0.0))).is_none());
        assert!(resolve_geo(None, None).is_none());
    }
}
