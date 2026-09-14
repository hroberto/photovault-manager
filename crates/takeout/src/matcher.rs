//! Casamento entre arquivos de mídia e seus sidecars, dentro de um diretório.
//!
//! O escopo é sempre um diretório só. O Takeout repete o mesmo arquivo em várias pastas — em
//! `Photos from 2019` e em cada álbum — com sidecars possivelmente divergentes. Casar através de
//! fronteiras de diretório associa o sidecar errado ao arquivo certo.
//!
//! Ordem de tentativa, do mais seguro ao mais arriscado:
//!
//! 1. Chave exata, lendo o nome completo do sidecar como nome de mídia (formato antigo).
//! 2. Chave exata, depois de remover o sufixo supplemental.
//! 3. Truncamento: o nome da mídia no sidecar foi cortado, e só um candidato serve.
//!
//! Nada além disso. Se sobrar ambiguidade, o item vira órfão com o motivo registrado — nunca um
//! palpite silencioso.

use std::collections::{BTreeMap, HashMap};

use crate::filename::MediaKey;

/// Resultado do casamento de um diretório.
#[derive(Debug, Default)]
pub struct MatchReport {
    /// Pares mídia → sidecar.
    pub matched: Vec<Match>,
    /// Arquivos de mídia sem sidecar.
    pub media_without_sidecar: Vec<String>,
    /// Sidecars que não puderam ser associados, com o motivo.
    pub orphan_sidecars: Vec<OrphanSidecar>,
}

impl MatchReport {
    /// Quantos itens de mídia foram vistos.
    pub fn media_seen(&self) -> usize {
        self.matched.len() + self.media_without_sidecar.len()
    }

    /// Se algo precisa de revisão humana.
    pub fn needs_review(&self) -> bool {
        !self.orphan_sidecars.is_empty() || !self.media_without_sidecar.is_empty()
    }
}

/// Uma associação estabelecida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Nome do arquivo de mídia, como está no disco.
    pub media: String,
    /// Nome do arquivo de sidecar, como está no disco.
    pub sidecar: String,
    /// Como a associação foi obtida.
    pub confidence: MatchConfidence,
}

/// Quão segura é uma associação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    /// A chave do sidecar bate exatamente com a chave da mídia.
    Exact,
    /// O nome da mídia dentro do sidecar estava truncado, e havia um único candidato.
    Truncated,
}

/// Um sidecar que não encontrou dono.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanSidecar {
    /// Nome do arquivo.
    pub sidecar: String,
    /// Por que não foi associado.
    pub reason: OrphanReason,
}

/// Motivo de um sidecar ficar órfão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanReason {
    /// Nenhum arquivo de mídia no diretório corresponde.
    NoCandidate,
    /// Mais de um arquivo de mídia poderia ser o dono.
    ///
    /// Carrega os candidatos para que a revisão humana não precise adivinhar.
    Ambiguous(Vec<String>),
}

impl OrphanReason {
    /// Explicação para o relatório.
    pub fn describe(&self) -> String {
        match self {
            Self::NoCandidate => "nenhum arquivo de mídia correspondente no diretório".into(),
            Self::Ambiguous(candidates) => {
                format!("mais de um candidato possível: {}", candidates.join(", "))
            }
        }
    }
}

/// Casa mídias e sidecars de um mesmo diretório.
///
/// `entries` são os nomes de arquivo do diretório, sem caminho.
pub fn match_directory<I, S>(entries: I) -> MatchReport
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut media_files: Vec<String> = Vec::new();
    let mut sidecar_files: Vec<String> = Vec::new();

    for entry in entries {
        let name = entry.as_ref();
        if is_json(name) {
            sidecar_files.push(name.to_owned());
        } else {
            media_files.push(name.to_owned());
        }
    }

    // Índice das mídias por chave. BTreeMap para que a ordem dos candidatos numa ambiguidade
    // seja estável entre execuções — um relatório que muda sozinho não é auditável.
    let mut by_key: BTreeMap<MediaKey, Vec<String>> = BTreeMap::new();
    for name in &media_files {
        by_key
            .entry(MediaKey::from_media_filename(name))
            .or_default()
            .push(name.clone());
    }

    let mut report = MatchReport::default();
    let mut claimed: HashMap<String, ()> = HashMap::new();

    for sidecar in &sidecar_files {
        match resolve(sidecar, &by_key, &claimed) {
            Resolution::Matched(media, confidence) => {
                claimed.insert(media.clone(), ());
                report.matched.push(Match {
                    media,
                    sidecar: sidecar.clone(),
                    confidence,
                });
            }
            Resolution::Orphan(reason) => report.orphan_sidecars.push(OrphanSidecar {
                sidecar: sidecar.clone(),
                reason,
            }),
            Resolution::NotASidecar => {}
        }
    }

    for name in media_files {
        if !claimed.contains_key(&name) {
            report.media_without_sidecar.push(name);
        }
    }

    report.matched.sort_by(|a, b| a.media.cmp(&b.media));
    report.media_without_sidecar.sort();
    report
        .orphan_sidecars
        .sort_by(|a, b| a.sidecar.cmp(&b.sidecar));
    report
}

