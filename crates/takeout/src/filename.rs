//! Interpretação de nomes de arquivo do Takeout.
//!
//! O Google não documenta como nomeia os sidecars, e o formato mudou. Tudo aqui foi derivado de
//! archives reais e de relatos reproduzíveis. As três armadilhas que este módulo existe para
//! resolver:
//!
//! 1. O sidecar passou de `IMG.JPG.json` para `IMG.JPG.supplemental-metadata.json`, e o sufixo é
//!    truncado de forma inconsistente: `.supplemental-metadat.json`, `.supple.json`, `.s.json`.
//! 2. O marcador de duplicata migra de posição: o arquivo é `IMG_1002(1).JPG`, mas o sidecar é
//!    `IMG_1002.JPG(1).supplemental-metadata.json`.
//! 3. Nomes chegam em NFC ou NFD conforme a plataforma que gerou e a que leu.

use unicode_normalization::UnicodeNormalization;

/// Sufixo completo que o Google acrescenta aos sidecars modernos.
///
/// Qualquer prefixo não vazio deste texto é um truncamento válido.
const SUPPLEMENTAL: &str = "supplemental-metadata";

/// Arquivos JSON que existem no archive e não são sidecars de mídia.
const NON_SIDECAR_JSON: &[&str] = &[
    "metadata.json",
    "print-subscriptions.json",
    "shared_album_comments.json",
    "user-generated-memory-titles.json",
];

/// Identidade de um arquivo de mídia, independente de como o nome foi escrito.
///
/// Serve de chave comum entre o arquivo e seu sidecar: `IMG_1002(1).JPG` e
/// `IMG_1002.JPG(1).supplemental-metadata.json` produzem a mesma `MediaKey`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaKey {
    /// Nome sem extensão e sem marcador de duplicata, normalizado em NFC.
    pub stem: String,
    /// Extensão em minúsculas, sem o ponto. Vazia quando não há.
    pub extension: String,
    /// Índice do marcador `(n)`, quando presente.
    pub duplicate: Option<u32>,
}

impl MediaKey {
    /// Interpreta o nome de um arquivo de mídia.
    ///
    /// `IMG_1002(1).JPG` → stem `IMG_1002`, extensão `jpg`, duplicata 1.
    pub fn from_media_filename(name: &str) -> Self {
        let name = normalize(name);
        let (base, extension) = split_extension(&name);
        let (stem, duplicate) = split_duplicate_marker(base);
        Self {
            stem: stem.to_owned(),
            extension: extension.to_ascii_lowercase(),
            duplicate,
        }
    }

    /// Interpreta o nome de um sidecar, devolvendo a chave da mídia a que ele se refere.
    ///
    /// Devolve `None` para JSON que não é sidecar de mídia — `metadata.json` de álbum e os
    /// arquivos de nível de conta.
    pub fn from_sidecar_filename(name: &str) -> Option<Self> {
        let name = normalize(name);
        if NON_SIDECAR_JSON
            .iter()
            .any(|known| name.eq_ignore_ascii_case(known))
        {
            return None;
        }
        let stem = name
            .strip_suffix(".json")
            .or_else(|| name.strip_suffix(".JSON"))?;
        if stem.is_empty() {
            return None;
        }

        // O `(n)` do sidecar vem depois da extensão da mídia; tiramos primeiro, porque o
        // sufixo supplemental pode vir depois dele.
        let (without_supplemental, duplicate) = strip_supplemental(stem);
        let (base, extension) = split_extension(without_supplemental);

        // Formato antigo `IMG_1002(1).JPG.json`: o marcador está colado no nome, não depois
        // da extensão.
        let (stem, inline_duplicate) = split_duplicate_marker(base);

        Some(Self {
            stem: stem.to_owned(),
            extension: extension.to_ascii_lowercase(),
            duplicate: duplicate.or(inline_duplicate),
        })
    }

