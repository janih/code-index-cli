//! Search-quality benchmark harness (M1).
//!
//! Compares local embedding models on codebase search over this repo:
//! golden queries with graded file-level labels → index + search per model
//! via llama-server rotation → recall/MRR/nDCG/AUC/thresholds report.
//!
//! Usage:
//!   cargo run --release --example search_quality -- run [--model NAME].. [--port 8099] [--keep]
//!   cargo run --release --example search_quality -- analyze [--baseline NAME]
//!   cargo run --release --example search_quality -- clean
//!
//! See bench/README.md for the design and bench/golden/*.jsonl for labels.

mod config;
mod golden;
mod metrics;
mod report;
mod runner;
mod server;
mod types;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use config::BenchConfig;
use golden::GoldenSet;

#[derive(Parser)]
#[command(
    name = "search-quality",
    about = "Embedding-model search-quality benchmark"
)]
struct Cli {
    /// Qdrant base URL.
    #[arg(long, default_value = "http://localhost:6333")]
    qdrant_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rotate llama-server over models.json, index + query each, analyze.
    Run {
        /// Restrict to these model names (repeatable). Default: all.
        #[arg(long)]
        model: Vec<String>,
        /// Port for llama-server (sequential, one at a time).
        #[arg(long, default_value_t = 8099)]
        port: u16,
        /// Keep collections/cache after the run (default cleans up).
        #[arg(long)]
        keep: bool,
    },
    /// Recompute metrics + report from bench/results (no servers needed).
    Analyze {
        /// Baseline model for paired bootstrap deltas (default: models.json flag).
        #[arg(long)]
        baseline: Option<String>,
    },
    /// Delete collections, cache dirs and symlinks for all models.
    Clean,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bench_dir = repo_root.join("bench");
    let paths = runner::RunPaths {
        repo_root: repo_root.clone(),
        bench_dir: bench_dir.clone(),
    };

    match cli.command {
        Command::Clean => {
            let config = config::load(&bench_dir)?;
            for spec in &config.models {
                runner::cleanup_model(&paths, spec, &cli.qdrant_url).await;
            }
            println!("cleaned collections, caches and symlinks");
        }
        Command::Run { model, port, keep } => {
            let config = config::load(&bench_dir)?;
            let golden = GoldenSet::load(&bench_dir)?;
            warn_stale_paths(&golden, &repo_root);

            let selected: Vec<_> = config
                .models
                .iter()
                .filter(|m| model.is_empty() || model.contains(&m.name))
                .collect();
            if selected.is_empty() {
                anyhow::bail!(
                    "no models match --model {:?} (known: {:?})",
                    model,
                    config
                        .models
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>()
                );
            }

            std::fs::create_dir_all(bench_dir.join("tmp"))?;
            for spec in &selected {
                let slug = spec.slug();
                println!("\n=== {} ({slug}) ===", spec.name);
                let gguf = PathBuf::from(&spec.gguf);
                if !gguf.exists() {
                    eprintln!("[{slug}] GGUF not found: {} — skipping", spec.gguf);
                    continue;
                }
                let log = bench_dir.join("tmp").join(format!("{slug}-server.log"));
                let (mut llama, meta) =
                    match server::LlamaServer::start(&config.llama_server, &gguf, port, &log).await
                    {
                        Ok(v) => v,
                        Err(err) => {
                            eprintln!("[{slug}] {err:#}");
                            continue;
                        }
                    };
                println!(
                    "[{slug}] server up: model {} dim {} n_ctx {:?}",
                    meta.model_id, meta.dim, meta.n_ctx
                );
                let result =
                    runner::run_model(&paths, spec, &meta, port, &golden, &cli.qdrant_url).await;
                llama.kill();
                if let Err(err) = result {
                    eprintln!("[{slug}] {err:#}");
                }
                if !keep {
                    runner::cleanup_model(&paths, spec, &cli.qdrant_url).await;
                }
            }

            analyze(&bench_dir, &golden, &config, None)?;
        }
        Command::Analyze { baseline } => {
            let config = config::load(&bench_dir)?;
            let golden = GoldenSet::load(&bench_dir)?;
            analyze(&bench_dir, &golden, &config, baseline)?;
        }
    }
    Ok(())
}

fn analyze(
    bench_dir: &std::path::Path,
    golden: &GoldenSet,
    config: &BenchConfig,
    baseline_override: Option<String>,
) -> anyhow::Result<()> {
    let results = report::load_results(bench_dir, &[])?;
    let all_metrics: Vec<_> = results
        .iter()
        .map(|r| metrics::compute(r, golden))
        .collect();

    let baseline_name = baseline_override.or_else(|| {
        config
            .models
            .iter()
            .find(|m| m.baseline)
            .map(|m| m.name.clone())
    });
    let paired = report::paired_recall(&results, golden);
    let bootstrap: Vec<(String, Option<report::Delta>)> = match &baseline_name {
        Some(base) => paired
            .iter()
            .filter(|(name, _)| name != base)
            .map(|(name, m)| {
                let b = paired.iter().find(|(n, _)| n == base).map(|(_, v)| v);
                (name.clone(), b.and_then(|b| metrics::bootstrap_delta(m, b)))
            })
            .collect(),
        None => Vec::new(),
    };

    let text = report::render(&all_metrics, &results, golden, &bootstrap);
    let report_dir = bench_dir.join("reports");
    std::fs::create_dir_all(&report_dir)?;
    let latest = report_dir.join("latest.md");
    std::fs::write(&latest, &text)?;
    println!("\n{text}");
    println!("report written to {}", latest.display());
    Ok(())
}

fn warn_stale_paths(golden: &GoldenSet, repo_root: &std::path::Path) {
    for warning in golden.check_paths(repo_root) {
        eprintln!("[WARN] golden label stale: {warning}");
    }
}
