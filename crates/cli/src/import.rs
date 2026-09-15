//! Importação de um archive do Google Takeout.
//!
//! Percorre a árvore extraída, casa cada arquivo de mídia com seu sidecar, guarda os bytes no
//! CAS e cataloga o significado. Trabalha **um diretório por vez**, porque o Takeout repete o
//! mesmo arquivo em várias pastas com sidecars possivelmente divergentes, e casar através de
//! fronteiras de diretório associa o sidecar errado ao arquivo certo.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use photovault_cas::ObjectStore;
use photovault_catalog::{AlbumId, Catalog, ImportRunId, ImportTally, NewMedia};
use photovault_core::MediaId;
use photovault_core::{Fidelity, MediaKind, SourceKind};
use photovault_takeout::{
    detect_relations, match_directory, MatchConfidence, OrphanReason, Relation, Sidecar,
};

/// Resultado de uma importação, para o relatório final.
#[derive(Debug, Default)]
pub struct ImportOutcome {
    /// Contagens agregadas.
    pub tally: ImportTally,
    /// Álbuns reconstruídos, com quantos itens cada um recebeu.
    pub albums: BTreeMap<String, u64>,
    /// Sidecars que precisam de revisão humana.
    pub orphans: Vec<OrphanNote>,
    /// Avisos emitidos ao interpretar sidecars.
    pub warnings: Vec<String>,
    /// Arquivos que falharam, com o motivo.
    pub failures: Vec<String>,
}

/// Um sidecar sem dono, preservado para revisão.
#[derive(Debug)]
pub struct OrphanNote {
    /// Diretório onde estava.
    pub directory: String,
    /// Nome do sidecar.
    pub sidecar: String,
    /// Por que não casou.
    pub reason: String,
}

/// Importa um archive já extraído.
pub async fn import_takeout(
    root: &Path,
    store: &ObjectStore,
    catalog: &Catalog,
    label: Option<&str>,
) -> Result<ImportOutcome> {
    let run = catalog
        .begin_import("takeout", label)
        .await
        .context("abrir registro de importação")?;
    catalog
        .append_audit("import.begin", None, "ok", label)
        .await
        .context("registrar início na auditoria")?;

    let mut outcome = ImportOutcome::default();

    for directory in collect_directories(root)? {
        if let Err(error) =
            import_directory(&directory, root, store, catalog, run, &mut outcome).await
        {
            outcome.tally.failures += 1;
            outcome
                .failures
                .push(format!("{}: {error:#}", directory.display()));
        }
    }

    catalog
        .finish_import(run, &outcome.tally)
        .await
        .context("fechar registro de importação")?;
    catalog
        .append_audit(
            "import.finish",
            None,
            "ok",
            Some(&format!(
                "{} itens catalogados, {} órfãos",
                outcome.tally.media_imported, outcome.tally.sidecars_orphan
            )),
        )
        .await
        .context("registrar fim na auditoria")?;

    Ok(outcome)
}

/// Lista todos os diretórios da árvore, incluindo a raiz.
fn collect_directories(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = vec![root.to_owned()];
    let mut queue = vec![root.to_owned()];

    while let Some(current) = queue.pop() {
        let entries =
            fs::read_dir(&current).with_context(|| format!("listar {}", current.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("listar {}", current.display()))?;
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let path = entry.path();
                found.push(path.clone());
                queue.push(path);
            }
        }
    }

    found.sort();
    Ok(found)
}

