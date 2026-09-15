//! Catálogo em SQLite.
//!
//! Guarda o que se sabe sobre cada item: quando foi tirado, onde, quem aparece, em que álbuns
//! está e de onde veio. Os bytes ficam no CAS; aqui fica o significado.
//!
//! Duas decisões que atravessam o módulo:
//!
//! - **Nada é apagado de verdade.** `deleted_at` em vez de `DELETE`, para que a auditoria e a
//!   reconciliação entre importações continuem possíveis.
//! - **O log de auditoria é encadeado por hash.** Sem o encadeamento é registro; com ele é
//!   prova, porque alterar uma linha antiga invalida todas as seguintes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use photovault_core::{Fidelity, MediaId, ObjectHash};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

pub mod records;

pub use records::{
    AlbumId, CatalogStats, ImportRunId, ImportTally, NewMedia, PersonId, RestoreItemRow,
    RestoreRunId,
};

/// Falha ao operar sobre o catálogo.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// Erro vindo do SQLite.
    #[error("banco de dados: {0}")]
    Database(#[from] sqlx::Error),
    /// Falha ao aplicar migrações.
    #[error("migração: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// O log de auditoria não fecha.
    #[error("cadeia de auditoria quebrada na entrada {id}")]
    BrokenAuditChain {
        /// Primeira entrada inconsistente.
        id: i64,
    },
}

type Result<T> = std::result::Result<T, CatalogError>;

/// O catálogo aberto.
#[derive(Debug, Clone)]
pub struct Catalog {
    pool: SqlitePool,
}

impl Catalog {
    /// Abre (ou cria) o catálogo no caminho indicado e aplica as migrações pendentes.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true)
            // WAL dá um escritor e vários leitores; a engine de jobs tem um escritor por
            // desenho. Se for preciso um segundo, o desenho está errado.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));

