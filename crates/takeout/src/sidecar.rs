//! O JSON lateral de cada item.
//!
//! É daqui que saem geolocalização, pessoas, descrição e favoritos — nada disso vem pela API do
//! Google Fotos. Este arquivo é a razão de o Takeout ser a fonte canônica do projeto.
//!
//! O formato não é documentado e campos aparecem e somem entre versões. Por isso toda
//! desserialização é tolerante: campo desconhecido é ignorado, campo ausente vira `None`, e um
//! valor malformado invalida aquele campo, nunca o arquivo inteiro.

use photovault_core::{place, GeoPoint, PersonName};
use serde::Deserialize;
use time::OffsetDateTime;

/// Conteúdo de um sidecar, já validado e convertido para o domínio.
#[derive(Debug, Clone, PartialEq)]
pub struct Sidecar {
    /// Nome original do arquivo, como o Google o conhece.
    pub title: String,
    /// Descrição escrita pelo usuário.
    pub description: Option<String>,
    /// Quando a foto foi tirada.
    pub taken_at: Option<OffsetDateTime>,
    /// Quando entrou no Google Fotos.
    pub uploaded_at: Option<OffsetDateTime>,
    /// Coordenada efetiva e de onde ela veio.
    pub location: Option<(GeoPoint, place::GeoSource)>,
    /// Coordenada gravada pela câmera, guardada quando diverge da efetiva.
    pub camera_location: Option<GeoPoint>,
    /// Pessoas marcadas. O Google dá os nomes, nunca as coordenadas do rosto.
    pub people: Vec<PersonName>,
    /// Marcada como favorita.
    pub favorited: bool,
    /// Link para o item no Google Fotos.
    ///
    /// Usado pelo Advisor para levar o usuário até a foto exata quando ele for apagar à mão.
    pub google_url: Option<String>,
    /// Campos que não soubemos interpretar, para diagnóstico.
    pub warnings: Vec<String>,
}

impl Sidecar {
    /// Lê um sidecar a partir do conteúdo do arquivo.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SidecarError> {
        let raw: RawSidecar = serde_json::from_slice(bytes)?;
        Ok(raw.into_domain())
    }

    /// Se a divergência entre `geoData` e `geoDataExif` merece registro.
    pub fn has_geo_divergence(&self) -> bool {
        match (self.location.map(|(point, _)| point), self.camera_location) {
            (Some(effective), Some(camera)) => effective != camera,
            _ => false,
        }
    }
}

/// Falha ao ler um sidecar.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// JSON inválido.
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSidecar {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    photo_taken_time: Option<RawTimestamp>,
    #[serde(default)]
    creation_time: Option<RawTimestamp>,
    #[serde(default)]
    geo_data: Option<RawGeo>,
    #[serde(default)]
    geo_data_exif: Option<RawGeo>,
    #[serde(default)]
    people: Vec<RawPerson>,
    #[serde(default)]
    favorited: bool,
    #[serde(default)]
    url: Option<String>,
}

