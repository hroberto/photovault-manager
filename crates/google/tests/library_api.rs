//! Cliente da Library API contra um servidor HTTP simulado.
//!
//! Estes testes exercitam requisições e respostas reais — cabeçalhos, corpo, códigos de status —
//! sem depender de credencial do Google. Quando a credencial existir, o que muda é o endereço.

use photovault_google::client::PhotosClient;
use photovault_google::{ApiError, PendingItem, BATCH_LIMIT};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn client(server: &MockServer) -> PhotosClient {
    PhotosClient::with_base_url("ya29.TOKEN", server.uri())
}

fn item(token: &str, filename: &str) -> PendingItem {
    PendingItem {
        upload_token: token.into(),
        filename: filename.into(),
        description: None,
    }
}

#[tokio::test]
async fn upload_sends_raw_bytes_with_the_required_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .and(header("authorization", "Bearer ya29.TOKEN"))
        .and(header("x-goog-upload-protocol", "raw"))
        .and(header("x-goog-file-name", "IMG_1002.JPG"))
        .respond_with(ResponseTemplate::new(200).set_body_string("UPLOAD_TOKEN_123"))
        .expect(1)
        .mount(&server)
        .await;

    let token = client(&server)
        .await
        .upload_bytes("IMG_1002.JPG", b"bytes da foto".to_vec())
        .await
        .expect("envia");

    assert_eq!(token, "UPLOAD_TOKEN_123");
}

#[tokio::test]
async fn upload_sanitises_accented_filenames_in_the_header() {
    // Acento em nome de arquivo é a regra num acervo em português, e cabeçalho HTTP não aceita.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .and(header("x-goog-file-name", "Anivers_rio.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_string("TOKEN"))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .await
        .upload_bytes("Aniversário.jpg", b"bytes".to_vec())
        .await
        .expect("envia mesmo com acento");
}

#[tokio::test]
async fn empty_upload_token_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_string("   "))
        .mount(&server)
        .await;

    match client(&server).await.upload_bytes("a.jpg", vec![]).await {
        Err(ApiError::Malformed(_)) => {}
        other => panic!("token vazio precisa ser recusado, veio {other:?}"),
    }
}

#[tokio::test]
async fn batch_create_builds_the_expected_payload() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "newMediaItems": [{
            "description": "Templo em Kyoto",
            "simpleMediaItem": { "uploadToken": "T1", "fileName": "IMG_1002.JPG" }
        }]
    });

    Mock::given(method("POST"))
        .and(path("/v1/mediaItems:batchCreate"))
        .and(body_json(&expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "newMediaItemResults": [{
                "uploadToken": "T1",
                "status": { "message": "Success" },
                "mediaItem": { "id": "REMOTE_1" }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let results = client(&server)
        .await
        .batch_create(&[PendingItem {
            upload_token: "T1".into(),
            filename: "IMG_1002.JPG".into(),
            description: Some("Templo em Kyoto".into()),
        }])
        .await
        .expect("cria");

    assert_eq!(results.len(), 1);
    assert!(results[0].succeeded());
    assert_eq!(results[0].remote_id.as_deref(), Some("REMOTE_1"));
}

#[tokio::test]
async fn partial_failure_is_reported_per_item() {
    // A API responde item a item: alguns passam, outros não. A retomada precisa saber quais.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/mediaItems:batchCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "newMediaItemResults": [
                { "uploadToken": "T1", "status": { "message": "Success" },
                  "mediaItem": { "id": "REMOTE_1" } },
                { "uploadToken": "T2", "status": { "code": 3, "message": "Invalid media." } }
            ]
        })))
        .mount(&server)
        .await;

    let results = client(&server)
        .await
        .batch_create(&[item("T1", "a.jpg"), item("T2", "b.jpg")])
        .await
        .expect("responde");

    assert!(results[0].succeeded());
    assert!(!results[1].succeeded());
    assert_eq!(results[1].error.as_deref(), Some("Invalid media."));
    assert_eq!(
        results[1].upload_token, "T2",
        "o token identifica o que refazer"
    );
}

