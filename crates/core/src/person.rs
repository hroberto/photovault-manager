//! Pessoas marcadas em um item.
//!
//! O Takeout entrega apenas o nome — não as coordenadas do rosto. E não há API para devolver
//! essa marcação ao Google. O valor desses dados está em sobreviverem dentro do arquivo, como
//! região XMP sem retângulo e como palavra-chave, onde Lightroom, digiKam e Immich os leem.

use std::fmt;

/// Nome de uma pessoa, normalizado para comparação.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersonName {
    display: String,
    key: String,
}

impl PersonName {
    /// Constrói a partir do nome bruto do sidecar.
    ///
    /// Recusa nome vazio ou só com espaços: uma marcação sem nome não é informação.
    pub fn new(raw: &str) -> Option<Self> {
        let display = raw.trim();
        if display.is_empty() {
            return None;
        }
        Some(Self {
            display: display.to_owned(),
            key: normalize_key(display),
        })
    }

    /// Nome como deve ser exibido e gravado no arquivo.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Chave de comparação: minúsculas e espaços colapsados.
    ///
    /// "Ana Maria", "ana maria" e "Ana  Maria" são a mesma pessoa.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Display for PersonName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

fn normalize_key(text: &str) -> String {
    let mut key = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !key.is_empty();
            continue;
        }
        if pending_space {
            key.push(' ');
            pending_space = false;
        }
        key.extend(ch.to_lowercase());
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_names() {
        assert!(PersonName::new("").is_none());
        assert!(PersonName::new("   ").is_none());
    }

    #[test]
    fn keeps_display_form() {
        let person = PersonName::new("  Ana Maria  ").expect("nome válido");
        assert_eq!(person.display(), "Ana Maria");
    }

    #[test]
    fn same_person_despite_case_and_spacing() {
        let a = PersonName::new("Ana Maria").expect("nome válido");
        let b = PersonName::new("ana  maria").expect("nome válido");
        assert_eq!(a.key(), b.key());
        assert_ne!(a.display(), b.display());
    }

    #[test]
    fn handles_accents_and_unicode() {
        let person = PersonName::new("HENRIQUE JOSÉ").expect("nome válido");
        assert_eq!(person.key(), "henrique josé");
    }
}