impl RawSidecar {
    fn into_domain(self) -> Sidecar {
        let mut warnings = Vec::new();

        let taken_at = self
            .photo_taken_time
            .and_then(|ts| ts.parse(&mut warnings, "photoTakenTime"));
        let uploaded_at = self
            .creation_time
            .and_then(|ts| ts.parse(&mut warnings, "creationTime"));

        let google_edited = self
            .geo_data
            .and_then(|geo| geo.parse(&mut warnings, "geoData"));
        let camera_exif = self
            .geo_data_exif
            .and_then(|geo| geo.parse(&mut warnings, "geoDataExif"));

        let location = place::resolve_geo(google_edited, camera_exif);

        let people = self
            .people
            .into_iter()
            .filter_map(|person| match PersonName::new(&person.name) {
                Some(name) => Some(name),
                None => {
                    warnings.push("marcação de pessoa com nome vazio".to_owned());
                    None
                }
            })
            .collect();

        Sidecar {
            title: self.title,
            description: self.description.filter(|text| !text.trim().is_empty()),
            taken_at,
            uploaded_at,
            location,
            camera_location: camera_exif.filter(|point| !point.is_null_island()),
            people,
            favorited: self.favorited,
            google_url: self.url,
            warnings,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawTimestamp {
    /// Epoch em segundos, sempre como texto no formato do Google.
    #[serde(default)]
    timestamp: Option<String>,
}

impl RawTimestamp {
    fn parse(self, warnings: &mut Vec<String>, field: &str) -> Option<OffsetDateTime> {
        let raw = self.timestamp?;
        let seconds = match raw.parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                warnings.push(format!("{field}: timestamp não numérico ({raw})"));
                return None;
            }
        };
        match OffsetDateTime::from_unix_timestamp(seconds) {
            Ok(moment) => Some(moment),
            Err(_) => {
                warnings.push(format!("{field}: timestamp fora de faixa ({seconds})"));
                None
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawGeo {
    #[serde(default)]
    latitude: f64,
    #[serde(default)]
    longitude: f64,
    #[serde(default)]
    altitude: Option<f64>,
}

impl RawGeo {
    fn parse(self, warnings: &mut Vec<String>, field: &str) -> Option<GeoPoint> {
        // O Google usa 0,0 para "sem localização". Deixamos passar para que `resolve_geo`
        // aplique a regra de precedência em um lugar só.
        match GeoPoint::new(self.latitude, self.longitude, self.altitude) {
            Ok(point) => Some(point),
            Err(error) => {
                warnings.push(format!("{field}: {error}"));
                None
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawPerson {
    #[serde(default)]
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &[u8] = br#"{
      "title": "IMG_1002.JPG",
      "description": "Templo em Kyoto",
      "imageViews": "12",
      "creationTime": { "timestamp": "1571401200", "formatted": "18/10/2019 11:00:00 UTC" },
      "photoTakenTime": { "timestamp": "1571394000", "formatted": "18/10/2019 09:00:00 UTC" },
      "geoData": { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
      "geoDataExif": { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
      "people": [ { "name": "Henrique" }, { "name": "Ana" } ],
      "favorited": true,
      "url": "https://photos.google.com/photo/AF1Qip",
      "googlePhotosOrigin": { "mobileUpload": { "deviceType": "ANDROID_PHONE" } }
    }"#;

    #[test]
    fn reads_every_field_that_matters() {
        let sidecar = Sidecar::from_json(FULL).expect("sidecar válido");
        assert_eq!(sidecar.title, "IMG_1002.JPG");
        assert_eq!(sidecar.description.as_deref(), Some("Templo em Kyoto"));
        assert!(sidecar.favorited);
        assert_eq!(sidecar.people.len(), 2);
        assert_eq!(sidecar.people[0].display(), "Henrique");
        assert_eq!(
            sidecar.google_url.as_deref(),
            Some("https://photos.google.com/photo/AF1Qip")
        );
        assert!(sidecar.warnings.is_empty());
    }

    #[test]
    fn reads_capture_time_not_upload_time() {
        let sidecar = Sidecar::from_json(FULL).expect("sidecar válido");
        let taken = sidecar.taken_at.expect("tem data de captura");
        let uploaded = sidecar.uploaded_at.expect("tem data de upload");
        assert_eq!(taken.unix_timestamp(), 1_571_394_000);
        // As duas diferem, e confundi-las empilha o acervo na data de upload.
        assert!(taken < uploaded);
    }

    #[test]
    fn reads_location_with_altitude() {
        let sidecar = Sidecar::from_json(FULL).expect("sidecar válido");
        let (point, source) = sidecar.location.expect("tem localização");
        assert!((point.latitude() - 35.0116).abs() < 1e-9);
        assert_eq!(point.altitude(), Some(52.0));
        assert_eq!(source, place::GeoSource::GoogleEdited);
        assert!(!sidecar.has_geo_divergence());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // `imageViews` e `googlePhotosOrigin` existem no JSON e não quebram a leitura.
        assert!(Sidecar::from_json(FULL).is_ok());
    }

    #[test]
    fn missing_optional_fields_are_fine() {
        let minimal = br#"{ "title": "IMG_1.JPG" }"#;
        let sidecar = Sidecar::from_json(minimal).expect("sidecar mínimo válido");
        assert_eq!(sidecar.title, "IMG_1.JPG");
        assert!(sidecar.location.is_none());
        assert!(sidecar.taken_at.is_none());
        assert!(sidecar.people.is_empty());
        assert!(!sidecar.favorited);
    }

    #[test]
    fn zeroed_geodata_falls_back_to_camera() {
        let json = br#"{
          "title": "IMG_1.JPG",
          "geoData": { "latitude": 0.0, "longitude": 0.0, "altitude": 0.0 },
          "geoDataExif": { "latitude": -30.0346, "longitude": -51.2177, "altitude": 10.0 }
        }"#;
        let sidecar = Sidecar::from_json(json).expect("sidecar válido");
        let (point, source) = sidecar.location.expect("cai para a coordenada da câmera");
        assert_eq!(source, place::GeoSource::CameraExif);
        assert_eq!(point.latitude_ref(), "S");
        assert_eq!(point.longitude_ref(), "W");
    }

    #[test]
    fn both_zeroed_means_no_location() {
        let json = br#"{
          "title": "IMG_1.JPG",
          "geoData": { "latitude": 0.0, "longitude": 0.0 },
          "geoDataExif": { "latitude": 0.0, "longitude": 0.0 }
        }"#;
        let sidecar = Sidecar::from_json(json).expect("sidecar válido");
        assert!(sidecar.location.is_none());
        assert!(sidecar.camera_location.is_none());
    }

    #[test]
    fn divergent_coordinates_are_both_kept() {
        let json = br#"{
          "title": "IMG_1.JPG",
          "geoData": { "latitude": 35.0, "longitude": 135.0 },
          "geoDataExif": { "latitude": 36.0, "longitude": 136.0 }
        }"#;
        let sidecar = Sidecar::from_json(json).expect("sidecar válido");
        let (effective, source) = sidecar.location.expect("tem localização");
        assert_eq!(source, place::GeoSource::GoogleEdited);
        assert!((effective.latitude() - 35.0).abs() < 1e-9);
        // A coordenada da câmera é preservada para auditoria.
        assert!((sidecar.camera_location.expect("guardada").latitude() - 36.0).abs() < 1e-9);
        assert!(sidecar.has_geo_divergence());
    }

    #[test]
    fn bad_coordinate_warns_without_failing_the_file() {
        let json = br#"{
          "title": "IMG_1.JPG",
          "description": "sobrevive",
          "geoData": { "latitude": 999.0, "longitude": 0.0 }
        }"#;
        let sidecar = Sidecar::from_json(json).expect("o arquivo inteiro não se perde");
        assert!(sidecar.location.is_none());
        assert_eq!(sidecar.description.as_deref(), Some("sobrevive"));
        assert_eq!(sidecar.warnings.len(), 1);
        assert!(sidecar.warnings[0].contains("geoData"));
    }

    #[test]
    fn bad_timestamp_warns_without_failing_the_file() {
        let json = br#"{
          "title": "IMG_1.JPG",
          "photoTakenTime": { "timestamp": "ontem" }
        }"#;
        let sidecar = Sidecar::from_json(json).expect("o arquivo inteiro não se perde");
        assert!(sidecar.taken_at.is_none());
        assert_eq!(sidecar.warnings.len(), 1);
    }

    #[test]
    fn empty_description_is_absence() {
        let json = br#"{ "title": "IMG_1.JPG", "description": "   " }"#;
        let sidecar = Sidecar::from_json(json).expect("sidecar válido");
        assert!(sidecar.description.is_none());
    }

    #[test]
    fn empty_person_name_is_dropped_with_a_warning() {
        let json = br#"{ "title": "IMG_1.JPG", "people": [ { "name": "" }, { "name": "Ana" } ] }"#;
        let sidecar = Sidecar::from_json(json).expect("sidecar válido");
        assert_eq!(sidecar.people.len(), 1);
        assert_eq!(sidecar.warnings.len(), 1);
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(Sidecar::from_json(b"{ nao e json").is_err());
    }
}