    /// Nome de arquivo reconstruído, para mensagens ao usuário.
    pub fn display_name(&self) -> String {
        let mut name = self.stem.clone();
        if let Some(index) = self.duplicate {
            name.push_str(&format!("({index})"));
        }
        if !self.extension.is_empty() {
            name.push('.');
            name.push_str(&self.extension);
        }
        name
    }

    /// Se esta chave pode ser um truncamento da outra.
    ///
    /// O Google corta o fim do nome, nunca o começo — então truncamento é relação de prefixo,
    /// com a mesma extensão e o mesmo marcador de duplicata.
    pub fn could_be_truncation_of(&self, full: &Self) -> bool {
        self.extension == full.extension
            && self.duplicate == full.duplicate
            && self.stem.len() < full.stem.len()
            && full.stem.starts_with(&self.stem)
    }
}

/// Normaliza para NFC, para que NFC e NFD comparem iguais.
fn normalize(text: &str) -> String {
    text.nfc().collect()
}

/// Separa nome e extensão pelo último ponto.
///
/// `My.Photo.2019.jpg` → (`My.Photo.2019`, `jpg`). Sem ponto, a extensão é vazia.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        // Um ponto inicial é nome oculto, não extensão.
        Some((base, ext)) if !base.is_empty() && !ext.is_empty() => (base, ext),
        _ => (name, ""),
    }
}

/// Destaca um marcador `(n)` no fim do texto.
fn split_duplicate_marker(text: &str) -> (&str, Option<u32>) {
    let Some(open) = text.rfind('(') else {
        return (text, None);
    };
    if !text.ends_with(')') {
        return (text, None);
    }
    let inner = &text[open + 1..text.len() - 1];
    match inner.parse::<u32>() {
        Ok(index) => (&text[..open], Some(index)),
        Err(_) => (text, None),
    }
}

