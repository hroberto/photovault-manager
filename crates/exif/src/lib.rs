//! Escrita e verificação de metadados embutidos nos arquivos.
//!
//! Este crate implementa o ADR-013: os metadados que o Google entrega no sidecar são gravados
//! *dentro* de uma cópia do arquivo. Duas razões, e a segunda é a que torna o passo obrigatório:
//!
//! 1. Um acervo cujo significado só existe num banco SQLite morre junto com o software.
//! 2. O Google Fotos **lê o EXIF dos bytes que recebe**. Sem geolocalização embutida no arquivo,
//!    a restauração perde a localização — e o usuário só descobre quando abre o mapa e ele está
//!    vazio.
//!
//! O objeto original no CAS nunca é tocado (ADR-004). A normalização sempre produz uma cópia.
//!
//! ## Honestidade sobre o que este backend faz
//!
//! O ecossistema Rust não cobre o que o ExifTool cobre, e fingir o contrário custaria metadados.
//! O backend nativo grava **EXIF**: data de captura, GPS e descrição, em JPEG, PNG, WebP, JXL,
//! TIFF e HEIF. Não grava **XMP** — portanto nomes de pessoas e a nota de favorito ficam de fora
//! e são reportados como pulados, com o motivo. Quando o ExifTool está instalado, ele cobre essa
//! lacuna. Nunca é dependência obrigatória.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use photovault_core::{GeoPoint, PersonName};
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

pub mod coords;

/// Formato de data do EXIF: `AAAA:MM:DD HH:MM:SS`, sempre com dois dígitos.
const EXIF_DATE: &[BorrowedFormatItem<'_>] =
    format_description!("[year]:[month]:[day] [hour]:[minute]:[second]");

/// O que se quer embutir no arquivo.
#[derive(Debug, Clone, Default)]
pub struct EmbeddedMetadata {
    /// Quando a foto foi tirada.
    pub captured_at: Option<OffsetDateTime>,
    /// Onde foi tirada.
    pub location: Option<GeoPoint>,
    /// Descrição escrita pelo usuário.
    pub description: Option<String>,
    /// Pessoas marcadas. Exige ExifTool — o backend nativo não escreve XMP.
    pub people: Vec<PersonName>,
    /// Marcada como favorita. Exige ExifTool.
    pub favorited: bool,
}

impl EmbeddedMetadata {
    /// Se não há nada para gravar.
    pub fn is_empty(&self) -> bool {
        self.captured_at.is_none()
            && self.location.is_none()
            && self.description.is_none()
            && self.people.is_empty()
            && !self.favorited
    }
}

/// Um campo que se tenta gravar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// `EXIF:DateTimeOriginal` e `EXIF:CreateDate`.
    CaptureTime,
    /// `EXIF:GPSLatitude`, `GPSLongitude`, `GPSAltitude` e os respectivos `Ref`.
    Location,
    /// `EXIF:ImageDescription`.
    Description,
    /// `XMP-mwg-rs:RegionName` e `XMP:Subject`.
    People,
    /// `XMP:Rating`.
    Rating,
}

impl Field {
    /// Nome legível, para relatório.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::CaptureTime => "data de captura",
            Self::Location => "geolocalização",
            Self::Description => "descrição",
            Self::People => "nomes de pessoas",
            Self::Rating => "marcação de favorito",
        }
    }
}

/// Por que um campo não foi gravado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// O backend em uso não sabe gravar este campo.
    UnsupportedByBackend {
        /// Como resolver.
        remedy: &'static str,
    },
    /// O formato do arquivo não aceita este metadado.
    UnsupportedFormat(String),
    /// Não havia valor para gravar.
    NoValue,
}

impl SkipReason {
    /// Explicação para o relatório.
    pub fn describe(&self) -> String {
        match self {
            Self::UnsupportedByBackend { remedy } => remedy.to_string(),
            Self::UnsupportedFormat(extension) => {
                format!("o formato .{extension} não aceita este metadado")
            }
            Self::NoValue => "não havia valor a gravar".into(),
        }
    }
}

