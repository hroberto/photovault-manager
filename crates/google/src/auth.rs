//! OAuth 2.0 com PKCE e redirecionamento para loopback.
//!
//! Aplicativo de computador não tem segredo: qualquer coisa embarcada no binário está ao alcance
//! de quem tiver o binário. Por isso o fluxo é PKCE — o desafio é gerado a cada autorização e
//! nunca sai da máquina, e o `client_secret` deixa de ser necessário.
//!
//! O redirecionamento vai para `http://127.0.0.1:<porta>`, aberto só durante a autorização. Não
//! se usa webview capturando a senha do usuário: a credencial do Google é digitada no navegador
//! do Google, e o PhotoVault nunca a vê.

use std::fmt;

use sha2::{Digest, Sha256};

/// Endpoint de autorização do Google.
pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Endpoint de troca e renovação de token.
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Escopo para enviar arquivos e criar álbuns.
pub const SCOPE_APPEND: &str = "https://www.googleapis.com/auth/photoslibrary.appendonly";

/// Escopo para reler o que o próprio aplicativo enviou.
///
/// É com ele que a restauração se verifica. Restauração que não se verifica não é restauração.
pub const SCOPE_READ_APP_DATA: &str =
    "https://www.googleapis.com/auth/photoslibrary.readonly.appcreateddata";

/// Escopo do Picker, para a importação incremental.
pub const SCOPE_PICKER: &str = "https://www.googleapis.com/auth/photospicker.mediaitems.readonly";

/// Escopo do Drive, para buscar os archives do Takeout entregues lá.
///
/// É o que elimina a pior parte da exportação: baixar oito arquivos de 50 GB à mão.
pub const SCOPE_DRIVE_READONLY: &str = "https://www.googleapis.com/auth/drive.readonly";

/// Desafio PKCE de uma autorização.
///
/// O `verifier` nunca sai da máquina. O `challenge` é o que viaja na URL.
#[derive(Clone)]
pub struct PkceChallenge {
    verifier: String,
    challenge: String,
}

impl PkceChallenge {
    /// Gera um desafio a partir de bytes aleatórios.
    ///
    /// Recebe a entropia em vez de produzi-la para que o teste seja determinístico. Em produção
    /// use [`PkceChallenge::generate`].
    pub fn from_entropy(entropy: &[u8; 32]) -> Self {
        let verifier = base64url(entropy);
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            challenge: base64url(&digest),
            verifier,
        }
    }

    /// Gera um desafio com entropia do sistema.
    pub fn generate() -> Self {
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy).expect("fonte de entropia do sistema");
        Self::from_entropy(&entropy)
    }

    /// Segredo que fica na máquina e é enviado só na troca do código.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// Valor derivado que viaja na URL de autorização.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

impl fmt::Debug for PkceChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // O verificador é segredo: nunca deve aparecer em log nem em relatório de erro.
        f.debug_struct("PkceChallenge")
            .field("challenge", &self.challenge)
            .field("verifier", &"<oculto>")
            .finish()
    }
}

/// Monta a URL para a qual o navegador do usuário é enviado.
pub fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    challenge: &PkceChallenge,
    state: &str,
) -> String {
    let scope = scopes.join(" ");
    format!(
        "{AUTH_ENDPOINT}?response_type=code\
         &client_id={}\
         &redirect_uri={}\
         &scope={}\
         &code_challenge={}\
         &code_challenge_method=S256\
         &state={}\
         &access_type=offline\
         &prompt=consent",
        encode(client_id),
        encode(redirect_uri),
        encode(&scope),
        encode(challenge.challenge()),
        encode(state),
    )
}

/// Tokens devolvidos pelo Google.
#[derive(Clone, serde::Deserialize)]
pub struct TokenSet {
    /// Token de acesso, de vida curta.
    pub access_token: String,
    /// Token de renovação. Só vem na primeira autorização.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Segundos até o token de acesso expirar.
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Escopos efetivamente concedidos.
    #[serde(default)]
    pub scope: Option<String>,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Um token em log é o vazamento mais provável deste projeto. Ver o modelo de ameaça
        // em `.claude/agents/vault-security.md`.
        f.debug_struct("TokenSet")
            .field("access_token", &"<oculto>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<oculto>"),
            )
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

impl TokenSet {
    /// Se os escopos concedidos incluem o pedido.
    ///
    /// O usuário pode desmarcar permissões na tela de consentimento. Descobrir isso agora é
    /// melhor do que descobrir no meio de uma restauração de seis dias.
    pub fn granted(&self, scope: &str) -> bool {
        self.scope
            .as_deref()
            .is_some_and(|granted| granted.split_whitespace().any(|item| item == scope))
    }
}

/// Onde as credenciais ficam guardadas.
///
/// O catálogo guarda apenas uma referência; o token vive no chaveiro do sistema operacional.
pub trait TokenStore: Send + Sync {
    /// Lê o token de renovação de uma conta.
    fn load(&self, account: &str) -> Option<String>;
    /// Guarda o token de renovação.
    fn store(&self, account: &str, refresh_token: &str) -> Result<(), String>;
    /// Remove a credencial.
    fn forget(&self, account: &str) -> Result<(), String>;
}

/// Armazenamento em memória, para testes.
#[derive(Debug, Default)]
pub struct InMemoryTokenStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl TokenStore for InMemoryTokenStore {
    fn load(&self, account: &str) -> Option<String> {
        self.entries.lock().ok()?.get(account).cloned()
    }