enum Resolution {
    Matched(String, MatchConfidence),
    Orphan(OrphanReason),
    NotASidecar,
}

fn resolve(
    sidecar: &str,
    by_key: &BTreeMap<MediaKey, Vec<String>>,
    claimed: &HashMap<String, ()>,
) -> Resolution {
    // Passo 1: o nome inteiro, sem `.json`, lido como nome de mídia. Cobre o formato antigo e
    // protege o arquivo raro cujo nome termina em algo parecido com o sufixo supplemental.
    if let Some(stem) = strip_json(sidecar) {
        let literal = MediaKey::from_media_filename(stem);
        if let Some(name) = pick_unclaimed(by_key.get(&literal), claimed) {
            return Resolution::Matched(name, MatchConfidence::Exact);
        }
    }

    // Passo 2: chave derivada com remoção do sufixo supplemental e do marcador deslocado.
    let Some(key) = MediaKey::from_sidecar_filename(sidecar) else {
        return Resolution::NotASidecar;
    };
    if let Some(name) = pick_unclaimed(by_key.get(&key), claimed) {
        return Resolution::Matched(name, MatchConfidence::Exact);
    }

    // Passo 3: o nome da mídia dentro do sidecar foi truncado. Aceita apenas se houver
    // exatamente um candidato ainda livre.
    let candidates: Vec<String> = by_key
        .iter()
        .filter(|(candidate, _)| key.could_be_truncation_of(candidate))
        .flat_map(|(_, names)| names.iter())
        .filter(|name| !claimed.contains_key(*name))
        .cloned()
        .collect();

    match candidates.len() {
        0 => Resolution::Orphan(OrphanReason::NoCandidate),
        1 => Resolution::Matched(candidates[0].clone(), MatchConfidence::Truncated),
        _ => Resolution::Orphan(OrphanReason::Ambiguous(candidates)),
    }
}

fn pick_unclaimed(names: Option<&Vec<String>>, claimed: &HashMap<String, ()>) -> Option<String> {
    names?
        .iter()
        .find(|name| !claimed.contains_key(*name))
        .cloned()
}

fn is_json(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("json"))
}

