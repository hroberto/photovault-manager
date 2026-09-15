//! Conversão entre graus decimais e a representação do EXIF.
//!
//! O EXIF guarda coordenadas como três racionais sem sinal — graus, minutos e segundos — e o
//! sinal vive em um campo separado, `GPSLatitudeRef` / `GPSLongitudeRef`. **Esquecer o campo de
//! hemisfério é o erro clássico**: a latitude negativa vira positiva e a foto de Porto Alegre
//! aparece na Ucrânia. Por isso este módulo nunca devolve os graus sem o hemisfério junto.

use little_exif::rational::uR64;

/// Denominador dos segundos.
///
/// Dez mil dá resolução de 0,0001 segundo de arco — cerca de 3 mm. Muito além do que qualquer
/// GPS de câmera entrega, e o bastante para que a ida e volta não perca precisão.
const SECOND_SCALE: u32 = 10_000;

/// Coordenada pronta para o EXIF.
#[derive(Debug, Clone, PartialEq)]
pub struct ExifCoordinate {
    /// Graus, minutos e segundos como racionais sem sinal.
    pub dms: [uR64; 3],
    /// Hemisfério: `N`/`S` para latitude, `E`/`W` para longitude.
    pub reference: &'static str,
}

/// Converte uma latitude em graus decimais.
pub fn latitude_to_exif(degrees: f64) -> ExifCoordinate {
    ExifCoordinate {
        dms: to_dms(degrees.abs()),
        reference: if degrees < 0.0 { "S" } else { "N" },
    }
}

/// Converte uma longitude em graus decimais.
pub fn longitude_to_exif(degrees: f64) -> ExifCoordinate {
    ExifCoordinate {
        dms: to_dms(degrees.abs()),
        reference: if degrees < 0.0 { "W" } else { "E" },
    }
}

/// Altitude em metros, com a referência do EXIF.
///
/// `GPSAltitudeRef` vale 0 acima do nível do mar e 1 abaixo; a altitude em si é sempre positiva.
pub fn altitude_to_exif(meters: f64) -> (uR64, u8) {
    let scaled = (meters.abs() * 1000.0).round().min(f64::from(u32::MAX)) as u32;
    (
        uR64 {
            nominator: scaled,
            denominator: 1000,
        },
        u8::from(meters < 0.0),
    )
}

/// Decompõe graus decimais absolutos em grau, minuto e segundo.
fn to_dms(absolute: f64) -> [uR64; 3] {
    let degrees = absolute.trunc();
    let minutes_total = (absolute - degrees) * 60.0;
    let minutes = minutes_total.trunc();
    let seconds = (minutes_total - minutes) * 60.0;

    // O arredondamento dos segundos pode chegar a 60; nesse caso propaga para o minuto, e daí
    // possivelmente para o grau. Sem isso, o EXIF recebe "35° 0' 60"", que é inválido.
    let mut degrees = degrees as u32;
    let mut minutes = minutes as u32;
    let mut scaled_seconds = (seconds * f64::from(SECOND_SCALE)).round() as u32;

    if scaled_seconds >= 60 * SECOND_SCALE {
        scaled_seconds -= 60 * SECOND_SCALE;
        minutes += 1;
    }
    if minutes >= 60 {
        minutes -= 60;
        degrees += 1;
    }

    [
        uR64 {
            nominator: degrees,
            denominator: 1,
        },
        uR64 {
            nominator: minutes,
            denominator: 1,
        },
        uR64 {
            nominator: scaled_seconds,
            denominator: SECOND_SCALE,
        },
    ]
}