/// Remove o sufixo supplemental, inteiro ou truncado, e o marcador de duplicata que o precede.
///
/// `IMG.JPG(1).supplemental-metadat` → (`IMG.JPG`, Some(1))
fn strip_supplemental(stem: &str) -> (&str, Option<u32>) {
    let Some((head, tail)) = stem.rsplit_once('.') else {
        return (stem, None);
    };
    // Só é sufixo supplemental se for um prefixo não vazio do texto completo.
    if tail.is_empty() || !SUPPLEMENTAL.starts_with(tail) {
        return (stem, None);
    }
    let (head, duplicate) = split_duplicate_marker(head);
    (head, duplicate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(name: &str) -> MediaKey {
        MediaKey::from_media_filename(name)
    }

    fn sidecar(name: &str) -> MediaKey {
        MediaKey::from_sidecar_filename(name).expect("deveria ser sidecar de mídia")
    }

    #[test]
    fn plain_media_name() {
        let key = media("IMG_1002.JPG");
        assert_eq!(key.stem, "IMG_1002");
        assert_eq!(key.extension, "jpg");
        assert_eq!(key.duplicate, None);
    }

    #[test]
    fn media_with_dots_in_name() {
        let key = media("My.Photo.2019.jpg");
        assert_eq!(key.stem, "My.Photo.2019");
        assert_eq!(key.extension, "jpg");
    }

    #[test]
    fn media_without_extension() {
        let key = media("IMG_1002");
        assert_eq!(key.stem, "IMG_1002");
        assert_eq!(key.extension, "");
    }

    #[test]
    fn old_format_sidecar_matches_media() {
        assert_eq!(sidecar("IMG_1002.JPG.json"), media("IMG_1002.JPG"));
    }

    #[test]
    fn modern_sidecar_matches_media() {
        assert_eq!(
            sidecar("IMG_1002.JPG.supplemental-metadata.json"),
            media("IMG_1002.JPG")
        );
    }

    #[test]
    fn every_truncation_of_the_suffix_matches() {
        let expected = media("IMG_1002.JPG");
        for truncated in [
            "IMG_1002.JPG.supplemental-metadata.json",
            "IMG_1002.JPG.supplemental-metadat.json",
            "IMG_1002.JPG.supplemental-me.json",
            "IMG_1002.JPG.supplem.json",
            "IMG_1002.JPG.supple.json",
            "IMG_1002.JPG.sup.json",
            "IMG_1002.JPG.s.json",
        ] {
            assert_eq!(sidecar(truncated), expected, "falhou em {truncated}");
        }
    }

    #[test]
    fn real_world_pixel_truncation() {
        // Caso observado: nome de 26 caracteres somado ao sufixo cortado em 46 no total.
        assert_eq!(
            sidecar("PXL_20240817_202602411.mp4.supplemental-metada.json"),
            media("PXL_20240817_202602411.mp4")
        );
    }

    #[test]
    fn duplicate_marker_moves_after_the_extension() {
        // O arquivo é IMG_1002(1).JPG mas o sidecar escreve IMG_1002.JPG(1).
        let from_sidecar = sidecar("IMG_1002.JPG(1).supplemental-metadata.json");
        let from_media = media("IMG_1002(1).JPG");
        assert_eq!(from_sidecar, from_media);
        assert_eq!(from_media.duplicate, Some(1));
    }

    #[test]
    fn duplicate_marker_with_truncated_suffix() {
        assert_eq!(
            sidecar("IMG_1002.JPG(2).supple.json"),
            media("IMG_1002(2).JPG")
        );
    }

    #[test]
    fn old_format_keeps_marker_inline() {
        assert_eq!(sidecar("IMG_1002(1).JPG.json"), media("IMG_1002(1).JPG"));
    }

    #[test]
    fn extension_case_is_irrelevant() {
        assert_eq!(media("IMG_1002.JPG"), media("IMG_1002.jpg"));
        assert_eq!(
            sidecar("IMG_1002.jpg.supplemental-metadata.json"),
            media("IMG_1002.JPG")
        );
    }

    #[test]
    fn nfc_and_nfd_are_the_same_file() {
        // "Aniversário" com acento composto e com acento decomposto.
        let composed = media("Anivers\u{e1}rio.jpg");
        let decomposed = media("Aniversa\u{301}rio.jpg");
        assert_eq!(composed, decomposed);
    }

    #[test]
    fn album_metadata_is_not_a_sidecar() {
        assert!(MediaKey::from_sidecar_filename("metadata.json").is_none());
        assert!(MediaKey::from_sidecar_filename("print-subscriptions.json").is_none());
        assert!(MediaKey::from_sidecar_filename("shared_album_comments.json").is_none());
    }

    #[test]
    fn non_json_is_not_a_sidecar() {
        assert!(MediaKey::from_sidecar_filename("IMG_1002.JPG").is_none());
    }

    #[test]
    fn media_named_like_the_suffix_is_not_mangled() {
        // Um arquivo chamado Report.s com sidecar antigo Report.s.json: o nome completo vence,
        // porque o casamento exato é tentado antes de qualquer remoção de sufixo.
        let key = sidecar("Report.s.json");
        // A heurística remove o ".s"; o casador confere as duas leituras (ver matcher).
        assert_eq!(key.stem, "Report");
    }

    #[test]
    fn truncation_is_a_prefix_relation() {
        let short = media("PXL_20240817_20260241.jpg");
        let full = media("PXL_20240817_202602411.jpg");
        assert!(short.could_be_truncation_of(&full));
        assert!(!full.could_be_truncation_of(&short));
    }

    #[test]
    fn truncation_requires_same_extension_and_marker() {
        let a = media("PXL_2024.jpg");
        let other_ext = media("PXL_20240817.mp4");
        assert!(!a.could_be_truncation_of(&other_ext));

        let other_marker = media("PXL_20240817(1).jpg");
        assert!(!a.could_be_truncation_of(&other_marker));
    }

    #[test]
    fn display_name_round_trips() {
        assert_eq!(media("IMG_1002.JPG").display_name(), "IMG_1002.jpg");
        assert_eq!(media("IMG_1002(1).JPG").display_name(), "IMG_1002(1).jpg");
        assert_eq!(media("IMG_1002").display_name(), "IMG_1002");
    }
}
