//! Testes do catálogo.
//!
//! Rodam contra um banco em memória com as migrações reais aplicadas — o mesmo caminho de
//! código que o cofre de verdade usa.

use photovault_catalog::{Catalog, ImportTally, NewMedia};
use photovault_core::{Fidelity, MediaKind, ObjectHash, SourceKind};

async fn catalog() -> Catalog {
    Catalog::open_in_memory()
        .await
        .expect("abre catálogo em memória")
}

fn hash(seed: u8) -> ObjectHash {
    ObjectHash::from_bytes([seed; 32])
}

fn media(object: ObjectHash, name: &str, run: photovault_catalog::ImportRunId) -> NewMedia {
    NewMedia {
        object,
        filename: name.to_owned(),
        kind: MediaKind::Photo,
        source: SourceKind::Takeout,
        captured_at: Some(1_571_394_000),
        uploaded_at: Some(1_571_401_200),
        description: None,
        favorited: false,
        google_url: None,
        import_run: run,
    }
}

#[tokio::test]
async fn migrations_apply_on_a_fresh_database() {
    let catalog = catalog().await;
    let stats = catalog.stats().await.expect("consulta estatísticas");
    assert_eq!(stats.media, 0);
    assert_eq!(stats.objects, 0);
    assert_eq!(stats.bytes, 0);
}

#[tokio::test]
async fn stores_an_item_and_counts_it() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", Some("teste"))
        .await
        .expect("abre importação");

    catalog
        .upsert_object(&hash(1), 2048, Fidelity::Original)
        .await
        .expect("registra objeto");
    let (_, is_new) = catalog
        .upsert_media(&media(hash(1), "IMG_1002.JPG", run))
        .await
        .expect("cataloga item");
    assert!(is_new);

    let stats = catalog.stats().await.expect("estatísticas");
    assert_eq!(stats.media, 1);
    assert_eq!(stats.objects, 1);
    assert_eq!(stats.bytes, 2048);
}

#[tokio::test]
async fn same_bytes_seen_again_reuse_the_item() {
    // A mesma foto aparece em "Photos from 2019" e em cada álbum. São quatro caminhos e um
    // item, não quatro itens.
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");
    catalog
        .upsert_object(&hash(7), 1024, Fidelity::Original)
        .await
        .expect("registra objeto");

    let (first, first_is_new) = catalog
        .upsert_media(&media(hash(7), "IMG_1002.JPG", run))
        .await
        .expect("cataloga");
    let (second, second_is_new) = catalog
        .upsert_media(&media(hash(7), "IMG_1002.JPG", run))
        .await
        .expect("cataloga de novo");

    assert_eq!(first, second);
    assert!(first_is_new);
    assert!(!second_is_new);
    assert_eq!(catalog.stats().await.expect("estatísticas").media, 1);
}

#[tokio::test]
async fn one_photo_in_four_albums_is_one_item() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");
    catalog
        .upsert_object(&hash(3), 4096, Fidelity::Original)
        .await
        .expect("registra objeto");
    let (item, _) = catalog
        .upsert_media(&media(hash(3), "IMG_1002.JPG", run))
        .await
        .expect("cataloga");

    for title in ["Viagem Japão", "Família", "Favoritas", "2019"] {
        let album = catalog
            .upsert_album(title, &title.to_lowercase(), None)
            .await
            .expect("cria álbum");
        catalog
            .add_to_album(album, item, None)
            .await
            .expect("associa");
    }

    let stats = catalog.stats().await.expect("estatísticas");
    assert_eq!(stats.media, 1, "fisicamente um item");
    assert_eq!(stats.albums, 4, "logicamente quatro associações");
}

