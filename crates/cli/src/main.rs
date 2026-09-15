//! `photovault` — linha de comando do cofre.
//!
//! A interface gráfica vem depois (V0.4). Esta CLI existe porque o risco do projeto está na
//! ingestão, não na tela: se o parser do Takeout não for confiável, nada mais importa.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use photovault_cas::ObjectStore;
use photovault_catalog::{records::human_bytes, Catalog};
use photovault_core::{GeoPoint, PersonName};
use photovault_exif::{EmbeddedMetadata, ExifError, Field};

mod import;

/// Gerenciador do acervo do Google Fotos.
#[derive(Debug, Parser)]
#[command(name = "photovault", version, about)]
struct Cli {
    /// Raiz do cofre.
    #[arg(long, global = true, default_value = "~/PhotoVault")]
    vault: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Importa um archive do Google Takeout já extraído.
    ImportTakeout {
        /// Diretório extraído do Takeout.
        path: PathBuf,
        /// Rótulo desta importação, para o histórico. Ex.: "archive 1 de 8".
        #[arg(long)]
        label: Option<String>,
    },
    /// Mostra os números do cofre.
    Status,
    /// Relê os objetos do disco e confere a integridade.
    Verify {
        /// Verifica apenas uma fração dos objetos, como faz o scrub periódico.
        #[arg(long, value_name = "PERCENTUAL")]
        sample: Option<u8>,
    },
    /// Lista os sidecars que precisam de revisão humana.
    Orphans,
    /// Grava os metadados do catálogo dentro de cópias dos arquivos.
    ///
    /// O objeto original nunca é tocado. A cópia enriquecida vai para `derived/normalized/`,
    /// e é ela que sobe para o Google — sem GPS embutido, a localização se perde no caminho.
    Normalize {
        /// Processa apenas os primeiros N itens.
        #[arg(long)]
        limit: Option<i64>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "photovault=info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let vault = expand_home(&cli.vault)?;

    match cli.command {
        Command::ImportTakeout { path, label } => run_import(&vault, &path, label.as_deref()).await,
        Command::Status => run_status(&vault).await,
        Command::Verify { sample } => run_verify(&vault, sample).await,
        Command::Orphans => run_orphans(&vault).await,
        Command::Normalize { limit } => run_normalize(&vault, limit).await,
    }
}

/// Abre (ou cria) o cofre e devolve suas duas metades.
async fn open_vault(vault: &Path) -> Result<(ObjectStore, Catalog)> {
    std::fs::create_dir_all(vault.join("database"))
        .with_context(|| format!("criar cofre em {}", vault.display()))?;
    let store = ObjectStore::open(vault.join("repository")).context("abrir repositório")?;
    let catalog = Catalog::open(vault.join("database/photovault.db"))
        .await
        .context("abrir catálogo")?;
    Ok((store, catalog))
}

async fn run_import(vault: &Path, source: &Path, label: Option<&str>) -> Result<()> {
    if !source.is_dir() {
        bail!(
            "{} não é um diretório. Extraia o archive do Takeout antes de importar.",
            source.display()
        );
    }

    let (store, catalog) = open_vault(vault).await?;

    println!("Cofre     {}", vault.display());
    println!("Origem    {}", source.display());
    println!();

    let outcome = import::import_takeout(source, &store, &catalog, label).await?;
    let tally = &outcome.tally;

    println!("IMPORTAÇÃO");
    println!("  arquivos de mídia vistos     {:>8}", tally.media_seen);
    println!("  itens catalogados            {:>8}", tally.media_imported);
    println!(
        "  já existiam no cofre         {:>8}",
        tally.media_deduplicated
    );
    println!(
        "  bytes guardados              {:>8}",
        human_bytes(tally.bytes_stored)
    );
    if tally.relations_linked > 0 {
        println!(
            "  parentescos ligados          {:>8}",
            tally.relations_linked
        );
    }
    println!();
    println!("SIDECARS");
    println!(
        "  associados                   {:>8}",
        tally.sidecars_matched
    );
    println!(
        "  mídia sem sidecar            {:>8}",
        tally.media_without_sidecar
    );
    println!(
        "  órfãos (precisam revisão)    {:>8}",
        tally.sidecars_orphan
    );

    if !outcome.albums.is_empty() {
        println!();
        println!("ÁLBUNS  {}", outcome.albums.len());
        for (title, items) in outcome.albums.iter().take(15) {
            println!("  {items:>6}  {title}");
        }
        if outcome.albums.len() > 15 {
            println!("  … e mais {}", outcome.albums.len() - 15);
        }
    }

    if !outcome.orphans.is_empty() {
        println!();
        println!("SIDECARS SEM DONO  {}", outcome.orphans.len());
        for orphan in outcome.orphans.iter().take(10) {
            println!("  {}/{}", orphan.directory, orphan.sidecar);
            println!("      {}", orphan.reason);
        }
        if outcome.orphans.len() > 10 {
            println!("  … e mais {}", outcome.orphans.len() - 10);
        }
    }

    if !outcome.warnings.is_empty() {
        println!();
        println!("AVISOS  {}", outcome.warnings.len());
        for warning in outcome.warnings.iter().take(10) {
            println!("  {warning}");
        }
        if outcome.warnings.len() > 10 {
            println!("  … e mais {}", outcome.warnings.len() - 10);
        }
    }

    if !outcome.failures.is_empty() {
        println!();
        println!("FALHAS  {}", outcome.failures.len());
        for failure in outcome.failures.iter().take(10) {
            println!("  {failure}");
        }
    }

    if tally.sidecars_orphan > 0 {
        println!();
        println!("Nenhum órfão foi descartado. Veja com: photovault orphans");
    }

    Ok(())
}

async fn run_status(vault: &Path) -> Result<()> {
    let (_, catalog) = open_vault(vault).await?;
    let stats = catalog.stats().await.context("consultar estatísticas")?;

    println!("COFRE  {}", vault.display());
    println!();
    println!("  itens                  {:>10}", stats.media);
    println!("  objetos distintos      {:>10}", stats.objects);
    println!("  tamanho                {:>10}", stats.human_bytes());
    println!("  álbuns                 {:>10}", stats.albums);
    println!("  pessoas                {:>10}", stats.people);
    println!("  com geolocalização     {:>10}", stats.located);
    println!();
    println!("  verificados            {:>10}", stats.verified);
    if stats.objects > 0 && stats.verified < stats.objects {
        println!(
            "  não verificados        {:>10}   rode: photovault verify",
            stats.objects - stats.verified
        );
    }
    if stats.pending_orphans > 0 {
        println!();
        println!("  sidecars para revisar  {:>10}", stats.pending_orphans);
    }

    Ok(())
}

async fn run_verify(vault: &Path, sample: Option<u8>) -> Result<()> {
    let (store, catalog) = open_vault(vault).await?;
    let objects = store.iter_objects().context("listar objetos")?;

    let step = match sample {
        Some(0) => bail!("a amostra precisa ser maior que zero"),
        Some(percent) if percent < 100 => (100 / u64::from(percent)).max(1),
        _ => 1,
    };

    let mut checked = 0u64;
    let mut corrupted = Vec::new();

    for (index, hash) in objects.iter().enumerate() {
        if index as u64 % step != 0 {
            continue;
        }
        checked += 1;
        match store.verify(hash) {
            Ok(()) => catalog
                .mark_verified(hash)
                .await
                .context("marcar verificado")?,
            Err(error) => corrupted.push(format!("{hash}: {error}")),
        }
    }

    println!("VERIFICAÇÃO DE INTEGRIDADE");
    println!("  objetos no cofre       {:>10}", objects.len());
    println!("  verificados agora      {:>10}", checked);
    println!("  corrompidos            {:>10}", corrupted.len());

    if !corrupted.is_empty() {
        println!();
        for entry in &corrupted {
            println!("  {entry}");
        }
        catalog
            .append_audit(
                "integrity.verify",
                None,
                "falha",
                Some(&format!("{} objetos corrompidos", corrupted.len())),
            )
            .await?;
        bail!("{} objeto(s) corrompido(s)", corrupted.len());
    }

    catalog
        .append_audit(
            "integrity.verify",
            None,
            "ok",
            Some(&format!("{checked} objetos verificados")),
        )
        .await?;

    println!();
    println!("Integridade local confirmada.");
    println!("Isto verifica que os bytes no disco continuam íntegros — não que sejam");
    println!("idênticos ao que está no Google, que não expõe hash para comparação.");

    Ok(())
}

async fn run_orphans(vault: &Path) -> Result<()> {
    let (_, catalog) = open_vault(vault).await?;

    let rows = catalog
        .pending_orphans()
        .await
        .context("consultar órfãos")?;

    if rows.is_empty() {
        println!("Nenhum sidecar pendente de revisão.");
        return Ok(());
    }

    println!("SIDECARS SEM DONO  {}", rows.len());
    println!();
    for orphan in rows {
        println!("  #{}  {}/{}", orphan.id, orphan.directory, orphan.sidecar);
        println!("      {}", orphan.reason);
        for candidate in orphan.candidates {
            println!("      candidato: {candidate}");
        }
    }

    Ok(())
}

async fn run_normalize(vault: &Path, limit: Option<i64>) -> Result<()> {
    let (store, catalog) = open_vault(vault).await?;
    let rows = catalog
        .media_for_normalization(limit)
        .await
        .context("consultar itens")?;

    let target_root = vault.join("derived/normalized");
    let mut normalized = 0u64;
    let mut unsupported: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut with_location = 0u64;
    let mut losses: std::collections::BTreeMap<Field, (u64, String)> = Default::default();

    for row in &rows {
        let Ok(hash) = photovault_core::ObjectHash::from_hex(&row.object_hash) else {
            failures.push(format!("{}: hash inválido no catálogo", row.filename));
            continue;
        };
        let source = store.path_for(&hash);

        let extension = row
            .filename
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        let destination = target_root
            .join(hash.fanout())
            .join(format!("{}.{extension}", hash.to_hex()));

        let metadata = EmbeddedMetadata {
            captured_at: row
                .captured_at
                .and_then(|epoch| time::OffsetDateTime::from_unix_timestamp(epoch).ok()),
            location: row
                .location
                .and_then(|(lat, lon, alt)| GeoPoint::new(lat, lon, alt).ok()),
            description: row.description.clone(),
            people: row
                .people
                .iter()
                .filter_map(|n| PersonName::new(n))
                .collect(),
            favorited: row.favorited,
        };

        match photovault_exif::normalize(&source, &destination, &metadata) {
            Ok(outcome) => {
                normalized += 1;
                if outcome.has_location() {
                    with_location += 1;
                }
                for (field, remedy) in outcome.losses() {
                    let entry = losses.entry(field).or_insert((0, remedy));
                    entry.0 += 1;
                }
            }
            Err(ExifError::UnsupportedFormat(extension)) => unsupported.push(extension),
            Err(error) => failures.push(format!("{}: {error}", row.filename)),
        }
    }

    println!("NORMALIZAÇÃO");
    println!("  itens no catálogo            {:>8}", rows.len());
    println!("  arquivos normalizados        {:>8}", normalized);
    println!("  com geolocalização embutida  {:>8}", with_location);

    if !unsupported.is_empty() {
        let mut kinds: Vec<String> = unsupported.clone();
        kinds.sort();
        kinds.dedup();
        println!(
            "  sem escrita nativa           {:>8}   ({})",
            unsupported.len(),
            kinds.join(", ")
        );
    }

    if !losses.is_empty() {
        println!();
        println!("NÃO EMBUTIDO");
        for (field, (count, remedy)) in &losses {
            println!("  {:>6}  {}", count, field.describe());
            println!("          {remedy}");
        }
        if !photovault_exif::exiftool_available() {
            println!();
            println!("  O ExifTool não está instalado. Instale-o para cobrir estes campos:");
            println!("    sudo apt install libimage-exiftool-perl");
        }
    }

    if !failures.is_empty() {
        println!();
        println!("FALHAS  {}", failures.len());
        for failure in failures.iter().take(10) {
            println!("  {failure}");
        }
    }

    catalog
        .append_audit(
            "metadata.normalize",
            None,
            "ok",
            Some(&format!("{normalized} arquivos, {with_location} com GPS")),
        )
        .await?;

    println!();
    println!("Os objetos originais não foram modificados (ADR-004).");
    Ok(())
}

/// Expande `~` no início do caminho.
fn expand_home(raw: &str) -> Result<PathBuf> {
    let Some(rest) = raw.strip_prefix('~') else {
        return Ok(PathBuf::from(raw));
    };
    let home = std::env::var("HOME").context("HOME não está definido para expandir '~'")?;
    Ok(PathBuf::from(home).join(rest.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde() {
        std::env::set_var("HOME", "/home/teste");
        assert_eq!(
            expand_home("~/PhotoVault").expect("expande"),
            PathBuf::from("/home/teste/PhotoVault")
        );
        assert_eq!(
            expand_home("/absoluto/cofre").expect("expande"),
            PathBuf::from("/absoluto/cofre")
        );
    }
}