        Self::from_options(options).await
    }

    /// Abre um catálogo em memória. Para testes.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        Self::from_options(options).await
    }

    async fn from_options(options: SqliteConnectOptions) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// Acesso ao pool, para consultas específicas de outros crates.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // -- Importações ---------------------------------------------------------

    /// Registra o início de uma importação.
    pub async fn begin_import(&self, kind: &str, label: Option<&str>) -> Result<ImportRunId> {
        let id = sqlx::query(
            "INSERT INTO import_run (kind, source_label, started_at) VALUES (?, ?, ?)
             RETURNING id",
        )
        .bind(kind)
        .bind(label)
        .bind(now())
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("id");
        Ok(ImportRunId(id))
    }

    /// Fecha uma importação com os números finais.
    pub async fn finish_import(&self, run: ImportRunId, tally: &ImportTally) -> Result<()> {
        sqlx::query(
            "UPDATE import_run SET
                finished_at = ?, media_seen = ?, media_imported = ?, media_deduplicated = ?,
                sidecars_matched = ?, sidecars_orphan = ?, media_without_sidecar = ?,
                bytes_stored = ?, failures = ?
             WHERE id = ?",
        )
        .bind(now())
        .bind(tally.media_seen as i64)
        .bind(tally.media_imported as i64)
        .bind(tally.media_deduplicated as i64)
        .bind(tally.sidecars_matched as i64)
        .bind(tally.sidecars_orphan as i64)
        .bind(tally.media_without_sidecar as i64)
        .bind(tally.bytes_stored as i64)
        .bind(tally.failures as i64)
        .bind(run.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Objetos e itens -----------------------------------------------------

    /// Registra um objeto guardado no CAS.
    ///
    /// Idempotente: reimportar o mesmo archive não duplica nada.
    pub async fn upsert_object(
        &self,
        hash: &ObjectHash,
        size: u64,
        fidelity: Fidelity,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO object (hash_blake3, size, fidelity, stored_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(hash_blake3) DO NOTHING",
        )
        .bind(hash.to_hex())
        .bind(size as i64)
        .bind(fidelity.as_str())
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Marca um objeto como verificado por releitura.
    pub async fn mark_verified(&self, hash: &ObjectHash) -> Result<()> {
        sqlx::query("UPDATE object SET verified_at = ? WHERE hash_blake3 = ?")
            .bind(now())
            .bind(hash.to_hex())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insere um item, ou devolve o existente quando os mesmos bytes já estão catalogados.
    ///
    /// O segundo elemento diz se o item é novo. Reencontrar o mesmo arquivo em outro álbum
    /// devolve `false` — é assim que uma foto em quatro álbuns vira um item com quatro
    /// associações, e não quatro cópias.
    pub async fn upsert_media(&self, media: &NewMedia) -> Result<(MediaId, bool)> {
        if let Some(existing) = self.media_id_for_object(&media.object).await? {
            // O mesmo arquivo aparece na linha do tempo e em cada álbum, e os sidecars das
            // cópias costumam ser mais pobres que o da linha do tempo. Enriquecemos os campos
            // ainda vazios sem jamais sobrescrever o que já foi aprendido — assim a ordem em
            // que os diretórios são visitados deixa de influenciar o resultado.
            sqlx::query(
                "UPDATE media SET
                    last_seen_at = ?,
                    captured_at  = COALESCE(captured_at, ?),
                    uploaded_at  = COALESCE(uploaded_at, ?),
                    description  = COALESCE(description, ?),
                    google_url   = COALESCE(google_url, ?),
                    favorited    = MAX(favorited, ?)
                 WHERE id = ?",
            )
            .bind(now())
            .bind(media.captured_at)
            .bind(media.uploaded_at)
            .bind(media.description.as_deref())
            .bind(media.google_url.as_deref())
            .bind(i64::from(media.favorited))
            .bind(existing.get())
            .execute(&self.pool)
            .await?;
            return Ok((existing, false));
        }

        let id = sqlx::query(
            "INSERT INTO media (
                object_hash, filename, kind, source, captured_at, uploaded_at,
                description, favorited, google_url, import_run_id, last_seen_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(media.object.to_hex())
        .bind(&media.filename)
        .bind(media.kind.as_str())
        .bind(media.source.as_str())
        .bind(media.captured_at)
        .bind(media.uploaded_at)
        .bind(media.description.as_deref())
        .bind(i64::from(media.favorited))
        .bind(media.google_url.as_deref())
        .bind(media.import_run.0)
        .bind(now())
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("id");

        Ok((MediaId::new(id), true))
    }

    /// Procura o item que corresponde a um objeto.
    pub async fn media_id_for_object(&self, hash: &ObjectHash) -> Result<Option<MediaId>> {
        let row = sqlx::query("SELECT id FROM media WHERE object_hash = ?")
            .bind(hash.to_hex())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| MediaId::new(row.get::<i64, _>("id"))))
    }

    /// Grava a localização de um item.
    pub async fn set_place(
        &self,
        media: MediaId,
        lat: f64,
        lon: f64,
        altitude: Option<f64>,
        source: &str,
        camera: Option<(f64, f64)>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO media_place (media_id, lat, lon, altitude, lat_exif, lon_exif, source)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(media_id) DO UPDATE SET
                lat = excluded.lat, lon = excluded.lon, altitude = excluded.altitude,
                lat_exif = excluded.lat_exif, lon_exif = excluded.lon_exif,
                source = excluded.source",
        )
        .bind(media.get())
        .bind(lat)
        .bind(lon)
        .bind(altitude)
        .bind(camera.map(|(lat, _)| lat))
        .bind(camera.map(|(_, lon)| lon))
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Pessoas -------------------------------------------------------------

    /// Registra uma pessoa, ou devolve a existente.
    pub async fn upsert_person(&self, display: &str, key: &str) -> Result<PersonId> {
        let id = sqlx::query(
            "INSERT INTO person (name, name_key) VALUES (?, ?)
             ON CONFLICT(name_key) DO UPDATE SET name = name
             RETURNING id",
        )
        .bind(display)
        .bind(key)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("id");
        Ok(PersonId(id))
    }

    /// Marca uma pessoa em um item.
    pub async fn tag_person(&self, media: MediaId, person: PersonId, source: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO person_tag (media_id, person_id, source) VALUES (?, ?, ?)
             ON CONFLICT(media_id, person_id) DO NOTHING",
        )
        .bind(media.get())
        .bind(person.0)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Álbuns --------------------------------------------------------------

    /// Registra um álbum, ou devolve o existente.
    pub async fn upsert_album(
        &self,
        title: &str,
        key: &str,
        description: Option<&str>,
    ) -> Result<AlbumId> {
        let id = sqlx::query(
            "INSERT INTO album (title, title_key, description, created_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(title_key) DO UPDATE SET
                description = COALESCE(excluded.description, album.description)
             RETURNING id",
        )
        .bind(title)
        .bind(key)
        .bind(description)
        .bind(now())
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("id");
        Ok(AlbumId(id))
    }

    /// Associa um item a um álbum.
    ///
    /// Idempotente: reimportar não multiplica associações.
    pub async fn add_to_album(
        &self,
        album: AlbumId,
        media: MediaId,
        position: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO album_media (album_id, media_id, position) VALUES (?, ?, ?)
             ON CONFLICT(album_id, media_id) DO NOTHING",
        )
        .bind(album.0)
        .bind(media.get())
        .bind(position)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Fila de revisão -----------------------------------------------------

    /// Registra um sidecar que não encontrou dono.
    pub async fn record_orphan(
        &self,
        run: ImportRunId,
        directory: &str,
        sidecar: &str,
        reason: &str,
        candidates: &[String],
    ) -> Result<()> {
        let candidates = if candidates.is_empty() {
            None
        } else {
            Some(serde_json::to_string(candidates).unwrap_or_default())
        };
        sqlx::query(
            "INSERT INTO sidecar_orphan (import_run_id, directory, sidecar, reason, candidates)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(directory, sidecar) WHERE resolved_at IS NULL DO NOTHING",
        )
        .bind(run.0)
        .bind(directory)
        .bind(sidecar)
        .bind(reason)
        .bind(candidates)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Registra que um item é versão editada de outro.
    ///
    /// Não é apenas informação: é o que impede a deduplicação de apagar a foto editada por ela
    /// ser visualmente quase idêntica à original.
    pub async fn set_edited_from(&self, edited: MediaId, original: MediaId) -> Result<()> {
        self.link(edited, original, "edited_from").await
    }

    /// Registra que um item é o componente de movimento de uma Live Photo.
    pub async fn set_motion_part_of(&self, motion: MediaId, still: MediaId) -> Result<()> {
        self.link(motion, still, "motion_part_of").await
    }

    async fn link(&self, child: MediaId, parent: MediaId, column: &str) -> Result<()> {
        // Um item não pode ser parente de si mesmo. Acontece quando dois caminhos apontam para
        // os mesmos bytes e, portanto, para o mesmo item lógico.
        if child == parent {
            return Ok(());
        }
        // `column` vem de chamadas internas com valores literais, nunca de entrada externa.
        let sql = format!("UPDATE media SET {column} = ? WHERE id = ? AND {column} IS NULL");
        sqlx::query(&sql)
            .bind(parent.get())
            .bind(child.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Itens prontos para normalização, com tudo que precisa ser embutido no arquivo.
    ///
    /// Exclui itens já apagados. Devolve também as pessoas, mesmo sabendo que o backend nativo
    /// não as grava — para que o relatório possa dizer exatamente o que ficou de fora.
    pub async fn media_for_normalization(
        &self,
        limit: Option<i64>,
    ) -> Result<Vec<records::NormalizationRow>> {
        let rows = sqlx::query(
            "SELECT media.id, media.object_hash, media.filename, media.captured_at,
                    media.description, media.favorited,
                    media_place.lat, media_place.lon, media_place.altitude
             FROM media
             LEFT JOIN media_place ON media_place.media_id = media.id
             WHERE media.deleted_at IS NULL
             ORDER BY media.id
             LIMIT ?",
        )
        .bind(limit.unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let media_id: i64 = row.get("id");
            let people: Vec<String> = sqlx::query_scalar(
                "SELECT person.name FROM person
                 JOIN person_tag ON person_tag.person_id = person.id
                 WHERE person_tag.media_id = ?
                 ORDER BY person.name_key",
            )
            .bind(media_id)
            .fetch_all(&self.pool)
            .await?;

            let lat: Option<f64> = row.get("lat");
            let lon: Option<f64> = row.get("lon");

            out.push(records::NormalizationRow {
                media_id,
                object_hash: row.get("object_hash"),
                filename: row.get("filename"),
                captured_at: row.get("captured_at"),
                description: row.get("description"),
                favorited: row.get::<i64, _>("favorited") != 0,
                location: lat
                    .zip(lon)
                    .map(|(lat, lon)| (lat, lon, row.get("altitude"))),
                people,
            });
        }
        Ok(out)
    }

    /// Sidecars que ainda aguardam revisão humana.
    pub async fn pending_orphans(&self) -> Result<Vec<records::OrphanRow>> {
        let rows = sqlx::query(
            "SELECT id, directory, sidecar, reason, candidates
             FROM sidecar_orphan WHERE resolved_at IS NULL
             ORDER BY directory, sidecar",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| records::OrphanRow {
                id: row.get("id"),
                directory: row.get("directory"),
                sidecar: row.get("sidecar"),
                reason: row.get("reason"),
                candidates: row
                    .get::<Option<String>, _>("candidates")
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
            })
            .collect())
    }

    // -- Restauração ---------------------------------------------------------

    /// Abre uma restauração.
    pub async fn begin_restore(
        &self,
        sink: &str,
        dry_run: bool,
        items_total: u64,
        bytes_total: u64,
        requests_estimated: u64,
    ) -> Result<RestoreRunId> {
        let id = sqlx::query(
            "INSERT INTO restore_run
                (sink, started_at, status, dry_run, items_total, bytes_total, requests_estimated)
             VALUES (?, ?, 'running', ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(sink)
        .bind(now())
        .bind(i64::from(dry_run))
        .bind(items_total as i64)
        .bind(bytes_total as i64)
        .bind(requests_estimated as i64)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("id");
        Ok(RestoreRunId(id))
    }

    /// Enfileira um item numa restauração.
    ///
    /// Devolve `false` quando a chave de idempotência já existe — isto é, quando o item já foi
    /// enviado para esta conta, nesta ou em outra execução. É a única proteção contra duplicar
    /// itens na biblioteca, porque não há API para listar o que já está lá.
    pub async fn enqueue_restore_item(
        &self,
        run: RestoreRunId,
        media: MediaId,
        idempotency_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO restore_item
                (restore_run_id, media_id, idempotency_key, status, updated_at)
             VALUES (?, ?, ?, 'pending', ?)
             ON CONFLICT(idempotency_key) DO NOTHING",
        )
        .bind(run.0)
        .bind(media.get())
        .bind(idempotency_key)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Itens de uma restauração que ainda não subiram.
    pub async fn pending_restore_items(&self, run: RestoreRunId) -> Result<Vec<RestoreItemRow>> {
        let rows = sqlx::query(
            "SELECT restore_item.media_id, restore_item.idempotency_key,
                    restore_item.remote_media_id, restore_item.status, restore_item.attempts,
                    media.object_hash, media.filename, media.description
             FROM restore_item
             JOIN media ON media.id = restore_item.media_id
             WHERE restore_item.restore_run_id = ?
               AND restore_item.remote_media_id IS NULL
               AND restore_item.status != 'skipped'
             ORDER BY restore_item.media_id",
        )
        .bind(run.0)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| RestoreItemRow {
                media_id: row.get("media_id"),
                object_hash: row.get("object_hash"),
                filename: row.get("filename"),
                description: row.get("description"),
                idempotency_key: row.get("idempotency_key"),
                remote_media_id: row.get("remote_media_id"),
                status: row.get("status"),
                attempts: row.get("attempts"),
            })
            .collect())
    }

    /// Registra que os bytes de um item subiram.
    pub async fn mark_uploaded(
        &self,
        run: RestoreRunId,
        media: MediaId,
        upload_token: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE restore_item
             SET status = 'uploaded', upload_token = ?, updated_at = ?
             WHERE restore_run_id = ? AND media_id = ?",
        )
        .bind(upload_token)
        .bind(now())
        .bind(run.0)
        .bind(media.get())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Registra que o item foi criado no destino.
    ///
    /// O identificador remoto é gravado na mesma transação em que o item é marcado como criado.
    /// Gravá-lo depois abriria uma janela em que uma queda faria o item ser reenviado.
    pub async fn mark_created(
        &self,
        run: RestoreRunId,
        media: MediaId,
        remote_id: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE restore_item
             SET status = 'created', remote_media_id = ?, updated_at = ?
             WHERE restore_run_id = ? AND media_id = ?",
        )
        .bind(remote_id)
        .bind(now())
        .bind(run.0)
        .bind(media.get())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE restore_run SET items_done = items_done + 1 WHERE id = ?")
            .bind(run.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Registra uma falha num item.
    pub async fn mark_restore_failed(
        &self,
        run: RestoreRunId,
        media: MediaId,
        error: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE restore_item
             SET status = 'failed', attempts = attempts + 1, last_error = ?, updated_at = ?
             WHERE restore_run_id = ? AND media_id = ?",
        )
        .bind(error)
        .bind(now())
        .bind(run.0)
        .bind(media.get())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE restore_run SET items_failed = items_failed + 1 WHERE id = ?")
            .bind(run.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Fecha uma restauração com o estado final.
    pub async fn finish_restore(&self, run: RestoreRunId, status: &str) -> Result<()> {
        sqlx::query("UPDATE restore_run SET status = ?, finished_at = ? WHERE id = ?")
            .bind(status)
            .bind(now())
            .bind(run.0)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Contagens de uma restauração.
    pub async fn restore_progress(&self, run: RestoreRunId) -> Result<(u64, u64, u64)> {
        let row = sqlx::query(
            "SELECT items_total, items_done, items_failed FROM restore_run WHERE id = ?",
        )
        .bind(run.0)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            row.get::<i64, _>("items_total") as u64,
            row.get::<i64, _>("items_done") as u64,
            row.get::<i64, _>("items_failed") as u64,
        ))
    }

    // -- Auditoria -----------------------------------------------------------

    /// Acrescenta uma entrada ao log encadeado.
    pub async fn append_audit(
        &self,
        operation: &str,
        object_ref: Option<&str>,
        result: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let ts = now();
        let previous: Option<String> =
            sqlx::query("SELECT entry_hash FROM audit_log ORDER BY id DESC LIMIT 1")
                .fetch_optional(&self.pool)
                .await?
                .map(|row| row.get("entry_hash"));

        let entry_hash = chain_hash(
            previous.as_deref(),
            ts,
            operation,
            object_ref,
            result,
            details,
        );

        sqlx::query(
            "INSERT INTO audit_log (ts, operation, object_ref, result, details, prev_hash, entry_hash)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(operation)
        .bind(object_ref)
        .bind(result)
        .bind(details)
        .bind(previous.as_deref())
        .bind(entry_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Recalcula a cadeia inteira e aponta a primeira entrada inconsistente.
    pub async fn verify_audit_chain(&self) -> Result<usize> {
        let rows = sqlx::query(
            "SELECT id, ts, operation, object_ref, result, details, prev_hash, entry_hash
             FROM audit_log ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut previous: Option<String> = None;
        for row in &rows {
            let id: i64 = row.get("id");
            let stored_prev: Option<String> = row.get("prev_hash");
            if stored_prev != previous {
                return Err(CatalogError::BrokenAuditChain { id });
            }
            let expected = chain_hash(
                previous.as_deref(),
                row.get("ts"),
                row.get("operation"),
                row.get::<Option<String>, _>("object_ref").as_deref(),
                row.get("result"),
                row.get::<Option<String>, _>("details").as_deref(),
            );
            let stored: String = row.get("entry_hash");
            if stored != expected {
                return Err(CatalogError::BrokenAuditChain { id });
            }
            previous = Some(stored);
        }
        Ok(rows.len())
    }

    // -- Consultas -----------------------------------------------------------

    /// Números gerais do cofre.
    pub async fn stats(&self) -> Result<CatalogStats> {
        let row = sqlx::query(
            "SELECT
                (SELECT COUNT(*) FROM media WHERE deleted_at IS NULL)     AS media,
                (SELECT COUNT(*) FROM object)                             AS objects,
                (SELECT COALESCE(SUM(size), 0) FROM object)               AS bytes,
                (SELECT COUNT(*) FROM album)                              AS albums,
                (SELECT COUNT(*) FROM person)                             AS people,
                (SELECT COUNT(*) FROM media_place)                        AS located,
                (SELECT COUNT(*) FROM object WHERE verified_at IS NOT NULL) AS verified,
                (SELECT COUNT(*) FROM sidecar_orphan WHERE resolved_at IS NULL) AS orphans",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(CatalogStats {
            media: row.get::<i64, _>("media") as u64,
            objects: row.get::<i64, _>("objects") as u64,
            bytes: row.get::<i64, _>("bytes") as u64,
            albums: row.get::<i64, _>("albums") as u64,
            people: row.get::<i64, _>("people") as u64,
            located: row.get::<i64, _>("located") as u64,
            verified: row.get::<i64, _>("verified") as u64,
            pending_orphans: row.get::<i64, _>("orphans") as u64,
        })
    }
}

/// Epoch em segundos, UTC.
fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Hash de uma entrada de auditoria, encadeado à anterior.
fn chain_hash(
    previous: Option<&str>,
    ts: i64,
    operation: &str,
    object_ref: Option<&str>,
    result: &str,
    details: Option<&str>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    // Separador explícito entre campos: sem ele, ("ab", "c") e ("a", "bc") colidiriam.
    for field in [
        previous.unwrap_or(""),
        &ts.to_string(),
        operation,
        object_ref.unwrap_or(""),
        result,
        details.unwrap_or(""),
    ] {
        hasher.update(field.as_bytes());
        hasher.update(b"\x1f");
    }
    hasher.finalize().to_hex().to_string()
}
