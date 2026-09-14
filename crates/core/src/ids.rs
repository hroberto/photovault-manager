//! Identificadores tipados.
//!
//! Nenhuma `String` crua atravessa fronteira de módulo. Trocar um id de mídia por um id remoto
//! é o tipo de erro que o compilador deve pegar, não o usuário.

use std::fmt;

/// Identificador de um item de mídia dentro do catálogo local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaId(i64);

impl MediaId {
    /// Cria um `MediaId` a partir do valor persistido no catálogo.
    pub const fn new(raw: i64) -> Self {
        Self(raw)
    }

    /// Valor bruto, para persistência.
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for MediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "media:{}", self.0)
    }
}

/// Hash BLAKE3 dos bytes de um objeto — a identidade de um arquivo no CAS.
///
/// Guardado como os 32 bytes, não como texto: comparação e uso como chave ficam baratos, e a
/// representação hexadecimal é derivada quando precisa aparecer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectHash([u8; 32]);

impl ObjectHash {
    /// Constrói a partir dos bytes do digest.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Constrói a partir de um digest BLAKE3.
    pub fn from_hash(hash: blake3::Hash) -> Self {
        Self(*hash.as_bytes())
    }

    /// Bytes do digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Representação hexadecimal em minúsculas, 64 caracteres.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            // Não pode falhar: escrita em String.
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Lê de uma representação hexadecimal de 64 caracteres.
    pub fn from_hex(text: &str) -> Result<Self, InvalidObjectHash> {
        if text.len() != 64 {
            return Err(InvalidObjectHash::Length(text.len()));
        }
        let mut bytes = [0u8; 32];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let pair = &text[index * 2..index * 2 + 2];
            *slot = u8::from_str_radix(pair, 16).map_err(|_| InvalidObjectHash::NotHex)?;
        }
        Ok(Self(bytes))
    }

    /// Prefixo de dois caracteres usado como diretório de fanout no CAS.
    pub fn fanout(self) -> String {
        format!("{:02x}", self.0[0])
    }
}

impl fmt::Display for ObjectHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for ObjectHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Digest inteiro polui o log; os 12 primeiros caracteres identificam sem ruído.
        write!(f, "ObjectHash({}…)", &self.to_hex()[..12])
    }
}

/// Erro ao interpretar um hash textual.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidObjectHash {
    /// Comprimento diferente de 64 caracteres.
    #[error("hash deve ter 64 caracteres hexadecimais, veio com {0}")]
    Length(usize),
    /// Caractere fora do alfabeto hexadecimal.
    #[error("hash contém caractere não hexadecimal")]
    NotHex,
}

/// Identificador de um item já enviado a um destino remoto.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteMediaId(String);

impl RemoteMediaId {
    /// Constrói a partir do identificador devolvido pelo destino.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Valor bruto.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RemoteMediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_hex() {
        let hash = ObjectHash::from_hash(blake3::hash(b"photovault"));
        let hex = hash.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(ObjectHash::from_hex(&hex), Ok(hash));
    }

    #[test]
    fn hash_rejects_bad_input() {
        assert_eq!(
            ObjectHash::from_hex("abc"),
            Err(InvalidObjectHash::Length(3))
        );
        assert_eq!(
            ObjectHash::from_hex(&"z".repeat(64)),
            Err(InvalidObjectHash::NotHex)
        );
    }

    #[test]
    fn fanout_is_first_byte() {
        let hash = ObjectHash::from_bytes([0xa1; 32]);
        assert_eq!(hash.fanout(), "a1");
        assert!(hash.to_hex().starts_with("a1a1"));
    }

    #[test]
    fn debug_does_not_dump_whole_digest() {
        let hash = ObjectHash::from_bytes([0xab; 32]);
        assert_eq!(format!("{hash:?}"), "ObjectHash(abababababab…)");
    }
}