/// Resultado de uma normalização.
#[derive(Debug, Clone, Default)]
pub struct NormalizationOutcome {
    /// Caminho do arquivo normalizado.
    pub path: PathBuf,
    /// Campos efetivamente gravados e confirmados por releitura.
    pub written: Vec<Field>,
    /// Campos que não entraram, com o motivo.
    pub skipped: Vec<(Field, SkipReason)>,
}

impl NormalizationOutcome {
    /// Se a geolocalização chegou ao arquivo.
    ///
    /// É a pergunta que a restauração precisa fazer antes de enviar ao Google: sem isso, a
    /// localização se perde silenciosamente.
    pub fn has_location(&self) -> bool {
        self.written.contains(&Field::Location)
    }

    /// Campos pulados que o usuário precisa saber que ficaram de fora.
    pub fn losses(&self) -> Vec<(Field, String)> {
        self.skipped
            .iter()
            .filter(|(_, reason)| !matches!(reason, SkipReason::NoValue))
            .map(|(field, reason)| (*field, reason.describe()))
            .collect()
    }
}

/// Falha ao normalizar.
#[derive(Debug, thiserror::Error)]
pub enum ExifError {
    /// Erro de entrada e saída.
    #[error("{operation} em {path}: {source}")]
    Io {
        /// O que estava sendo feito.
        operation: &'static str,
        /// Caminho envolvido.
        path: PathBuf,
        /// Causa.
        source: std::io::Error,
    },
    /// A biblioteca de metadados recusou o arquivo.
    #[error("gravar metadados em {path}: {message}")]
    Write {
        /// Arquivo.
        path: PathBuf,
        /// Mensagem da biblioteca.
        message: String,
    },
    /// O arquivo foi gravado mas a releitura não confirmou o valor.
    ///
    /// Bibliotecas de EXIF falham em silêncio com mais frequência do que se admite. É por isso
    /// que toda escrita é conferida.
    #[error("{field:?} não sobreviveu à releitura de {path}: {detail}")]
    VerificationFailed {
        /// Campo que não conferiu.
        field: Field,
        /// Arquivo.
        path: PathBuf,
        /// O que foi encontrado.
        detail: String,
    },
    /// Formato para o qual não há escrita nativa.
    #[error("não há escrita nativa de metadados para .{0}")]
    UnsupportedFormat(String),
}

type Result<T> = std::result::Result<T, ExifError>;

fn io_err(
    operation: &'static str,
    path: impl Into<PathBuf>,
) -> impl FnOnce(std::io::Error) -> ExifError {
    let path = path.into();
    move |source| ExifError::Io {
        operation,
        path,
        source,
    }
}

/// Formatos em que o backend nativo sabe gravar EXIF.
const NATIVE_FORMATS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "jxl", "tif", "tiff", "heic", "heif",
];

/// Se o backend nativo sabe gravar neste formato.
pub fn supports_natively(filename: &str) -> bool {
    extension_of(filename).is_some_and(|ext| NATIVE_FORMATS.contains(&ext.as_str()))
}

fn extension_of(filename: &str) -> Option<String> {
    Some(filename.rsplit_once('.')?.1.to_ascii_lowercase())
}

