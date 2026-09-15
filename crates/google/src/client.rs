//! Cliente da Library API do Google Fotos.
//!
//! Cobre o caminho de volta: enviar bytes, criar itens, recriar álbuns e verificar o que foi
//! enviado. Tudo com o escopo `photoslibrary.appendonly`, o único que ainda permite escrever na
//! biblioteca do usuário desde 31/03/2025.
//!
//! O `base_url` é configurável para que os testes apontem para um servidor simulado. Em
//! produção aponta para `https://photoslibrary.googleapis.com`.

use serde::Deserialize;

/// Endereço real da API.
pub const PRODUCTION_BASE: &str = "https://photoslibrary.googleapis.com";

/// Máximo de itens por chamada de criação em lote, imposto pela API.
pub const BATCH_LIMIT: usize = 50;

/// Falha ao falar com a API.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Falha de rede ou de protocolo.
    #[error("rede: {0}")]
    Transport(String),
    /// A cota do projeto estourou.
    ///
    /// Não é acidente: é o curso normal de uma restauração longa. Quem chama deve reagendar,
    /// não falhar.
    #[error("cota esgotada (429)")]
    QuotaExceeded,
    /// Credencial inválida ou expirada.
    #[error("autenticação recusada ({0})")]
    Unauthorized(u16),
    /// A API recusou o pedido.
    #[error("a API recusou ({status}): {message}")]
    Rejected {
        /// Código HTTP.
        status: u16,
        /// Mensagem devolvida.
        message: String,
    },
    /// Resposta que não se consegue interpretar.
    #[error("resposta inesperada: {0}")]
    Malformed(String),
}

impl ApiError {
    /// Se vale a pena tentar de novo mais tarde.
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::QuotaExceeded | Self::Transport(_) => true,
            Self::Rejected { status, .. } => *status >= 500,
            Self::Unauthorized(_) | Self::Malformed(_) => false,
        }
    }
}

type Result<T> = std::result::Result<T, ApiError>;

/// Cliente autenticado.
#[derive(Debug, Clone)]
pub struct PhotosClient {
    http: reqwest::Client,
    base_url: String,
    access_token: String,
}

/// Um item a criar, já com os bytes enviados.
#[derive(Debug, Clone)]
pub struct PendingItem {
    /// Token devolvido pelo envio dos bytes.
    pub upload_token: String,
    /// Nome do arquivo, exibido no Google Fotos.
    pub filename: String,
    /// Descrição. É o único metadado que a API aceita por campo.
    ///
    /// Data e geolocalização não têm campo: o Google os lê do EXIF dos bytes enviados. Por isso
    /// a normalização é obrigatória antes do envio.
    pub description: Option<String>,
}

/// Resultado da criação de um item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedItem {
    /// Token que originou o item, para casar com o pedido.
    pub upload_token: String,
    /// Identificador do item no Google, quando deu certo.
    pub remote_id: Option<String>,
    /// Mensagem de erro, quando não deu.
    pub error: Option<String>,
}

impl CreatedItem {
    /// Se o item foi efetivamente criado.
    pub const fn succeeded(&self) -> bool {
        self.remote_id.is_some()
    }
}

impl PhotosClient {
    /// Cria um cliente apontando para a API real.
    pub fn new(access_token: impl Into<String>) -> Self {
        Self::with_base_url(access_token, PRODUCTION_BASE)
    }

