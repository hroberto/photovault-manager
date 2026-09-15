//! Parentesco entre arquivos de um mesmo diretório.
//!
//! Existe para que a deduplicação não elimine o que não é duplicata. Dois casos reais:
//!
//! - `IMG_1002.JPG` e `IMG_1002-edited.JPG` têm hash perceptual quase idêntico e são coisas
//!   diferentes. Um deduplicador ingênuo apaga a versão editada.
//! - `IMG_1004.HEIC` e `IMG_1004.MP4` são **um** item — uma Live Photo que o Takeout exporta
//!   partida em dois arquivos. Não são cópias um do outro.
//!
//! A detecção é por nome, que é o único sinal disponível antes de decodificar os arquivos. Por
//! isso é deliberadamente conservadora: na dúvida, não relaciona. Um vínculo perdido custa uma
//! revisão manual; um vínculo inventado custa uma foto.

use std::collections::BTreeMap;

use crate::filename::MediaKey;

/// Sufixos que o Google acrescenta a uma versão editada, por idioma da conta.
///
/// A lista é aberta por natureza: uma conta em idioma não previsto produz um sufixo que não
/// reconhecemos, e o arquivo entra como item independente — que é o comportamento seguro.
const EDITED_SUFFIXES: &[&str] = &[
    "-edited",     // inglês
    "-editado",    // português, espanhol
    "-modifié",    // francês
    "-bearbeitet", // alemão
    "-bewerkt",    // neerlandês
    "-redigerad",  // sueco
    "-muokattu",   // finlandês
    "-redigeret",  // dinamarquês
    "-redigert",   // norueguês
    "-modificato", // italiano
    "-edytowane",  // polonês
    "-düzenlendi", // turco
];

/// Extensões que costumam carregar o componente de movimento de uma Live Photo.
const MOTION_EXTENSIONS: &[&str] = &["mp4", "mov"];

/// Extensões de imagem que podem ter componente de movimento.
const STILL_EXTENSIONS: &[&str] = &["heic", "heif", "jpg", "jpeg"];

/// Um vínculo detectado entre dois arquivos.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relation {
    /// O primeiro é versão editada do segundo.
    EditedFrom {
        /// Arquivo derivado.
        edited: String,
        /// Arquivo original.
        original: String,
    },
    /// O primeiro é o componente de vídeo da Live Photo cuja imagem é o segundo.
    MotionPartOf {
        /// Arquivo de vídeo.
        motion: String,
        /// Arquivo de imagem.
        still: String,
    },
}

/// Detecta vínculos entre os arquivos de mídia de um diretório.
///
/// Recebe apenas nomes de arquivos de mídia — sidecars já devem ter sido filtrados.
pub fn detect<I, S>(media_files: I) -> Vec<Relation>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let names: Vec<String> = media_files
        .into_iter()
        .map(|n| n.as_ref().to_owned())
        .collect();

    // Índice por chave, para achar o parente sem varrer a lista toda a cada arquivo.
    // BTreeMap para que a saída seja estável entre execuções.
    let mut by_key: BTreeMap<MediaKey, Vec<String>> = BTreeMap::new();
    for name in &names {
        by_key
            .entry(MediaKey::from_media_filename(name))
            .or_default()
            .push(name.clone());
    }

    let mut relations = Vec::new();

    for name in &names {
        let key = MediaKey::from_media_filename(name);

        if let Some(base_stem) = strip_edited_suffix(&key.stem) {
            // A versão editada costuma vir como JPG mesmo quando o original é HEIC, então não
            // se compara a extensão. Mas se compara a categoria: a edição de uma foto nunca
            // deriva de um vídeo, e numa Live Photo o MP4 disputaria o lugar do HEIC.
            let edited_is_motion = is_motion(&key.extension);
            let candidates: Vec<&String> = by_key
                .iter()
                .filter(|(candidate, _)| {
                    candidate.stem == base_stem
                        && candidate.duplicate == key.duplicate
                        && is_motion(&candidate.extension) == edited_is_motion
                })
                .flat_map(|(_, files)| files.iter())
                .collect();

            // Só relaciona quando não há ambiguidade.
            if let [original] = candidates.as_slice() {
                relations.push(Relation::EditedFrom {
                    edited: name.clone(),
                    original: (*original).clone(),
                });
            }
            continue;
        }

        if MOTION_EXTENSIONS.contains(&key.extension.as_str()) {
            let stills: Vec<&String> = by_key
                .iter()
                .filter(|(candidate, _)| {
                    candidate.stem == key.stem
                        && candidate.duplicate == key.duplicate
                        && STILL_EXTENSIONS.contains(&candidate.extension.as_str())
                })
                .flat_map(|(_, files)| files.iter())
                .collect();

            if let [still] = stills.as_slice() {
                relations.push(Relation::MotionPartOf {
                    motion: name.clone(),
                    still: (*still).clone(),
                });
            }
        }
    }

    relations.sort();
    relations
}