/// Grava metadados numa cópia do arquivo original.
///
/// `source` é o objeto no CAS, que **não é modificado**. `destination` recebe a cópia
/// enriquecida. Toda escrita é conferida por releitura antes de a função retornar.
pub fn normalize(
    source: &Path,
    destination: &Path,
    metadata: &EmbeddedMetadata,
) -> Result<NormalizationOutcome> {
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = extension_of(filename).unwrap_or_default();

    if !supports_natively(filename) {
        return Err(ExifError::UnsupportedFormat(extension));
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("criar diretório de destino", parent))?;
    }
    std::fs::copy(source, destination).map_err(io_err("copiar objeto original", source))?;

    // `fs::copy` preserva as permissões da origem, e os objetos do CAS são gravados como
    // somente-leitura para impor o ADR-004. Sem devolver a permissão de escrita aqui, a cópia
    // derivada nasce imutável e a gravação dos metadados falha com "Permission denied".
    make_writable(destination)?;

    let mut outcome = NormalizationOutcome {
        path: destination.to_owned(),
        ..Default::default()
    };

    let mut exif = Metadata::new();
    let mut pending = Vec::new();

    if let Some(captured_at) = metadata.captured_at {
        let formatted = captured_at
            .format(&EXIF_DATE)
            .unwrap_or_else(|_| "1970:01:01 00:00:00".to_owned());
        exif.set_tag(ExifTag::DateTimeOriginal(formatted.clone()));
        exif.set_tag(ExifTag::CreateDate(formatted));
        pending.push(Field::CaptureTime);
    } else {
        outcome
            .skipped
            .push((Field::CaptureTime, SkipReason::NoValue));
    }

    if let Some(point) = metadata.location.filter(|p| !p.is_null_island()) {
        let latitude = coords::latitude_to_exif(point.latitude());
        let longitude = coords::longitude_to_exif(point.longitude());

        exif.set_tag(ExifTag::GPSLatitudeRef(latitude.reference.to_owned()));
        exif.set_tag(ExifTag::GPSLatitude(latitude.dms.to_vec()));
        exif.set_tag(ExifTag::GPSLongitudeRef(longitude.reference.to_owned()));
        exif.set_tag(ExifTag::GPSLongitude(longitude.dms.to_vec()));

        if let Some(meters) = point.altitude() {
            let (value, reference) = coords::altitude_to_exif(meters);
            exif.set_tag(ExifTag::GPSAltitudeRef(vec![reference]));
            exif.set_tag(ExifTag::GPSAltitude(vec![value]));
        }
        pending.push(Field::Location);
    } else {
        outcome.skipped.push((Field::Location, SkipReason::NoValue));
    }

    if let Some(description) = metadata
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        exif.set_tag(ExifTag::ImageDescription(description.to_owned()));
        pending.push(Field::Description);
    } else {
        outcome
            .skipped
            .push((Field::Description, SkipReason::NoValue));
    }

    // XMP não é escrevível pelo backend nativo. Dizer isso é melhor do que perder o dado sem
    // avisar — ver a nota de honestidade no topo do módulo.
    const NEEDS_EXIFTOOL: SkipReason = SkipReason::UnsupportedByBackend {
        remedy: "requer XMP; instale o ExifTool para gravar este campo",
    };
    if metadata.people.is_empty() {
        outcome.skipped.push((Field::People, SkipReason::NoValue));
    } else {
        outcome.skipped.push((Field::People, NEEDS_EXIFTOOL));
    }
    if metadata.favorited {
        outcome.skipped.push((Field::Rating, NEEDS_EXIFTOOL));
    } else {
        outcome.skipped.push((Field::Rating, SkipReason::NoValue));
    }

    if !pending.is_empty() {
        exif.write_to_file(destination)
            .map_err(|error| ExifError::Write {
                path: destination.to_owned(),
                message: error.to_string(),
            })?;

        verify(destination, metadata, &pending)?;
        outcome.written = pending;
    }

    outcome.skipped.sort_by_key(|(field, _)| *field);
    Ok(outcome)
}

/// Devolve a permissão de escrita a um arquivo derivado.
#[cfg(unix)]
fn make_writable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
        .map_err(io_err("liberar escrita na cópia", path))
}

#[cfg(not(unix))]
fn make_writable(path: &Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)
        .map_err(io_err("ler permissões da cópia", path))?
        .permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).map_err(io_err("liberar escrita na cópia", path))
}