async fn import_directory(
    directory: &Path,
    root: &Path,
    store: &ObjectStore,
    catalog: &Catalog,
    run: ImportRunId,
    outcome: &mut ImportOutcome,
) -> Result<()> {
    let names = file_names(directory)?;
    if names.is_empty() {
        return Ok(());
    }

    let report = match_directory(&names);
    outcome.tally.sidecars_matched += report.matched.len() as u64;
    outcome.tally.media_without_sidecar += report.media_without_sidecar.len() as u64;

    let relative = directory
        .strip_prefix(root)
        .unwrap_or(directory)
        .display()
        .to_string();

    // Regra de discriminação: um diretório é álbum quando traz `metadata.json`. As pastas
    // "Photos from YYYY" não são álbuns, são a linha do tempo.
    let album = match read_album(directory)? {
        Some(info) => Some(
            catalog
                .upsert_album(
                    &info.title,
                    &album_key(&info.title),
                    info.description.as_deref(),
                )
                .await
                .context("registrar álbum")?,
        ),
        None => None,
    };

    // Nome do arquivo para o item catalogado, usado adiante para ligar os parentes.
    let mut ingested: BTreeMap<String, MediaId> = BTreeMap::new();

    for matched in &report.matched {
        let sidecar_path = directory.join(&matched.sidecar);
        let sidecar = read_sidecar(&sidecar_path, outcome);
        if let Some(id) = ingest(
            &directory.join(&matched.media),
            &matched.media,
            sidecar.as_ref(),
            album,
            store,
            catalog,
            run,
            outcome,
        )
        .await?
        {
            ingested.insert(matched.media.clone(), id);
        }

        if matched.confidence == MatchConfidence::Truncated {
            outcome.warnings.push(format!(
                "{relative}/{}: casado por truncamento com {}",
                matched.sidecar, matched.media
            ));
        }
    }

    // Mídia sem sidecar entra mesmo assim: perder os bytes seria pior que perder o metadado.
    for media in &report.media_without_sidecar {
        if let Some(id) = ingest(
            &directory.join(media),
            media,
            None,
            album,
            store,
            catalog,
            run,
            outcome,
        )
        .await?
        {
            ingested.insert(media.clone(), id);
        }
    }

    // Parentesco: precisa acontecer depois que todos os itens do diretório existem, porque um
    // vínculo liga dois deles.
    link_relations(&ingested, catalog, outcome).await?;

    for orphan in &report.orphan_sidecars {
        let candidates = match &orphan.reason {
            OrphanReason::Ambiguous(names) => names.clone(),
            OrphanReason::NoCandidate => Vec::new(),
        };
        let reason = orphan.reason.describe();
        catalog
            .record_orphan(run, &relative, &orphan.sidecar, &reason, &candidates)
            .await
            .context("registrar sidecar órfão")?;
        outcome.tally.sidecars_orphan += 1;
        outcome.orphans.push(OrphanNote {
            directory: relative.clone(),
            sidecar: orphan.sidecar.clone(),
            reason,
        });
    }

    if let (Some(_), Some(info)) = (album, read_album(directory)?) {
        *outcome.albums.entry(info.title).or_insert(0) += report.matched.len() as u64;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn ingest(
    path: &Path,
    filename: &str,
    sidecar: Option<&Sidecar>,
    album: Option<AlbumId>,
    store: &ObjectStore,
    catalog: &Catalog,
    run: ImportRunId,
    outcome: &mut ImportOutcome,
) -> Result<Option<MediaId>> {
    let Some(kind) = media_kind(filename) else {
        // Não é mídia: `archive_browser.html`, `.txt` de descrição, e afins.
        return Ok(None);
    };

    outcome.tally.media_seen += 1;

    let stored = match store.store_file(path) {
        Ok(stored) => stored,
        Err(error) => {
            outcome.tally.failures += 1;
            outcome
                .failures
                .push(format!("{}: {error}", path.display()));
            return Ok(None);
        }
    };

    if stored.deduplicated {
        outcome.tally.media_deduplicated += 1;
    } else {
        outcome.tally.bytes_stored += stored.size;
    }

    catalog
        .upsert_object(&stored.hash, stored.size, Fidelity::Original)
        .await
        .context("registrar objeto")?;

    let record = NewMedia {
        object: stored.hash,
        filename: filename.to_owned(),
        kind,
        source: SourceKind::Takeout,
        captured_at: sidecar.and_then(|s| s.taken_at.map(|t| t.unix_timestamp())),
        uploaded_at: sidecar.and_then(|s| s.uploaded_at.map(|t| t.unix_timestamp())),
        description: sidecar.and_then(|s| s.description.clone()),
        favorited: sidecar.is_some_and(|s| s.favorited),
        google_url: sidecar.and_then(|s| s.google_url.clone()),
        import_run: run,
    };

    let (media_id, is_new) = catalog
        .upsert_media(&record)
        .await
        .context("catalogar item")?;
    if is_new {
        outcome.tally.media_imported += 1;
    }

    if let Some(sidecar) = sidecar {
        if let Some((point, source)) = sidecar.location {
            catalog
                .set_place(
                    media_id,
                    point.latitude(),
                    point.longitude(),
                    point.altitude(),
                    source.as_str(),
                    sidecar
                        .camera_location
                        .map(|camera| (camera.latitude(), camera.longitude())),
                )
                .await
                .context("gravar localização")?;
        }

        for person in &sidecar.people {
            let person_id = catalog
                .upsert_person(person.display(), person.key())
                .await
                .context("registrar pessoa")?;
            catalog
                .tag_person(media_id, person_id, "takeout")
                .await
                .context("marcar pessoa")?;
        }
    }

    if let Some(album) = album {
        catalog
            .add_to_album(album, media_id, None)
            .await
            .context("associar ao álbum")?;
    }

    Ok(Some(media_id))
}

/// Aplica os vínculos de parentesco detectados entre os arquivos do diretório.
async fn link_relations(
    ingested: &BTreeMap<String, MediaId>,
    catalog: &Catalog,
    outcome: &mut ImportOutcome,
) -> Result<()> {
    let names: Vec<&String> = ingested.keys().collect();
    for relation in detect_relations(names) {
        match relation {
            Relation::EditedFrom { edited, original } => {
                if let (Some(&child), Some(&parent)) =
                    (ingested.get(&edited), ingested.get(&original))
                {
                    catalog
                        .set_edited_from(child, parent)
                        .await
                        .context("ligar versão editada")?;
                    outcome.tally.relations_linked += 1;
                }
            }
            Relation::MotionPartOf { motion, still } => {
                if let (Some(&child), Some(&parent)) = (ingested.get(&motion), ingested.get(&still))
                {
                    catalog
                        .set_motion_part_of(child, parent)
                        .await
                        .context("ligar componente de movimento")?;
                    outcome.tally.relations_linked += 1;
                }
            }
        }
    }
    Ok(())
}

fn read_sidecar(path: &Path, outcome: &mut ImportOutcome) -> Option<Sidecar> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            outcome
                .failures
                .push(format!("{}: {error}", path.display()));
            return None;
        }
    };
    match Sidecar::from_json(&bytes) {
        Ok(sidecar) => {
            for warning in &sidecar.warnings {
                outcome
                    .warnings
                    .push(format!("{}: {warning}", path.display()));
            }
            Some(sidecar)
        }
        Err(error) => {
            outcome
                .failures
                .push(format!("{}: {error}", path.display()));
            None
        }
    }
}

