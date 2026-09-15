//! Retomada e idempotência, contra um catálogo real.
//!
//! Uma restauração de 52 mil itens leva cerca de seis dias. Ela **vai** ser interrompida — por
//! reinício do computador, queda de rede ou fim de cota. Estes testes provam que retomar não
//! reenvia nada, que é a única proteção contra duplicar itens na biblioteca do usuário: não
//! existe API para listar o que já está lá.

use photovault_catalog::{Catalog, NewMedia};
use photovault_core::{Fidelity, MediaId, MediaKind, ObjectHash, SourceKind};
use photovault_restore::idempotency_key;

const ACCOUNT: &str = "conta@exemplo.com";
const SINK: &str = "google_photos";

fn hash(seed: u8) -> ObjectHash {
    ObjectHash::from_bytes([seed; 32])
}

/// Monta um catálogo com `count` itens catalogados.
async fn catalog_with(count: u8) -> (Catalog, Vec<MediaId>) {
    let catalog = Catalog::open_in_memory().await.expect("abre catálogo");
    let run = catalog
        .begin_import("takeout", None)
        .await
        .expect("abre importação");

    let mut ids = Vec::new();
    for seed in 1..=count {
        catalog
            .upsert_object(&hash(seed), 1_000_000, Fidelity::Original)
            .await
            .expect("registra objeto");
        let (id, _) = catalog
            .upsert_media(&NewMedia {
                object: hash(seed),
                filename: format!("IMG_{seed}.JPG"),
                kind: MediaKind::Photo,
                source: SourceKind::Takeout,
                captured_at: Some(1_571_394_000),
                uploaded_at: None,
                description: Some(format!("foto {seed}")),
                favorited: false,
                google_url: None,
                import_run: run,
            })
            .await
            .expect("cataloga");
        ids.push(id);
    }
    (catalog, ids)
}

#[tokio::test]
async fn enqueues_every_item_once() {
    let (catalog, ids) = catalog_with(5).await;
    let run = catalog
        .begin_restore(SINK, false, 5, 5_000_000, 120)
        .await
        .expect("abre restauração");

    for (index, id) in ids.iter().enumerate() {
        let key = idempotency_key(ACCOUNT, &hash(index as u8 + 1), SINK);
        assert!(
            catalog
                .enqueue_restore_item(run, *id, &key)
                .await
                .expect("enfileira"),
            "o item {index} deveria entrar"
        );
    }

    let pending = catalog.pending_restore_items(run).await.expect("consulta");
    assert_eq!(pending.len(), 5);
    assert!(pending.iter().all(|item| item.is_pending()));
}

#[tokio::test]
async fn the_same_object_is_refused_on_a_second_enqueue() {
    // O usuário pede a restauração duas vezes por engano. A segunda não pode duplicar nada.
    let (catalog, ids) = catalog_with(1).await;
    let run = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre");
    let key = idempotency_key(ACCOUNT, &hash(1), SINK);

    assert!(catalog
        .enqueue_restore_item(run, ids[0], &key)
        .await
        .expect("primeira"));
    assert!(
        !catalog
            .enqueue_restore_item(run, ids[0], &key)
            .await
            .expect("segunda"),
        "a chave de idempotência precisa recusar o reenvio"
    );
}

#[tokio::test]
async fn a_new_run_cannot_resend_what_an_older_run_already_sent() {
    // O índice de idempotência é global, não por execução. Sem isso, pedir a restauração de
    // novo no mês que vem duplicaria a biblioteca inteira.
    let (catalog, ids) = catalog_with(1).await;
    let key = idempotency_key(ACCOUNT, &hash(1), SINK);

    let first = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre");
    catalog
        .enqueue_restore_item(first, ids[0], &key)
        .await
        .expect("enfileira");
    catalog
        .mark_created(first, ids[0], "REMOTE_1")
        .await
        .expect("cria");
    catalog.finish_restore(first, "done").await.expect("fecha");

    let second = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre outra");
    assert!(
        !catalog
            .enqueue_restore_item(second, ids[0], &key)
            .await
            .expect("tenta"),
        "uma execução nova não pode reenviar o que já subiu"
    );
}

#[tokio::test]
async fn migrating_to_another_account_is_allowed() {
    // Migrar de conta é caso de uso legítimo: a chave da conta antiga não pode bloquear a nova.
    let (catalog, ids) = catalog_with(1).await;
    let run = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre");

    let old = idempotency_key("conta-antiga@exemplo.com", &hash(1), SINK);
    let new = idempotency_key("conta-nova@exemplo.com", &hash(1), SINK);

    assert!(catalog
        .enqueue_restore_item(run, ids[0], &old)
        .await
        .expect("conta antiga"));

    let migration = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre migração");
    assert!(
        catalog
            .enqueue_restore_item(migration, ids[0], &new)
            .await
            .expect("conta nova"),
        "o mesmo item precisa poder subir numa conta diferente"
    );
}