#[tokio::test]
async fn album_membership_is_idempotent() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");
    catalog
        .upsert_object(&hash(4), 100, Fidelity::Original)
        .await
        .expect("registra objeto");
    let (item, _) = catalog
        .upsert_media(&media(hash(4), "a.jpg", run))
        .await
        .expect("cataloga");
    let album = catalog
        .upsert_album("Álbum", "álbum", None)
        .await
        .expect("cria álbum");

    // Reimportar o mesmo archive não pode multiplicar associações.
    catalog
        .add_to_album(album, item, None)
        .await
        .expect("associa");
    catalog
        .add_to_album(album, item, None)
        .await
        .expect("associa de novo");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM album_media")
        .fetch_one(catalog.pool())
        .await
        .expect("conta associações");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn people_are_deduplicated_by_key() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");
    catalog
        .upsert_object(&hash(5), 100, Fidelity::Original)
        .await
        .expect("registra objeto");
    let (item, _) = catalog
        .upsert_media(&media(hash(5), "a.jpg", run))
        .await
        .expect("cataloga");

    // "Ana Maria" e "ana  maria" são a mesma pessoa.
    let first = catalog
        .upsert_person("Ana Maria", "ana maria")
        .await
        .expect("cria pessoa");
    let second = catalog
        .upsert_person("ana  maria", "ana maria")
        .await
        .expect("reencontra");
    assert_eq!(first, second);

    catalog
        .tag_person(item, first, "takeout")
        .await
        .expect("marca");
    catalog
        .tag_person(item, second, "takeout")
        .await
        .expect("marca de novo");

    assert_eq!(catalog.stats().await.expect("estatísticas").people, 1);
    let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM person_tag")
        .fetch_one(catalog.pool())
        .await
        .expect("conta marcações");
    assert_eq!(tags, 1);
}

#[tokio::test]
async fn location_keeps_both_coordinates_when_they_diverge() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");
    catalog
        .upsert_object(&hash(6), 100, Fidelity::Original)
        .await
        .expect("registra objeto");
    let (item, _) = catalog
        .upsert_media(&media(hash(6), "a.jpg", run))
        .await
        .expect("cataloga");

    catalog
        .set_place(
            item,
            35.0,
            135.0,
            Some(52.0),
            "geo_data",
            Some((36.0, 136.0)),
        )
        .await
        .expect("grava lugar");

    let row: (f64, f64, Option<f64>, Option<f64>) =
        sqlx::query_as("SELECT lat, lon, lat_exif, lon_exif FROM media_place WHERE media_id = ?")
            .bind(item.get())
            .fetch_one(catalog.pool())
            .await
            .expect("lê lugar");

    assert!((row.0 - 35.0).abs() < 1e-9, "vale a coordenada efetiva");
    assert_eq!(
        row.2,
        Some(36.0),
        "a da câmera fica guardada para auditoria"
    );
    assert_eq!(catalog.stats().await.expect("estatísticas").located, 1);
}

#[tokio::test]
async fn orphan_sidecars_are_persisted_for_review() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");

    catalog
        .record_orphan(
            run,
            "Photos from 2019",
            "PXL_2024_.jpg.supplemental-metadata.json",
            "mais de um candidato possível",
            &["PXL_2024_alpha.jpg".into(), "PXL_2024_beta.jpg".into()],
        )
        .await
        .expect("registra órfão");

    let stats = catalog.stats().await.expect("estatísticas");
    assert_eq!(stats.pending_orphans, 1);

    let candidates: Option<String> =
        sqlx::query_scalar("SELECT candidates FROM sidecar_orphan LIMIT 1")
            .fetch_one(catalog.pool())
            .await
            .expect("lê candidatos");
    let candidates = candidates.expect("guardou os candidatos");
    assert!(candidates.contains("PXL_2024_alpha.jpg"));
    assert!(candidates.contains("PXL_2024_beta.jpg"));
}

#[tokio::test]
async fn import_run_records_its_tally() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", Some("archive 1 de 8"))
        .await
        .expect("abre");

    let tally = ImportTally {
        media_seen: 48_231,
        media_imported: 48_100,
        media_deduplicated: 131,
        sidecars_matched: 48_198,
        sidecars_orphan: 12,
        media_without_sidecar: 33,
        bytes_stored: 390_000_000_000,
        failures: 0,
    };
    catalog
        .finish_import(run, &tally)
        .await
        .expect("fecha importação");

    let row: (i64, i64, Option<i64>) = sqlx::query_as(
        "SELECT media_seen, sidecars_orphan, finished_at FROM import_run WHERE id = ?",
    )
    .bind(run.get())
    .fetch_one(catalog.pool())
    .await
    .expect("lê importação");

    assert_eq!(row.0, 48_231);
    assert_eq!(row.1, 12);
    assert!(row.2.is_some(), "importação deve ficar fechada");
}