    /// Cria um cliente apontando para um endereço específico. Usado pelos testes.
    pub fn with_base_url(access_token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            access_token: access_token.into(),
        }
    }

    /// Envia os bytes de um arquivo e devolve o token de envio.
    ///
    /// Os bytes vão crus no corpo, com os cabeçalhos que a API exige. Nada de multipart.
    pub async fn upload_bytes(&self, filename: &str, bytes: Vec<u8>) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/v1/uploads", self.base_url))
            .bearer_auth(&self.access_token)
            .header("Content-Type", "application/octet-stream")
            .header("X-Goog-Upload-Content-Type", "application/octet-stream")
            .header("X-Goog-Upload-Protocol", "raw")
            .header("X-Goog-File-Name", sanitize_header(filename))
            .body(bytes)
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;

        if !status.is_success() {
            return Err(classify(status.as_u16(), body));
        }
        let token = body.trim().to_owned();
        if token.is_empty() {
            return Err(ApiError::Malformed("envio devolveu token vazio".into()));
        }
        Ok(token)
    }

    /// Cria até 50 itens de uma vez.
    ///
    /// A API responde por item: alguns podem falhar enquanto outros passam. Devolvemos o
    /// resultado de cada um para que a retomada saiba exatamente o que refazer.
    pub async fn batch_create(&self, items: &[PendingItem]) -> Result<Vec<CreatedItem>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        if items.len() > BATCH_LIMIT {
            return Err(ApiError::Rejected {
                status: 400,
                message: format!("{} itens excedem o limite de {BATCH_LIMIT}", items.len()),
            });
        }

        let payload = serde_json::json!({
            "newMediaItems": items.iter().map(|item| serde_json::json!({
                "description": item.description,
                "simpleMediaItem": {
                    "uploadToken": item.upload_token,
                    "fileName": item.filename,
                }
            })).collect::<Vec<_>>()
        });

        let parsed: BatchCreateResponse = self
            .post_json("/v1/mediaItems:batchCreate", &payload)
            .await?;

        Ok(parsed
            .new_media_item_results
            .into_iter()
            .map(|result| CreatedItem {
                upload_token: result.upload_token,
                remote_id: result.media_item.map(|item| item.id),
                error: result.status.and_then(|status| {
                    // `code` 0 é sucesso no formato de status da Google.
                    (status.code.unwrap_or(0) != 0).then_some(status.message.unwrap_or_default())
                }),
            })
            .collect())
    }

    /// Cria um álbum e devolve seu identificador remoto.
    pub async fn create_album(&self, title: &str) -> Result<String> {
        let payload = serde_json::json!({ "album": { "title": title } });
        let album: Album = self.post_json("/v1/albums", &payload).await?;
        Ok(album.id)
    }

    /// Adiciona até 50 itens a um álbum criado por este aplicativo.
    ///
    /// Não é possível adicionar a álbuns preexistentes do usuário: a API só governa o que o
    /// próprio aplicativo criou.
    pub async fn add_to_album(&self, album_id: &str, media_ids: &[String]) -> Result<()> {
        if media_ids.is_empty() {
            return Ok(());
        }
        if media_ids.len() > BATCH_LIMIT {
            return Err(ApiError::Rejected {
                status: 400,
                message: format!(
                    "{} itens excedem o limite de {BATCH_LIMIT}",
                    media_ids.len()
                ),
            });
        }
        let payload = serde_json::json!({ "mediaItemIds": media_ids });
        let _: serde_json::Value = self
            .post_json(
                &format!("/v1/albums/{album_id}:batchAddMediaItems"),
                &payload,
            )
            .await?;
        Ok(())
    }

    /// Relê um item que este aplicativo enviou.
    ///
    /// Exige o escopo `photoslibrary.readonly.appcreateddata`. É o que transforma "enviei" em
    /// "confirmei que chegou".
    pub async fn verify_item(&self, remote_id: &str) -> Result<bool> {
        let response = self
            .http
            .get(format!("{}/v1/mediaItems/{remote_id}", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;

        match response.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(classify(status, body))
            }
        }
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        payload: &serde_json::Value,
    ) -> Result<T> {
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.access_token)
            .json(payload)
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;

        if !(200..300).contains(&status) {
            return Err(classify(status, body));
        }
        serde_json::from_str(&body).map_err(|error| ApiError::Malformed(error.to_string()))
    }
}