/// Se a extensão é de um contêiner de vídeo.
///
/// A lista é mais larga que `MOTION_EXTENSIONS` porque aqui o objetivo é separar categorias,
/// não identificar o componente de uma Live Photo.
fn is_motion(extension: &str) -> bool {
    const VIDEO: &[&str] = &[
        "mp4", "mov", "avi", "mkv", "m4v", "3gp", "3g2", "mts", "m2ts", "m2t", "wmv", "mpg",
        "mpeg", "webm", "mod", "tod", "divx", "asf", "mmv",
    ];
    VIDEO.contains(&extension)
}

/// Remove o sufixo de edição, quando presente.
fn strip_edited_suffix(stem: &str) -> Option<String> {
    let lowered = stem.to_lowercase();
    for suffix in EDITED_SUFFIXES {
        if let Some(base) = lowered.strip_suffix(suffix) {
            if base.is_empty() {
                continue;
            }
            // Devolve recortado do original, para preservar a caixa do nome.
            return Some(stem[..base.len()].to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_edited_version() {
        let relations = detect(["IMG_1002.JPG", "IMG_1002-edited.JPG"]);
        assert_eq!(
            relations,
            vec![Relation::EditedFrom {
                edited: "IMG_1002-edited.JPG".into(),
                original: "IMG_1002.JPG".into(),
            }]
        );
    }

    #[test]
    fn detects_edited_version_across_formats() {
        // O Google entrega a edição de um HEIC como JPG.
        let relations = detect(["IMG_1002.HEIC", "IMG_1002-edited.JPG"]);
        assert_eq!(relations.len(), 1);
        assert!(
            matches!(&relations[0], Relation::EditedFrom { original, .. } if original == "IMG_1002.HEIC")
        );
    }

    #[test]
    fn detects_localized_edited_suffixes() {
        for (edited, language) in [
            ("IMG_1.JPG", "sem sufixo"),
            ("IMG_1-editado.JPG", "português"),
            ("IMG_1-bearbeitet.JPG", "alemão"),
            ("IMG_1-modifié.JPG", "francês"),
            ("IMG_1-muokattu.JPG", "finlandês"),
        ]
        .iter()
        .skip(1)
        {
            let relations = detect(["IMG_1.JPG", edited]);
            assert_eq!(relations.len(), 1, "falhou em {language}: {edited}");
        }
    }

    #[test]
    fn unknown_language_suffix_stays_independent() {
        // Comportamento seguro: um sufixo que não reconhecemos vira item próprio.
        let relations = detect(["IMG_1.JPG", "IMG_1-szerkesztett.JPG"]);
        assert!(relations.is_empty());
    }

    #[test]
    fn detects_live_photo_pair() {
        let relations = detect(["IMG_1004.HEIC", "IMG_1004.MP4"]);
        assert_eq!(
            relations,
            vec![Relation::MotionPartOf {
                motion: "IMG_1004.MP4".into(),
                still: "IMG_1004.HEIC".into(),
            }]
        );
    }

    #[test]
    fn video_without_a_still_is_just_a_video() {
        let relations = detect(["VID_2024.MP4", "IMG_1002.JPG"]);
        assert!(relations.is_empty());
    }

    #[test]
    fn two_stills_with_the_same_stem_block_the_pairing() {
        // IMG_1004.HEIC e IMG_1004.JPG: não dá para saber a qual o MP4 pertence.
        // Na dúvida, não relaciona.
        let relations = detect(["IMG_1004.HEIC", "IMG_1004.JPG", "IMG_1004.MP4"]);
        assert!(
            !relations
                .iter()
                .any(|r| matches!(r, Relation::MotionPartOf { .. })),
            "ambiguidade não pode virar palpite: {relations:?}"
        );
    }

    #[test]
    fn duplicate_markers_pair_with_their_own_copy() {
        let relations = detect([
            "IMG_1004.HEIC",
            "IMG_1004.MP4",
            "IMG_1004(1).HEIC",
            "IMG_1004(1).MP4",
        ]);
        assert_eq!(relations.len(), 2);
        assert!(relations.contains(&Relation::MotionPartOf {
            motion: "IMG_1004(1).MP4".into(),
            still: "IMG_1004(1).HEIC".into(),
        }));
    }

    #[test]
    fn edited_and_motion_coexist() {
        let relations = detect(["IMG_1002.HEIC", "IMG_1002.MP4", "IMG_1002-edited.JPG"]);
        assert_eq!(relations.len(), 2);
    }

    #[test]
    fn ambiguous_original_is_not_guessed() {
        // Dois candidatos a original com o mesmo stem e extensões diferentes.
        let relations = detect(["IMG_1.JPG", "IMG_1.PNG", "IMG_1-edited.JPG"]);
        assert!(
            relations.is_empty(),
            "não deve escolher entre dois originais"
        );
    }

    #[test]
    fn edited_without_original_is_independent() {
        let relations = detect(["IMG_1002-edited.JPG"]);
        assert!(relations.is_empty());
    }

    #[test]
    fn output_is_deterministic() {
        let files = ["b.MP4", "a.JPG", "b.HEIC", "a-edited.JPG"];
        assert_eq!(detect(files), detect(files));
    }

    #[test]
    fn empty_directory_has_no_relations() {
        assert!(detect(Vec::<String>::new()).is_empty());
    }
}