#[tokio::test]
async fn resuming_skips_what_already_went_up() {
    let (catalog, ids) = catalog_with(5).await;
    let run = catalog
        .begin_restore(SINK, false, 5, 0, 0)
        .await
        .expect("abre");
    for (index, id) in ids.iter().enumerate() {
        let key = idempotency_key(ACCOUNT, &hash(index as u8 + 1), SINK);
        catalog
            .enqueue_restore_item(run, *id, &key)
            .await
            .expect("enfileira");
    }

    // Três subiram antes de o computador ser desligado.
    for (index, id) in ids.iter().take(3).enumerate() {
        catalog
            .mark_uploaded(run, *id, &format!("UPLOAD_{index}"))
            .await
            .expect("envia bytes");
        catalog
            .mark_created(run, *id, &format!("REMOTE_{index}"))
            .await
            .expect("cria item");
    }

    // Ao reabrir, a fila traz apenas o que falta.
    let pending = catalog.pending_restore_items(run).await.expect("consulta");
    assert_eq!(pending.len(), 2, "só os dois que não subiram");
    assert!(pending.iter().all(|item| item.remote_media_id.is_none()));

    let (total, done, failed) = catalog.restore_progress(run).await.expect("progresso");
    assert_eq!((total, done, failed), (5, 3, 0));
}

#[tokio::test]
async fn an_item_uploaded_but_not_created_is_retried() {
    // Janela real: os bytes subiram e a queda aconteceu antes do batchCreate. O token de envio
    // ainda vale por um tempo, mas o item precisa continuar na fila.
    let (catalog, ids) = catalog_with(1).await;
    let run = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre");
    let key = idempotency_key(ACCOUNT, &hash(1), SINK);
    catalog
        .enqueue_restore_item(run, ids[0], &key)
        .await
        .expect("enfileira");

    catalog
        .mark_uploaded(run, ids[0], "UPLOAD_TOKEN")
        .await
        .expect("envia");

    let pending = catalog.pending_restore_items(run).await.expect("consulta");
    assert_eq!(pending.len(), 1, "sem remote_media_id, continua pendente");
    assert_eq!(pending[0].status, "uploaded");
}

#[tokio::test]
async fn failures_are_counted_and_stay_retriable() {
    let (catalog, ids) = catalog_with(2).await;
    let run = catalog
        .begin_restore(SINK, false, 2, 0, 0)
        .await
        .expect("abre");
    for (index, id) in ids.iter().enumerate() {
        let key = idempotency_key(ACCOUNT, &hash(index as u8 + 1), SINK);
        catalog
            .enqueue_restore_item(run, *id, &key)
            .await
            .expect("enfileira");
    }

    catalog
        .mark_restore_failed(run, ids[0], "cota esgotada")
        .await
        .expect("registra falha");

    let (_, _, failed) = catalog.restore_progress(run).await.expect("progresso");
    assert_eq!(failed, 1);

    // Falhar não tira da fila: o item volta a ser tentado na retomada.
    let pending = catalog.pending_restore_items(run).await.expect("consulta");
    assert_eq!(pending.len(), 2);
    let retried = pending
        .iter()
        .find(|i| i.media_id == ids[0].get())
        .expect("achou");
    assert_eq!(retried.attempts, 1);
    assert_eq!(retried.status, "failed");
}

#[tokio::test]
async fn the_queue_carries_what_the_upload_needs() {
    let (catalog, ids) = catalog_with(1).await;
    let run = catalog
        .begin_restore(SINK, false, 1, 0, 0)
        .await
        .expect("abre");
    let key = idempotency_key(ACCOUNT, &hash(1), SINK);
    catalog
        .enqueue_restore_item(run, ids[0], &key)
        .await
        .expect("enfileira");

    let pending = catalog.pending_restore_items(run).await.expect("consulta");
    let item = &pending[0];
    assert_eq!(item.filename, "IMG_1.JPG");
    assert_eq!(item.description.as_deref(), Some("foto 1"));
    assert_eq!(item.object_hash, hash(1).to_hex());
    assert_eq!(item.idempotency_key, key);
}

#[tokio::test]
async fn a_finished_run_records_its_state() {
    let (catalog, _) = catalog_with(1).await;
    let run = catalog
        .begin_restore(SINK, true, 1, 0, 0)
        .await
        .expect("abre simulação");
    catalog.finish_restore(run, "done").await.expect("fecha");

    let status: String = sqlx::query_scalar("SELECT status FROM restore_run WHERE id = ?")
        .bind(run.get())
        .fetch_one(catalog.pool())
        .await
        .expect("consulta");
    assert_eq!(status, "done");

    let dry_run: i64 = sqlx::query_scalar("SELECT dry_run FROM restore_run WHERE id = ?")
        .bind(run.get())
        .fetch_one(catalog.pool())
        .await
        .expect("consulta");
    assert_eq!(dry_run, 1, "a simulação fica registrada como tal");
}