/// Traduz um código HTTP em erro do domínio.
fn classify(status: u16, body: String) -> ApiError {
    match status {
        429 => ApiError::QuotaExceeded,
        401 | 403 => ApiError::Unauthorized(status),
        _ => ApiError::Rejected {
            status,
            message: extract_message(&body).unwrap_or(body),
        },
    }
}

/// Extrai a mensagem do envelope de erro da Google, quando presente.
fn extract_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")?
        .get("message")?
        .as_str()
        .map(str::to_owned)
}

/// Remove de um valor de cabeçalho o que não pode trafegar nele.
///
/// Nome de arquivo com acento ou quebra de linha derruba a requisição — e acento em nome de
/// arquivo é a regra, não a exceção, num acervo em português.
fn sanitize_header(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim().is_empty() {
        "arquivo".to_owned()
    } else {
        cleaned
    }
}

#[derive(Deserialize)]
struct BatchCreateResponse {
    #[serde(default, rename = "newMediaItemResults")]
    new_media_item_results: Vec<NewMediaItemResult>,
}

#[derive(Deserialize)]
struct NewMediaItemResult {
    #[serde(default, rename = "uploadToken")]
    upload_token: String,
    #[serde(default, rename = "mediaItem")]
    media_item: Option<MediaItemRef>,
    #[serde(default)]
    status: Option<StatusRef>,
}

#[derive(Deserialize)]
struct MediaItemRef {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct StatusRef {
    #[serde(default)]
    code: Option<i32>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct Album {
    #[serde(default)]
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_and_server_errors_are_retryable() {
        assert!(ApiError::QuotaExceeded.is_retryable());
        assert!(ApiError::Transport("timeout".into()).is_retryable());
        assert!(ApiError::Rejected {
            status: 503,
            message: String::new()
        }
        .is_retryable());
    }

    #[test]
    fn auth_and_bad_requests_are_not_retryable() {
        // Repetir uma credencial recusada só gasta cota.
        assert!(!ApiError::Unauthorized(401).is_retryable());
        assert!(!ApiError::Rejected {
            status: 400,
            message: String::new()
        }
        .is_retryable());
        assert!(!ApiError::Malformed("json".into()).is_retryable());
    }

    #[test]
    fn classifies_status_codes() {
        assert!(matches!(
            classify(429, String::new()),
            ApiError::QuotaExceeded
        ));
        assert!(matches!(
            classify(401, String::new()),
            ApiError::Unauthorized(401)
        ));
        assert!(matches!(
            classify(403, String::new()),
            ApiError::Unauthorized(403)
        ));
    }

    #[test]
    fn extracts_the_google_error_message() {
        let body = r#"{"error":{"code":400,"message":"Request contains an invalid argument."}}"#;
        match classify(400, body.to_owned()) {
            ApiError::Rejected { message, .. } => {
                assert_eq!(message, "Request contains an invalid argument.");
            }
            other => panic!("esperava recusa, veio {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_the_raw_body_when_not_json() {
        match classify(500, "erro interno".to_owned()) {
            ApiError::Rejected { message, .. } => assert_eq!(message, "erro interno"),
            other => panic!("esperava recusa, veio {other:?}"),
        }
    }

    #[test]
    fn header_sanitisation_keeps_names_usable() {
        // Acento em nome de arquivo é a regra num acervo em português.
        assert_eq!(sanitize_header("Aniversário.jpg"), "Anivers_rio.jpg");
        assert_eq!(sanitize_header("IMG_1002.JPG"), "IMG_1002.JPG");
        assert_eq!(sanitize_header("quebra\nlinha.jpg"), "quebra_linha.jpg");
        assert_eq!(sanitize_header("   "), "arquivo");
        assert_eq!(sanitize_header(""), "arquivo");
    }

    #[test]
    fn base_url_trailing_slash_is_normalised() {
        let client = PhotosClient::with_base_url("token", "http://exemplo/");
        assert_eq!(client.base_url, "http://exemplo");
    }
}