#[tokio::test]
async fn batch_over_the_limit_is_refused_before_spending_a_request() {
    let server = MockServer::start().await;
    // Nenhuma expectativa montada: se o cliente chamar o servidor, o teste falha.
    let items: Vec<PendingItem> = (0..=BATCH_LIMIT)
        .map(|i| item(&format!("T{i}"), "a.jpg"))
        .collect();

    match client(&server).await.batch_create(&items).await {
        Err(ApiError::Rejected {
            status: 400,
            message,
        }) => {
            assert!(
                message.contains("51"),
                "a mensagem diz quantos vieram: {message}"
            );
        }
        other => panic!("esperava recusa local, veio {other:?}"),
    }
}

#[tokio::test]
async fn empty_batch_does_not_touch_the_network() {
    let server = MockServer::start().await;
    let results = client(&server)
        .await
        .batch_create(&[])
        .await
        .expect("aceita vazio");
    assert!(results.is_empty());
}

#[tokio::test]
async fn creates_an_album() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/albums"))
        .and(body_json(
            serde_json::json!({ "album": { "title": "Viagem Japão" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "ALBUM_1", "title": "Viagem Japão"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let id = client(&server)
        .await
        .create_album("Viagem Japão")
        .await
        .expect("cria álbum");
    assert_eq!(id, "ALBUM_1");
}

#[tokio::test]
async fn adds_items_to_an_album() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/albums/ALBUM_1:batchAddMediaItems"))
        .and(body_json(
            serde_json::json!({ "mediaItemIds": ["R1", "R2"] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .await
        .add_to_album("ALBUM_1", &["R1".into(), "R2".into()])
        .await
        .expect("associa");
}

#[tokio::test]
async fn album_addition_respects_the_batch_limit() {
    let server = MockServer::start().await;
    let ids: Vec<String> = (0..=BATCH_LIMIT).map(|i| format!("R{i}")).collect();
    assert!(client(&server).await.add_to_album("A", &ids).await.is_err());
}

#[tokio::test]
async fn verifies_an_item_that_exists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/mediaItems/REMOTE_1"))
        .and(header("authorization", "Bearer ya29.TOKEN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "REMOTE_1", "filename": "IMG_1002.JPG"
        })))
        .mount(&server)
        .await;

    assert!(client(&server)
        .await
        .verify_item("REMOTE_1")
        .await
        .expect("verifica"));
}

#[tokio::test]
async fn verification_reports_a_missing_item_as_absent_not_as_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/mediaItems/SUMIU"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    assert!(!client(&server)
        .await
        .verify_item("SUMIU")
        .await
        .expect("responde"));
}

#[tokio::test]
async fn quota_exhaustion_is_a_distinct_retryable_error() {
    // Numa restauração de seis dias isto não é acidente: é o curso normal.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Quota exceeded"))
        .mount(&server)
        .await;

    match client(&server).await.upload_bytes("a.jpg", vec![1]).await {
        Err(error @ ApiError::QuotaExceeded) => assert!(error.is_retryable()),
        other => panic!("esperava cota esgotada, veio {other:?}"),
    }
}

#[tokio::test]
async fn expired_credentials_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/mediaItems:batchCreate"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "code": 401, "message": "Invalid Credentials" }
        })))
        .mount(&server)
        .await;

    match client(&server)
        .await
        .batch_create(&[item("T1", "a.jpg")])
        .await
    {
        Err(error @ ApiError::Unauthorized(401)) => {
            // Repetir uma credencial recusada só gasta cota.
            assert!(!error.is_retryable());
        }
        other => panic!("esperava recusa de credencial, veio {other:?}"),
    }
}

#[tokio::test]
async fn server_errors_are_retryable_and_carry_the_google_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/albums"))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "error": { "code": 503, "message": "The service is currently unavailable." }
        })))
        .mount(&server)
        .await;

    match client(&server).await.create_album("Teste").await {
        Err(error) => {
            assert!(error.is_retryable());
            assert!(error.to_string().contains("currently unavailable"));
        }
        Ok(_) => panic!("deveria falhar"),
    }
}

#[tokio::test]
async fn malformed_response_does_not_look_like_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/albums"))
        .respond_with(ResponseTemplate::new(200).set_body_string("isto não é json"))
        .mount(&server)
        .await;

    match client(&server).await.create_album("Teste").await {
        Err(ApiError::Malformed(_)) => {}
        other => panic!("esperava resposta inesperada, veio {other:?}"),
    }
}