fn strip_json(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    ext.eq_ignore_ascii_case("json").then_some(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched_pairs(report: &MatchReport) -> Vec<(&str, &str)> {
        report
            .matched
            .iter()
            .map(|m| (m.media.as_str(), m.sidecar.as_str()))
            .collect()
    }

    #[test]
    fn happy_path() {
        let report = match_directory([
            "IMG_1002.JPG",
            "IMG_1002.JPG.supplemental-metadata.json",
            "IMG_1003.HEIC",
            "IMG_1003.HEIC.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 2);
        assert!(report.orphan_sidecars.is_empty());
        assert!(report.media_without_sidecar.is_empty());
        assert!(!report.needs_review());
        assert!(report
            .matched
            .iter()
            .all(|m| m.confidence == MatchConfidence::Exact));
    }

    #[test]
    fn mixed_truncations_in_one_directory() {
        // Todas as formas convivendo, como acontece de verdade.
        let report = match_directory([
            "a.jpg",
            "a.jpg.supplemental-metadata.json",
            "b.jpg",
            "b.jpg.supplemental-metadat.json",
            "c.jpg",
            "c.jpg.supple.json",
            "d.jpg",
            "d.jpg.s.json",
            "e.jpg",
            "e.jpg.json",
        ]);
        assert_eq!(report.matched.len(), 5);
        assert!(!report.needs_review());
    }

    #[test]
    fn duplicate_marker_pairs_correctly() {
        let report = match_directory([
            "IMG_1002.JPG",
            "IMG_1002.JPG.supplemental-metadata.json",
            "IMG_1002(1).JPG",
            "IMG_1002.JPG(1).supplemental-metadata.json",
        ]);
        assert_eq!(
            matched_pairs(&report),
            vec![
                (
                    "IMG_1002(1).JPG",
                    "IMG_1002.JPG(1).supplemental-metadata.json"
                ),
                ("IMG_1002.JPG", "IMG_1002.JPG.supplemental-metadata.json"),
            ]
        );
    }

    #[test]
    fn album_metadata_is_ignored_not_orphaned() {
        let report = match_directory([
            "metadata.json",
            "IMG_1002.JPG",
            "IMG_1002.JPG.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 1);
        assert!(report.orphan_sidecars.is_empty());
    }

    #[test]
    fn media_without_sidecar_is_reported() {
        let report = match_directory(["IMG_1002.JPG", "IMG_1003.JPG", "IMG_1002.JPG.json"]);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.media_without_sidecar, vec!["IMG_1003.JPG"]);
        assert!(report.needs_review());
    }

    #[test]
    fn orphan_sidecar_is_never_dropped_silently() {
        let report = match_directory(["IMG_1002.JPG", "IMG_9999.JPG.supplemental-metadata.json"]);
        assert!(report.matched.is_empty());
        assert_eq!(report.orphan_sidecars.len(), 1);
        assert_eq!(report.orphan_sidecars[0].reason, OrphanReason::NoCandidate);
        assert_eq!(report.media_without_sidecar, vec!["IMG_1002.JPG"]);
    }

    #[test]
    fn truncated_media_name_matches_single_candidate() {
        // O nome da mídia foi cortado dentro do sidecar; só um arquivo pode ser o dono.
        let report = match_directory([
            "PXL_20240817_202602411.LONG_EXPOSURE.jpg",
            "PXL_20240817_202602411.LONG_EXPO.jpg.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.matched[0].confidence, MatchConfidence::Truncated);
    }

    #[test]
    fn ambiguous_truncation_becomes_orphan_with_candidates() {
        // Dois arquivos poderiam ser o dono. Chutar aqui associa metadado à foto errada.
        let report = match_directory([
            "PXL_2024_alpha.jpg",
            "PXL_2024_beta.jpg",
            "PXL_2024_.jpg.supplemental-metadata.json",
        ]);
        assert!(report.matched.is_empty());
        assert_eq!(report.orphan_sidecars.len(), 1);
        match &report.orphan_sidecars[0].reason {
            OrphanReason::Ambiguous(candidates) => {
                assert_eq!(candidates.len(), 2);
                assert!(candidates.contains(&"PXL_2024_alpha.jpg".to_owned()));
            }
            other => panic!("esperava ambiguidade, veio {other:?}"),
        }
    }

    #[test]
    fn one_sidecar_per_media_file() {
        // Dois sidecars disputando o mesmo arquivo: o primeiro casa, o segundo vira órfão.
        let report = match_directory([
            "IMG_1002.JPG",
            "IMG_1002.JPG.json",
            "IMG_1002.JPG.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 1);
        assert_eq!(report.orphan_sidecars.len(), 1);
    }

    #[test]
    fn live_photo_pair_each_gets_its_sidecar() {
        let report = match_directory([
            "IMG_1002.HEIC",
            "IMG_1002.HEIC.supplemental-metadata.json",
            "IMG_1002.MP4",
            "IMG_1002.MP4.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 2);
        assert!(!report.needs_review());
    }

    #[test]
    fn edited_version_is_a_separate_file() {
        let report = match_directory([
            "IMG_1002.JPG",
            "IMG_1002.JPG.supplemental-metadata.json",
            "IMG_1002-edited.JPG",
        ]);
        assert_eq!(report.matched.len(), 1);
        // A versão editada normalmente não tem sidecar próprio; isso é esperado, não erro.
        assert_eq!(report.media_without_sidecar, vec!["IMG_1002-edited.JPG"]);
    }

    #[test]
    fn unicode_names_match_across_normalization_forms() {
        let report = match_directory([
            "Anivers\u{e1}rio.jpg",
            "Aniversa\u{301}rio.jpg.supplemental-metadata.json",
        ]);
        assert_eq!(report.matched.len(), 1);
    }

    #[test]
    fn empty_directory_is_fine() {
        let report = match_directory(Vec::<String>::new());
        assert_eq!(report.media_seen(), 0);
        assert!(!report.needs_review());
    }

    #[test]
    fn report_is_deterministic() {
        let entries = [
            "b.jpg",
            "a.jpg",
            "b.jpg.supplemental-metadata.json",
            "a.jpg.supplemental-metadata.json",
        ];
        let first = match_directory(entries);
        let second = match_directory(entries);
        assert_eq!(matched_pairs(&first), matched_pairs(&second));
    }
}