/// Recompõe graus decimais a partir do EXIF, aplicando o hemisfério.
///
/// Usada na verificação por releitura: escrever e não conferir é o caminho para descobrir que
/// a biblioteca falhou em silêncio só quando o acervo já foi restaurado errado.
pub fn from_exif(dms: [f64; 3], reference: &str) -> f64 {
    let magnitude = dms[0] + dms[1] / 60.0 + dms[2] / 3600.0;
    if matches!(reference.trim(), "S" | "W") {
        -magnitude
    } else {
        magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(degrees: f64, coordinate: &ExifCoordinate) -> f64 {
        let as_float = |r: &uR64| f64::from(r.nominator) / f64::from(r.denominator);
        let dms = [
            as_float(&coordinate.dms[0]),
            as_float(&coordinate.dms[1]),
            as_float(&coordinate.dms[2]),
        ];
        let recovered = from_exif(dms, coordinate.reference);
        assert!(
            (recovered - degrees).abs() < 1e-7,
            "ida e volta perdeu precisão: {degrees} virou {recovered}"
        );
        recovered
    }

    #[test]
    fn kyoto_is_north_and_east() {
        let lat = latitude_to_exif(35.0116);
        assert_eq!(lat.reference, "N");
        assert_eq!(
            lat.dms[0],
            uR64 {
                nominator: 35,
                denominator: 1
            }
        );
        assert_eq!(
            lat.dms[1],
            uR64 {
                nominator: 0,
                denominator: 1
            }
        );

        let lon = longitude_to_exif(135.7681);
        assert_eq!(lon.reference, "E");
        assert_eq!(
            lon.dms[0],
            uR64 {
                nominator: 135,
                denominator: 1
            }
        );
        assert_eq!(
            lon.dms[1],
            uR64 {
                nominator: 46,
                denominator: 1
            }
        );
    }

    #[test]
    fn porto_alegre_is_south_and_west() {
        // O teste que existe por causa do bug clássico: sem o hemisfério, esta foto vai parar
        // na Ucrânia.
        let lat = latitude_to_exif(-30.0346);
        assert_eq!(lat.reference, "S");
        assert_eq!(
            lat.dms[0],
            uR64 {
                nominator: 30,
                denominator: 1
            }
        );

        let lon = longitude_to_exif(-51.2177);
        assert_eq!(lon.reference, "W");
        assert_eq!(
            lon.dms[0],
            uR64 {
                nominator: 51,
                denominator: 1
            }
        );
    }

    #[test]
    fn magnitude_never_carries_the_sign() {
        // Os racionais do EXIF são sem sinal; o sinal vive apenas no campo de referência.
        for degrees in [-30.0346, -0.5, -179.9] {
            let coordinate = latitude_to_exif(degrees);
            assert!(coordinate.dms.iter().all(|r| r.nominator < u32::MAX));
            assert_eq!(coordinate.reference, "S");
        }
    }

    #[test]
    fn round_trips_both_hemispheres() {
        for degrees in [35.0116, -30.0346, 0.0, 89.999_999, -89.999_999] {
            round_trip(degrees, &latitude_to_exif(degrees));
        }
        for degrees in [135.7681, -51.2177, 0.0, 179.999_999, -179.999_999] {
            round_trip(degrees, &longitude_to_exif(degrees));
        }
    }

    #[test]
    fn zero_is_north_and_east_by_convention() {
        assert_eq!(latitude_to_exif(0.0).reference, "N");
        assert_eq!(longitude_to_exif(0.0).reference, "E");
    }

    #[test]
    fn seconds_rounding_to_sixty_carries_over() {
        // Um valor que arredonda os segundos para 60 produziria "35° 0' 60"", que é inválido.
        let coordinate = latitude_to_exif(35.0 + 59.999_999_99 / 3600.0);
        let seconds =
            f64::from(coordinate.dms[2].nominator) / f64::from(coordinate.dms[2].denominator);
        assert!(seconds < 60.0, "segundos não podem chegar a 60: {seconds}");
        assert!(coordinate.dms[1].nominator <= 60);
    }

    #[test]
    fn minutes_carry_into_degrees() {
        let coordinate = latitude_to_exif(35.0 - 1e-12);
        let degrees = coordinate.dms[0].nominator;
        let minutes = coordinate.dms[1].nominator;
        assert!(minutes < 60, "minutos não podem chegar a 60");
        assert!(degrees == 34 || degrees == 35);
    }

    #[test]
    fn altitude_above_and_below_sea_level() {
        let (value, reference) = altitude_to_exif(52.0);
        assert_eq!(reference, 0, "acima do nível do mar");
        assert_eq!(
            f64::from(value.nominator) / f64::from(value.denominator),
            52.0
        );

        let (value, reference) = altitude_to_exif(-12.5);
        assert_eq!(reference, 1, "abaixo do nível do mar");
        assert_eq!(
            f64::from(value.nominator) / f64::from(value.denominator),
            12.5,
            "a magnitude é sempre positiva"
        );
    }

    #[test]
    fn reading_back_applies_the_hemisphere() {
        assert!((from_exif([35.0, 0.0, 41.76], "N") - 35.0116).abs() < 1e-6);
        assert!((from_exif([30.0, 2.0, 4.56], "S") + 30.0346).abs() < 1e-6);
        // Espaço à direita acontece: o EXIF preenche strings com NUL e espaço.
        assert!((from_exif([51.0, 13.0, 3.72], "W ") + 51.2177).abs() < 1e-6);
    }
}