#[tokio::test]
async fn audit_chain_links_and_verifies() {
    let catalog = catalog().await;

    catalog
        .append_audit("import.begin", None, "ok", Some("archive 1"))
        .await
        .expect("registra");
    catalog
        .append_audit("object.store", Some(&hash(1).to_hex()), "ok", None)
        .await
        .expect("registra");
    catalog
        .append_audit("import.finish", None, "ok", Some("48231 itens"))
        .await
        .expect("registra");

    assert_eq!(catalog.verify_audit_chain().await.expect("verifica"), 3);

    // A segunda entrada aponta para o hash da primeira.
    let (prev, entry): (Option<String>, String) =
        sqlx::query_as("SELECT prev_hash, entry_hash FROM audit_log ORDER BY id LIMIT 1")
            .fetch_one(catalog.pool())
            .await
            .expect("lê primeira entrada");
    assert!(prev.is_none(), "a primeira entrada não tem antecessora");

    let second_prev: Option<String> =
        sqlx::query_scalar("SELECT prev_hash FROM audit_log ORDER BY id LIMIT 1 OFFSET 1")
            .fetch_one(catalog.pool())
            .await
            .expect("lê segunda entrada");
    assert_eq!(second_prev.as_deref(), Some(entry.as_str()));
}

#[tokio::test]
async fn tampering_with_an_old_entry_breaks_the_chain() {
    let catalog = catalog().await;
    for index in 0..4 {
        catalog
            .append_audit(
                "cleanup.suggest",
                None,
                "ok",
                Some(&format!("item {index}")),
            )
            .await
            .expect("registra");
    }
    assert!(catalog.verify_audit_chain().await.is_ok());

    // Alguém reescreve a segunda entrada para esconder o que foi sugerido.
    sqlx::query("UPDATE audit_log SET details = 'nada aconteceu' WHERE id = 2")
        .execute(catalog.pool())
        .await
        .expect("adultera");

    // É exatamente isto que o encadeamento existe para pegar.
    match catalog.verify_audit_chain().await {
        Err(photovault_catalog::CatalogError::BrokenAuditChain { id }) => assert_eq!(id, 2),
        other => panic!("a adulteração deveria ser detectada, veio {other:?}"),
    }
}

#[tokio::test]
async fn verifying_an_object_is_recorded() {
    let catalog = catalog().await;
    catalog
        .upsert_object(&hash(9), 100, Fidelity::Original)
        .await
        .expect("registra objeto");
    assert_eq!(catalog.stats().await.expect("estatísticas").verified, 0);

    catalog
        .mark_verified(&hash(9))
        .await
        .expect("marca verificado");
    assert_eq!(catalog.stats().await.expect("estatísticas").verified, 1);
}

#[tokio::test]
async fn object_insert_is_idempotent() {
    let catalog = catalog().await;
    catalog
        .upsert_object(&hash(2), 500, Fidelity::Original)
        .await
        .expect("registra");
    catalog
        .upsert_object(&hash(2), 500, Fidelity::Original)
        .await
        .expect("registra de novo");

    let stats = catalog.stats().await.expect("estatísticas");
    assert_eq!(stats.objects, 1);
    assert_eq!(stats.bytes, 500, "o tamanho não é somado duas vezes");
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let catalog = catalog().await;
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");

    // Catalogar um item cujo objeto não existe precisa falhar, não passar em silêncio.
    let orphan = media(hash(42), "fantasma.jpg", run);
    assert!(
        catalog.upsert_media(&orphan).await.is_err(),
        "item sem objeto no CAS não pode ser catalogado"
    );
}