/// Relê o arquivo e confirma que os campos entraram com os valores certos.
fn verify(path: &Path, metadata: &EmbeddedMetadata, fields: &[Field]) -> Result<()> {
    let file = std::fs::File::open(path).map_err(io_err("reabrir para verificar", path))?;
    let mut reader = std::io::BufReader::new(&file);
    let read = exif::Reader::new()
        .read_from_container(&mut reader)
        .map_err(|error| ExifError::VerificationFailed {
            field: fields[0],
            path: path.to_owned(),
            detail: format!("não foi possível reler o EXIF: {error}"),
        })?;

    for &field in fields {
        match field {
            Field::CaptureTime => {
                let Some(expected) = metadata.captured_at else {
                    continue;
                };
                let found = read
                    .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
                    .map(|f| f.display_value().to_string());
                // O EXIF grava "AAAA:MM:DD HH:MM:SS"; a leitura normaliza para
                // "AAAA-MM-DD HH:MM:SS". Só os dois primeiros separadores mudam — os da hora
                // continuam sendo dois-pontos. A comparação é do carimbo inteiro: conferir só a
                // data deixaria passar uma biblioteca que gravasse a hora errada.
                let expected_text = expected
                    .format(&EXIF_DATE)
                    .unwrap_or_default()
                    .replacen(':', "-", 2);
                let matches = found
                    .as_deref()
                    .is_some_and(|text| text.trim() == expected_text);
                if !matches {
                    return Err(ExifError::VerificationFailed {
                        field,
                        path: path.to_owned(),
                        detail: found.unwrap_or_else(|| "campo ausente".into()),
                    });
                }
            }
            Field::Location => {
                let Some(point) = metadata.location else {
                    continue;
                };
                let recovered =
                    read_coordinate(&read, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef).zip(
                        read_coordinate(&read, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef),
                    );

                let Some((latitude, longitude)) = recovered else {
                    return Err(ExifError::VerificationFailed {
                        field,
                        path: path.to_owned(),
                        detail: "GPS ausente após a escrita".into(),
                    });
                };

                // Tolerância de 1e-6 grau, cerca de 11 cm.
                if (latitude - point.latitude()).abs() > 1e-6
                    || (longitude - point.longitude()).abs() > 1e-6
                {
                    return Err(ExifError::VerificationFailed {
                        field,
                        path: path.to_owned(),
                        detail: format!(
                            "esperava {},{} e leu {latitude},{longitude}",
                            point.latitude(),
                            point.longitude()
                        ),
                    });
                }
            }
            Field::Description => {
                let Some(expected) = metadata.description.as_deref() else {
                    continue;
                };
                let found = read
                    .get_field(exif::Tag::ImageDescription, exif::In::PRIMARY)
                    .map(|f| f.display_value().to_string());
                if !found.as_deref().is_some_and(|text| text.contains(expected)) {
                    return Err(ExifError::VerificationFailed {
                        field,
                        path: path.to_owned(),
                        detail: found.unwrap_or_else(|| "campo ausente".into()),
                    });
                }
            }
            Field::People | Field::Rating => {}
        }
    }

    Ok(())
}

/// Lê uma coordenada do EXIF já com o hemisfério aplicado.
fn read_coordinate(read: &exif::Exif, value: exif::Tag, reference: exif::Tag) -> Option<f64> {
    let field = read.get_field(value, exif::In::PRIMARY)?;
    let exif::Value::Rational(parts) = &field.value else {
        return None;
    };
    if parts.len() < 3 {
        return None;
    }
    let hemisphere = read
        .get_field(reference, exif::In::PRIMARY)
        .map(|f| f.display_value().to_string())
        .unwrap_or_default();

    Some(coords::from_exif(
        [parts[0].to_f64(), parts[1].to_f64(), parts[2].to_f64()],
        &hemisphere,
    ))
}

