//! Search-quality benchmark harness (M1–M4).
//!
//! Compares local embedding models on codebase search over one or more
//! corpora: golden queries with graded file-level labels → index + search
//! per model via llama-server rotation → recall/MRR/nDCG/AUC/threshold/
//! --directory-slice report.
//!
//! Usage:
//!   cargo run --release --example search_quality -- run [--corpus NAME].. [--model NAME].. [--port 8099] [--keep]
//!   cargo run --release --example search_quality -- analyze [--corpus NAME].. [--baseline NAME]
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
use config::{BenchConfig, CorpusSpec};
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
        /// Restrict to these corpora (repeatable). Default: all.
        #[arg(long)]
        corpus: Vec<String>,
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
        /// Restrict to these corpora (repeatable). Default: all.
        #[arg(long)]
        corpus: Vec<String>,
        /// Baseline model for paired bootstrap deltas (default: models.json flag).
        #[arg(long)]
        baseline: Option<String>,
    },
    /// Delete collections, cache dirs and symlinks for all corpora/models.
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
            for corpus in config.effective_corpora() {
                for spec in &config.models {
                    runner::cleanup_model(&paths, &corpus, spec, &cli.qdrant_url).await;
                }
            }
            println!("cleaned collections, caches and symlinks (corpora × models)");
        }
        Command::Run {
            corpus,
            model,
            port,
            keep,
        } => {
            let config = config::load(&bench_dir)?;
            let selected_models: Vec<_> = config
                .models
                .iter()
                .filter(|m| model.is_empty() || model.contains(&m.name))
                .collect();
            if selected_models.is_empty() {
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
            let corpora = select_corpora(&config, &corpus)?;
            std::fs::create_dir_all(bench_dir.join("tmp"))?;

            for corpus_spec in &corpora {
                let golden = GoldenSet::load(&bench_dir, &corpus_spec.slug())?;
                let root = runner::corpus_root(&repo_root, corpus_spec)?;
                for warning in golden.check_paths(&root) {
                    eprintln!("[WARN] golden label stale: {warning}");
                }
                println!(
                    "\n########## corpus {} ({}) — {} queries ##########",
                    corpus_spec.name,
                    root.display(),
                    golden.queries.len()
                );

                for spec in &selected_models {
                    let slug = spec.slug();
                    let gguf = PathBuf::from(&spec.gguf);
                    if !gguf.exists() {
                        eprintln!("[{slug}] GGUF not found: {} — skipping", spec.gguf);
                        continue;
                    }
                    let log = bench_dir
                        .join("tmp")
                        .join(format!("{}-{slug}-server.log", corpus_spec.slug()));
                    let (mut llama, meta) =
                        match server::LlamaServer::start(&config.llama_server, &gguf, port, &log)
                            .await
                        {
                            Ok(v) => v,
                            Err(err) => {
                                eprintln!("[{slug}] {err:#}");
                                continue;
                            }
                        };
                    let result = runner::run_model(
                        &paths,
                        corpus_spec,
                        &root,
                        spec,
                        &meta,
                        port,
                        &golden,
                        &cli.qdrant_url,
                    )
                    .await;
                    llama.kill();
                    if let Err(err) = result {
                        eprintln!("[{slug}] {err:#}");
                    }
                    if !keep {
                        runner::cleanup_model(&paths, corpus_spec, spec, &cli.qdrant_url).await;
                    }
                }
            }

            for corpus_spec in &corpora {
                analyze(&bench_dir, &repo_root, corpus_spec, &config, None)?;
            }
        }
        Command::Analyze { corpus, baseline } => {
            let config = config::load(&bench_dir)?;
            for corpus_spec in select_corpora(&config, &corpus)? {
                analyze(
                    &bench_dir,
                    &repo_root,
                    &corpus_spec,
                    &config,
                    baseline.clone(),
                )?;
            }
        }
    }
    Ok(())
}

fn select_corpora(config: &BenchConfig, requested: &[String]) -> anyhow::Result<Vec<CorpusSpec>> {
    let all = config.effective_corpora();
    if requested.is_empty() {
        return Ok(all);
    }
    let mut out = Vec::new();
    for name in requested {
        let Some(corpus) = all.iter().find(|c| &c.name == name) else {
            anyhow::bail!(
                "unknown corpus {name:?} (known: {:?})",
                all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()
            );
        };
        out.push(corpus.clone());
    }
    Ok(out)
}

fn analyze(
    bench_dir: &std::path::Path,
    repo_root: &std::path::Path,
    corpus: &CorpusSpec,
    config: &BenchConfig,
    baseline_override: Option<String>,
) -> anyhow::Result<()> {
    let corpus_slug = corpus.slug();
    let golden = GoldenSet::load(bench_dir, &corpus_slug)?;
    let results = report::load_results(bench_dir, &corpus_slug, &[])?;
    let all_metrics: Vec<_> = results
        .iter()
        .map(|r| metrics::compute(r, &golden))
        .collect();

    let baseline_name = baseline_override.or_else(|| {
        config
            .models
            .iter()
            .find(|m| m.baseline)
            .map(|m| m.name.clone())
    });
    let paired = report::paired_recall(&results, &golden);
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

    let text = report::render(&corpus.name, &all_metrics, &results, &golden, &bootstrap);
    let report_dir = bench_dir.join("reports");
    std::fs::create_dir_all(&report_dir)?;
    let file = report_dir.join(format!("{corpus_slug}.md"));
    std::fs::write(&file, &text)?;
    println!("\n{text}");
    println!(
        "report written to {} ({})",
        file.display(),
        repo_root.display()
    );
    Ok(())
}
