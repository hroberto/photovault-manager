//! Importação de ponta a ponta contra uma árvore de Takeout sintética.
//!
//! Os fixtures reproduzem o que archives reais contêm: sufixos de sidecar truncados de várias
//! formas, o marcador de duplicata deslocado, a mesma foto repetida em álbuns, Live Photo
//! partida em dois arquivos, versão editada, nomes com acento e um sidecar sem dono.

use std::fs;
use std::path::Path;

use photovault_cas::ObjectStore;
use photovault_catalog::Catalog;

#[path = "../src/import.rs"]
mod import;

/// Monta a árvore de fixtures e devolve o diretório temporário.
fn build_takeout(root: &Path) {
    let timeline = root.join("Photos from 2019");
    let album = root.join("Viagem Japão");
    let album2 = root.join("Família");
    for dir in [&timeline, &album, &album2] {
        fs::create_dir_all(dir).expect("cria diretório");
    }

    // Linha do tempo: uma foto com metadados completos.
    write(&timeline.join("IMG_1002.JPG"), b"bytes da foto 1002");
    write(
        &timeline.join("IMG_1002.JPG.supplemental-metadata.json"),
        r#"{
          "title": "IMG_1002.JPG",
          "description": "Templo em Kyoto",
          "photoTakenTime": { "timestamp": "1571394000" },
          "creationTime": { "timestamp": "1571401200" },
          "geoData": { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
          "geoDataExif": { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
          "people": [ { "name": "Henrique" }, { "name": "Ana" } ],
          "favorited": true,
          "url": "https://photos.google.com/photo/AF1Qip"
        }"#,
    );

    // Sufixo truncado em ponto diferente, no mesmo archive.
    write(&timeline.join("IMG_1003.HEIC"), b"bytes da foto 1003");
    write(
        &timeline.join("IMG_1003.HEIC.supple.json"),
        r#"{ "title": "IMG_1003.HEIC", "photoTakenTime": { "timestamp": "1571480400" } }"#,
    );

    // Marcador de duplicata deslocado para depois da extensão.
    write(
        &timeline.join("IMG_1002(1).JPG"),
        b"bytes diferentes da copia",
    );
    write(
        &timeline.join("IMG_1002.JPG(1).supplemental-metadata.json"),
        r#"{ "title": "IMG_1002.JPG", "description": "mesma cena, outro arquivo" }"#,
    );

    // Live Photo: um item lógico, dois arquivos, cada um com seu sidecar.
    write(&timeline.join("IMG_1004.HEIC"), b"bytes da live photo");
    write(
        &timeline.join("IMG_1004.HEIC.supplemental-metadata.json"),
        r#"{ "title": "IMG_1004.HEIC" }"#,
    );
    write(&timeline.join("IMG_1004.MP4"), b"bytes do movimento");
    write(
        &timeline.join("IMG_1004.MP4.supplemental-metadata.json"),
        r#"{ "title": "IMG_1004.MP4" }"#,
    );

    // Versão editada, sem sidecar próprio — normal, não é erro.
    write(&timeline.join("IMG_1003-edited.HEIC"), b"bytes editados");

    // Nome com acento, sidecar em forma de normalização diferente.
    write(
        &timeline.join("Anivers\u{e1}rio.jpg"),
        b"bytes do aniversario",
    );
    write(
        &timeline.join("Aniversa\u{301}rio.jpg.supplemental-metadata.json"),
        r#"{ "title": "Aniversário.jpg", "people": [ { "name": "ana  maria" } ] }"#,
    );

    // Sidecar sem dono: precisa sobrar no relatório, nunca ser descartado.
    write(
        &timeline.join("IMG_9999.JPG.supplemental-metadata.json"),
        r#"{ "title": "IMG_9999.JPG" }"#,
    );

    // Coordenada zerada no geoData, válida no geoDataExif.
    write(&timeline.join("IMG_1005.JPG"), b"bytes com gps da camera");
    write(
        &timeline.join("IMG_1005.JPG.supplemental-metadata.json"),
        r#"{
          "title": "IMG_1005.JPG",
          "geoData": { "latitude": 0.0, "longitude": 0.0, "altitude": 0.0 },
          "geoDataExif": { "latitude": -30.0346, "longitude": -51.2177, "altitude": 10.0 }
        }"#,
    );

    // Álbum: metadata.json é o que distingue álbum de linha do tempo.
    write(
        &album.join("metadata.json"),
        r#"{ "title": "Viagem Japão", "description": "outubro de 2019" }"#,
    );
    // Os MESMOS bytes da foto da linha do tempo: um item, duas associações.
    write(&album.join("IMG_1002.JPG"), b"bytes da foto 1002");
    write(
        &album.join("IMG_1002.JPG.supplemental-metadata.json"),
        r#"{ "title": "IMG_1002.JPG", "description": "Templo em Kyoto" }"#,
    );

    write(&album2.join("metadata.json"), r#"{ "title": "Família" }"#);
    write(&album2.join("IMG_1002.JPG"), b"bytes da foto 1002");
    write(
        &album2.join("IMG_1002.JPG.json"),
        r#"{ "title": "IMG_1002.JPG" }"#,
    );

    // Arquivo que não é mídia: precisa ser ignorado sem virar falha.
    write(&root.join("archive_browser.html"), b"<html></html>");
}

fn write(path: &Path, content: impl AsRef<[u8]>) {
    fs::write(path, content).expect("escreve fixture");
}

struct Harness {
    _dir: tempfile::TempDir,
    catalog: Catalog,
    outcome: import::ImportOutcome,
}

async fn run() -> Harness {
    let dir = tempfile::tempdir().expect("diretório temporário");
    let takeout = dir.path().join("Takeout");
    fs::create_dir_all(&takeout).expect("cria raiz");
    build_takeout(&takeout);

    let store = ObjectStore::open(dir.path().join("vault/repository")).expect("abre CAS");
    let catalog = Catalog::open_in_memory().await.expect("abre catálogo");

    let outcome = import::import_takeout(&takeout, &store, &catalog, Some("fixture"))
        .await
        .expect("importa");

    Harness {
        _dir: dir,
        catalog,
        outcome,
    }
}

#[tokio::test]
async fn imports_without_failures() {
    let h = run().await;
    assert!(
        h.outcome.failures.is_empty(),
        "importação não deveria falhar: {:?}",
        h.outcome.failures
    );
    assert_eq!(h.outcome.tally.failures, 0);
}

#[tokio::test]
async fn same_bytes_in_three_places_become_one_item() {
    let h = run().await;
    // IMG_1002.JPG aparece na linha do tempo e em dois álbuns, sempre com os mesmos bytes.
    assert_eq!(h.outcome.tally.media_seen, 10, "arquivos de mídia vistos");
    assert_eq!(
        h.outcome.tally.media_deduplicated, 2,
        "duas repetições do 1002"
    );

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE filename = 'IMG_1002.JPG'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("conta");
    assert_eq!(count, 1, "um item lógico, não três");
}

#[tokio::test]
async fn the_shared_photo_belongs_to_both_albums() {
    let h = run().await;
    let albums: Vec<String> = sqlx::query_scalar(
        "SELECT album.title FROM album
         JOIN album_media ON album_media.album_id = album.id
         JOIN media ON media.id = album_media.media_id
         WHERE media.filename = 'IMG_1002.JPG'
         ORDER BY album.title",
    )
    .fetch_all(h.catalog.pool())
    .await
    .expect("consulta álbuns");

    assert_eq!(albums, vec!["Família", "Viagem Japão"]);
}

#[tokio::test]
async fn timeline_folder_is_not_an_album() {
    let h = run().await;
    let stats = h.catalog.stats().await.expect("estatísticas");
    // "Photos from 2019" não tem metadata.json, logo não é álbum.
    assert_eq!(stats.albums, 2);
}

#[tokio::test]
async fn album_description_is_preserved() {
    let h = run().await;
    let description: Option<String> =
        sqlx::query_scalar("SELECT description FROM album WHERE title = 'Viagem Japão'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("consulta");
    assert_eq!(description.as_deref(), Some("outubro de 2019"));
}

#[tokio::test]
async fn capture_time_and_description_come_from_the_sidecar() {
    let h = run().await;
    let row: (Option<i64>, Option<String>, i64) = sqlx::query_as(
        "SELECT captured_at, description, favorited FROM media WHERE filename = 'IMG_1002.JPG'",
    )
    .fetch_one(h.catalog.pool())
    .await
    .expect("consulta");

    assert_eq!(row.0, Some(1_571_394_000), "data de captura, não de upload");
    assert_eq!(row.1.as_deref(), Some("Templo em Kyoto"));
    assert_eq!(row.2, 1, "favorito preservado");
}

#[tokio::test]
async fn geolocation_is_catalogued() {
    let h = run().await;
    let row: (f64, f64, Option<f64>, String) = sqlx::query_as(
        "SELECT lat, lon, altitude, media_place.source FROM media_place
         JOIN media ON media.id = media_place.media_id
         WHERE media.filename = 'IMG_1002.JPG'",
    )
    .fetch_one(h.catalog.pool())
    .await
    .expect("consulta");

    assert!((row.0 - 35.0116).abs() < 1e-6);
    assert!((row.1 - 135.7681).abs() < 1e-6);
    assert_eq!(row.2, Some(52.0));
    assert_eq!(row.3, "geo_data");
}

#[tokio::test]
async fn zeroed_geodata_falls_back_to_the_camera_coordinate() {
    let h = run().await;
    let row: (f64, f64, String) = sqlx::query_as(
        "SELECT lat, lon, media_place.source FROM media_place
         JOIN media ON media.id = media_place.media_id
         WHERE media.filename = 'IMG_1005.JPG'",
    )
    .fetch_one(h.catalog.pool())
    .await
    .expect("consulta");

    // 0,0 é ausência, não a ilha Null no Golfo da Guiné.
    assert!((row.0 + 30.0346).abs() < 1e-6, "latitude do hemisfério sul");
    assert!(
        (row.1 + 51.2177).abs() < 1e-6,
        "longitude do hemisfério oeste"
    );
    assert_eq!(row.2, "geo_data_exif");
}

#[tokio::test]
async fn people_are_catalogued_and_deduplicated() {
    let h = run().await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM person ORDER BY name_key")
        .fetch_all(h.catalog.pool())
        .await
        .expect("consulta");

    // "Henrique", "Ana" do 1002 e "ana  maria" do aniversário — três pessoas distintas.
    assert_eq!(names.len(), 3, "pessoas encontradas: {names:?}");
    assert!(names.contains(&"Henrique".to_owned()));
}

#[tokio::test]
async fn truncated_sidecar_suffix_is_matched() {
    let h = run().await;
    let captured: Option<i64> =
        sqlx::query_scalar("SELECT captured_at FROM media WHERE filename = 'IMG_1003.HEIC'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("consulta");
    // O sidecar era IMG_1003.HEIC.supple.json.
    assert_eq!(captured, Some(1_571_480_400));
}

#[tokio::test]
async fn displaced_duplicate_marker_is_matched() {
    let h = run().await;
    let description: Option<String> =
        sqlx::query_scalar("SELECT description FROM media WHERE filename = 'IMG_1002(1).JPG'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("consulta");
    assert_eq!(description.as_deref(), Some("mesma cena, outro arquivo"));
}

#[tokio::test]
async fn accented_name_matches_across_normalization_forms() {
    let h = run().await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM person_tag
         JOIN media ON media.id = person_tag.media_id
         WHERE media.filename LIKE 'Anivers%'",
    )
    .fetch_one(h.catalog.pool())
    .await
    .expect("consulta");
    assert_eq!(count, 1, "o sidecar em NFD casou com o arquivo em NFC");
}

#[tokio::test]
async fn orphan_sidecar_survives_for_review() {
    let h = run().await;
    assert_eq!(h.outcome.tally.sidecars_orphan, 1);

    // O relatório da importação leva o órfão ao usuário na hora, com o motivo.
    let note = &h.outcome.orphans[0];
    assert_eq!(note.directory, "Photos from 2019");
    assert_eq!(note.sidecar, "IMG_9999.JPG.supplemental-metadata.json");
    assert!(
        note.reason.contains("nenhum arquivo de mídia"),
        "o motivo precisa ser explícito, veio: {}",
        note.reason
    );

    // E fica persistido na fila, para não depender de alguém ter lido o terminal.
    let orphans = h.catalog.pending_orphans().await.expect("consulta órfãos");
    assert_eq!(orphans.len(), 1);
    assert_eq!(
        orphans[0].sidecar,
        "IMG_9999.JPG.supplemental-metadata.json"
    );
}

#[tokio::test]
async fn edited_version_is_imported_without_a_sidecar() {
    let h = run().await;
    assert!(h.outcome.tally.media_without_sidecar >= 1);

    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE filename = 'IMG_1003-edited.HEIC'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("consulta");
    // Perder os bytes seria pior do que perder o metadado.
    assert_eq!(exists, 1);
}

#[tokio::test]
async fn live_photo_components_are_both_stored() {
    let h = run().await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE filename LIKE 'IMG_1004.%'")
            .fetch_one(h.catalog.pool())
            .await
            .expect("consulta");
    assert_eq!(count, 2, "HEIC e MP4 preservados");
}

#[tokio::test]
async fn non_media_files_are_ignored_silently() {
    let h = run().await;
    let html: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE filename LIKE '%.html'")
        .fetch_one(h.catalog.pool())
        .await
        .expect("consulta");
    assert_eq!(html, 0);
    assert!(h.outcome.failures.is_empty());
}

#[tokio::test]
async fn audit_chain_is_intact_after_import() {
    let h = run().await;
    let entries = h
        .catalog
        .verify_audit_chain()
        .await
        .expect("cadeia íntegra");
    assert!(entries >= 2, "início e fim da importação registrados");
}

#[tokio::test]
async fn reimporting_the_same_archive_changes_nothing() {
    let dir = tempfile::tempdir().expect("diretório");
    let takeout = dir.path().join("Takeout");
    fs::create_dir_all(&takeout).expect("cria raiz");
    build_takeout(&takeout);

    let store = ObjectStore::open(dir.path().join("vault/repository")).expect("abre CAS");
    let catalog = Catalog::open_in_memory().await.expect("abre catálogo");

    import::import_takeout(&takeout, &store, &catalog, Some("primeira"))
        .await
        .expect("primeira importação");
    let first = catalog.stats().await.expect("estatísticas");

    import::import_takeout(&takeout, &store, &catalog, Some("segunda"))
        .await
        .expect("segunda importação");
    let second = catalog.stats().await.expect("estatísticas");

    // Reimportar um archive é operação segura: o usuário vai fazer isso.
    assert_eq!(first, second);
}