/// Se o ExifTool está disponível para cobrir os campos XMP.
pub fn exiftool_available() -> bool {
    std::process::Command::new("exiftool")
        .arg("-ver")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Um JPEG 1×1 válido e mínimo.
    const TINY_JPEG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x01, 0x00,
        0x60, 0x00, 0x60, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B,
        0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
        0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF,
        0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00,
        0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x09, 0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xDA, 0x00, 0x08, 0x01,
        0x01, 0x00, 0x00, 0x3F, 0x00, 0x2A, 0x9F, 0xFF, 0xD9,
    ];

    struct Fixture {
        _dir: tempfile::TempDir,
        original: PathBuf,
        destination: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let original = dir.path().join("objeto");
        std::fs::write(&original, TINY_JPEG).expect("escreve original");
        let destination = dir.path().join("normalized/IMG_1002.jpg");
        Fixture {
            _dir: dir,
            original,
            destination,
        }
    }

    fn kyoto() -> GeoPoint {
        GeoPoint::new(35.0116, 135.7681, Some(52.0)).expect("coordenada válida")
    }

    fn porto_alegre() -> GeoPoint {
        GeoPoint::new(-30.0346, -51.2177, None).expect("coordenada válida")
    }

    #[test]
    fn writes_and_verifies_capture_time() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.written.contains(&Field::CaptureTime));
    }

    #[test]
    fn the_hour_is_written_not_just_the_date() {
        // Conferir só a data deixaria passar uma biblioteca que gravasse a hora errada.
        let f = fixture();
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 10:20:00 UTC)),
            ..Default::default()
        };
        normalize(&f.original, &f.destination, &metadata).expect("normaliza");

        let file = std::fs::File::open(&f.destination).expect("abre");
        let mut reader = std::io::BufReader::new(&file);
        let read = exif::Reader::new()
            .read_from_container(&mut reader)
            .expect("lê EXIF");
        let found = read
            .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
            .expect("tem data")
            .display_value()
            .to_string();
        assert_eq!(found, "2019-10-18 10:20:00");
    }

    #[test]
    fn writes_and_verifies_location() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            location: Some(kyoto()),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.has_location());
    }

    #[test]
    fn southern_and_western_hemispheres_survive() {
        // O teste que importa: sem GPSLatitudeRef, esta foto apareceria na Ucrânia. A
        // verificação por releitura de `normalize` compara o valor com sinal, então este teste
        // falharia se o hemisfério não fosse gravado.
        let f = fixture();
        let metadata = EmbeddedMetadata {
            location: Some(porto_alegre()),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.has_location());

        let file = std::fs::File::open(&f.destination).expect("abre");
        let mut reader = std::io::BufReader::new(&file);
        let read = exif::Reader::new()
            .read_from_container(&mut reader)
            .expect("lê EXIF");
        let latitude =
            read_coordinate(&read, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef).expect("lat");
        assert!(
            latitude < 0.0,
            "a latitude precisa continuar negativa: {latitude}"
        );
    }

    #[test]
    fn writes_description() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            description: Some("Templo em Kyoto".into()),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.written.contains(&Field::Description));
    }

    #[test]
    fn writes_everything_at_once() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            location: Some(kyoto()),
            description: Some("Templo em Kyoto".into()),
            people: vec![PersonName::new("Henrique").expect("nome")],
            favorited: true,
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.written.contains(&Field::CaptureTime));
        assert!(outcome.written.contains(&Field::Location));
        assert!(outcome.written.contains(&Field::Description));
    }

    #[test]
    fn xmp_fields_are_reported_as_losses_not_dropped_silently() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            people: vec![PersonName::new("Ana").expect("nome")],
            favorited: true,
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");

        let losses = outcome.losses();
        assert_eq!(
            losses.len(),
            2,
            "pessoas e favorito precisam ser reportados"
        );
        assert!(losses.iter().any(|(field, _)| *field == Field::People));
        assert!(losses.iter().any(|(field, _)| *field == Field::Rating));
        assert!(
            losses[0].1.contains("ExifTool"),
            "o relatório precisa dizer como resolver"
        );
    }

    #[test]
    fn absent_values_are_not_reported_as_losses() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        // Não ter pessoas não é uma perda; é ausência.
        assert!(outcome.losses().is_empty());
    }

    #[test]
    fn the_original_object_is_never_touched() {
        let f = fixture();
        let before = std::fs::read(&f.original).expect("lê antes");
        let metadata = EmbeddedMetadata {
            location: Some(kyoto()),
            description: Some("altera bastante".into()),
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            ..Default::default()
        };
        normalize(&f.original, &f.destination, &metadata).expect("normaliza");

        // ADR-004: escrever no objeto mudaria seu hash e destruiria a identidade do CAS.
        let after = std::fs::read(&f.original).expect("lê depois");
        assert_eq!(before, after, "o objeto original não pode mudar");
        assert_ne!(
            std::fs::read(&f.destination).expect("lê cópia"),
            before,
            "a cópia precisa ter mudado"
        );
    }

    #[test]
    fn works_when_the_source_is_read_only() {
        // Os objetos do CAS são gravados em modo 0444 para impor o ADR-004. Como `fs::copy`
        // preserva as permissões da origem, sem tratamento a cópia derivada nasce imutável e a
        // gravação falha com "Permission denied".
        let f = fixture();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f.original, std::fs::Permissions::from_mode(0o444))
                .expect("torna a origem somente leitura");
        }

        let metadata = EmbeddedMetadata {
            location: Some(kyoto()),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.has_location());
    }

    #[test]
    fn null_island_is_not_written_as_a_location() {
        let f = fixture();
        let metadata = EmbeddedMetadata {
            location: GeoPoint::new(0.0, 0.0, None).ok(),
            ..Default::default()
        };
        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(!outcome.has_location(), "0,0 é ausência, não coordenada");
    }

    #[test]
    fn creates_the_destination_directory() {
        let f = fixture();
        let deep = f.destination.parent().expect("tem pai").join("a/b/c.jpg");
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            ..Default::default()
        };
        assert!(normalize(&f.original, &deep, &metadata).is_ok());
        assert!(deep.is_file());
    }

    #[test]
    fn unsupported_format_is_refused_explicitly() {
        let f = fixture();
        let video = f.destination.with_extension("mp4");
        let metadata = EmbeddedMetadata {
            location: Some(kyoto()),
            ..Default::default()
        };
        match normalize(&f.original, &video, &metadata) {
            Err(ExifError::UnsupportedFormat(extension)) => assert_eq!(extension, "mp4"),
            other => panic!("esperava recusa explícita, veio {other:?}"),
        }
    }

    #[test]
    fn format_support_is_declared() {
        assert!(supports_natively("IMG.JPG"));
        assert!(supports_natively("IMG.heic"));
        assert!(supports_natively("IMG.png"));
        assert!(!supports_natively("VID.mp4"));
        assert!(!supports_natively("RAW.cr2"));
        assert!(!supports_natively("sem_extensao"));
    }

    #[test]
    fn normalization_is_reproducible() {
        // ADR-013: apagar `derived/` e regerar precisa dar o mesmo resultado, ou o cofre deixa
        // de ser reprodutível.
        let f = fixture();
        let metadata = EmbeddedMetadata {
            captured_at: Some(datetime!(2019-10-18 09:00:00 UTC)),
            location: Some(kyoto()),
            description: Some("Templo em Kyoto".into()),
            ..Default::default()
        };

        normalize(&f.original, &f.destination, &metadata).expect("primeira vez");
        let first = std::fs::read(&f.destination).expect("lê");

        std::fs::remove_file(&f.destination).expect("apaga");
        normalize(&f.original, &f.destination, &metadata).expect("segunda vez");
        let second = std::fs::read(&f.destination).expect("lê");

        assert_eq!(first, second, "a normalização precisa ser determinística");
    }

    #[test]
    fn empty_metadata_writes_nothing_but_still_copies() {
        let f = fixture();
        let metadata = EmbeddedMetadata::default();
        assert!(metadata.is_empty());

        let outcome = normalize(&f.original, &f.destination, &metadata).expect("normaliza");
        assert!(outcome.written.is_empty());
        assert_eq!(
            std::fs::read(&f.destination).expect("lê"),
            TINY_JPEG,
            "sem metadado, a cópia é idêntica ao original"
        );
    }
}