    fn store(&self, account: &str, refresh_token: &str) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "armazenamento de tokens envenenado".to_string())?
            .insert(account.to_owned(), refresh_token.to_owned());
        Ok(())
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "armazenamento de tokens envenenado".to_string())?
            .remove(account);
        Ok(())
    }
}

/// Codificação base64url sem preenchimento, como o PKCE exige.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let symbols = match chunk.len() {
            1 => 2,
            2 => 3,
            _ => 4,
        };
        for index in 0..symbols {
            let shift = 18 - index * 6;
            out.push(ALPHABET[((triple >> shift) & 0x3F) as usize] as char);
        }
    }
    out
}

/// Escapa um valor para query string.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_test_vector() {
        // RFC 7636, apêndice B: o verificador conhecido produz este desafio.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let digest = Sha256::digest(verifier.as_bytes());
        assert_eq!(
            base64url(&digest),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn challenge_is_derived_from_the_verifier() {
        let challenge = PkceChallenge::from_entropy(&[7u8; 32]);
        let expected = base64url(&Sha256::digest(challenge.verifier().as_bytes()));
        assert_eq!(challenge.challenge(), expected);
        assert_ne!(challenge.challenge(), challenge.verifier());
    }

    #[test]
    fn generated_challenges_differ() {
        let a = PkceChallenge::generate();
        let b = PkceChallenge::generate();
        assert_ne!(a.verifier(), b.verifier());
    }

    #[test]
    fn verifier_length_is_within_the_rfc_range() {
        // O RFC exige entre 43 e 128 caracteres.
        let challenge = PkceChallenge::generate();
        assert!((43..=128).contains(&challenge.verifier().len()));
    }

    #[test]
    fn base64url_has_no_padding_or_unsafe_characters() {
        for length in 1..=32 {
            let encoded = base64url(&vec![0xFBu8; length]);
            assert!(!encoded.contains('='), "não pode ter preenchimento");
            assert!(
                !encoded.contains('+') && !encoded.contains('/'),
                "alfabeto url-safe"
            );
        }
    }

    #[test]
    fn the_verifier_never_appears_in_debug_output() {
        // Um segredo em log é o vazamento mais provável deste projeto.
        let challenge = PkceChallenge::from_entropy(&[9u8; 32]);
        let printed = format!("{challenge:?}");
        assert!(!printed.contains(challenge.verifier()));
        assert!(printed.contains("<oculto>"));
    }

    #[test]
    fn tokens_never_appear_in_debug_output() {
        let tokens = TokenSet {
            access_token: "ya29.SEGREDO".into(),
            refresh_token: Some("1//REFRESH".into()),
            expires_in: Some(3599),
            scope: Some(SCOPE_APPEND.into()),
        };
        let printed = format!("{tokens:?}");
        assert!(!printed.contains("ya29.SEGREDO"));
        assert!(!printed.contains("1//REFRESH"));
        assert!(
            printed.contains("3599"),
            "o que não é segredo continua visível"
        );
    }

    #[test]
    fn authorization_url_carries_pkce_and_offline_access() {
        let challenge = PkceChallenge::from_entropy(&[1u8; 32]);
        let url = authorization_url(
            "123.apps.googleusercontent.com",
            "http://127.0.0.1:8731",
            &[SCOPE_APPEND, SCOPE_READ_APP_DATA],
            &challenge,
            "estado-aleatorio",
        );

        assert!(url.starts_with(AUTH_ENDPOINT));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&encode(challenge.challenge())));
        // Sem `access_type=offline` não vem refresh_token, e a restauração de seis dias morre
        // quando o token de acesso expira em uma hora.
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("state=estado-aleatorio"));
        // O verificador jamais viaja.
        assert!(!url.contains(challenge.verifier()));
    }

    #[test]
    fn authorization_url_escapes_the_redirect_and_scopes() {
        let challenge = PkceChallenge::from_entropy(&[2u8; 32]);
        let url = authorization_url(
            "id",
            "http://127.0.0.1:8731",
            &[SCOPE_APPEND],
            &challenge,
            "s",
        );
        assert!(url.contains("http%3A%2F%2F127.0.0.1%3A8731"));
        assert!(
            !url.contains("scope=https://"),
            "o escopo precisa vir escapado"
        );
    }

    #[test]
    fn detects_a_scope_the_user_declined() {
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            expires_in: None,
            scope: Some(SCOPE_APPEND.into()),
        };
        assert!(tokens.granted(SCOPE_APPEND));
        // O usuário pode desmarcar permissões na tela de consentimento. Melhor saber agora do
        // que no meio de uma restauração.
        assert!(!tokens.granted(SCOPE_READ_APP_DATA));
    }

    #[test]
    fn scope_matching_is_exact_not_prefix() {
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            expires_in: None,
            scope: Some(SCOPE_READ_APP_DATA.into()),
        };
        // `photoslibrary.readonly.appcreateddata` não concede `photoslibrary.readonly`.
        assert!(!tokens.granted("https://www.googleapis.com/auth/photoslibrary.readonly"));
    }

    #[test]
    fn in_memory_store_round_trips() {
        let store = InMemoryTokenStore::default();
        assert!(store.load("conta@exemplo.com").is_none());

        store
            .store("conta@exemplo.com", "1//REFRESH")
            .expect("guarda");
        assert_eq!(
            store.load("conta@exemplo.com").as_deref(),
            Some("1//REFRESH")
        );

        store.forget("conta@exemplo.com").expect("esquece");
        assert!(store.load("conta@exemplo.com").is_none());
    }
}