/// Metadados de álbum, quando o diretório é um álbum.
struct AlbumInfo {
    title: String,
    description: Option<String>,
}

fn read_album(directory: &Path) -> Result<Option<AlbumInfo>> {
    let path = directory.join("metadata.json");
    if !path.is_file() {
        return Ok(None);
    }

    #[derive(serde::Deserialize)]
    struct Raw {
        #[serde(default)]
        title: String,
        #[serde(default)]
        description: Option<String>,
    }

    let bytes = fs::read(&path).with_context(|| format!("ler {}", path.display()))?;
    let raw: Raw = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        // Um metadata.json ilegível não pode derrubar a importação do diretório inteiro.
        Err(_) => return Ok(None),
    };

    let title = if raw.title.trim().is_empty() {
        directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Álbum sem título")
            .to_owned()
    } else {
        raw.title
    };

    Ok(Some(AlbumInfo {
        title,
        description: raw.description.filter(|text| !text.trim().is_empty()),
    }))
}

fn album_key(title: &str) -> String {
    title.trim().to_lowercase()
}

fn file_names(directory: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let entries =
        fs::read_dir(directory).with_context(|| format!("listar {}", directory.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("listar {}", directory.display()))?;
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Natureza do arquivo pela extensão, ou `None` quando não é mídia.
fn media_kind(filename: &str) -> Option<MediaKind> {
    const PHOTO: &[&str] = &[
        "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "tif", "tiff", "bmp", "avif", "dng",
        "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2", "ico",
    ];
    const VIDEO: &[&str] = &[
        "mp4", "mov", "avi", "mkv", "m4v", "3gp", "3g2", "mts", "m2ts", "m2t", "wmv", "mpg",
        "mpeg", "webm", "mod", "tod", "divx", "asf", "mmv",
    ];

    let extension = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    if PHOTO.contains(&extension.as_str()) {
        Some(MediaKind::Photo)
    } else if VIDEO.contains(&extension.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_photo_and_video_extensions() {
        assert_eq!(media_kind("IMG_1002.JPG"), Some(MediaKind::Photo));
        assert_eq!(media_kind("IMG_1002.heic"), Some(MediaKind::Photo));
        assert_eq!(media_kind("PXL_2024.mp4"), Some(MediaKind::Video));
        assert_eq!(media_kind("VID.MOV"), Some(MediaKind::Video));
    }

    #[test]
    fn ignores_non_media_files() {
        assert_eq!(media_kind("archive_browser.html"), None);
        assert_eq!(media_kind("metadata.json"), None);
        assert_eq!(media_kind("sem_extensao"), None);
    }

    #[test]
    fn album_key_is_case_and_space_insensitive() {
        assert_eq!(album_key("  Viagem Japão  "), album_key("viagem japão"));
    }
}
