//! Markdown report generation for the search-quality benchmark.

use std::collections::HashMap;
use std::path::Path;

use crate::golden::GoldenSet;
use crate::metrics::{per_query_recall_at_10, ModelMetrics, TOP_K};
use crate::types::ModelResults;

/// (point estimate, lo, hi) of a paired bootstrap delta.
pub type Delta = (f64, f64, f64);

pub fn render(
    corpus_name: &str,
    metrics: &[ModelMetrics],
    results: &[ModelResults],
    golden: &GoldenSet,
    bootstrap: &[(String, Option<Delta>)],
) -> String {
    let mut out = String::new();
    out.push_str("# Search-quality benchmark\n\n");
    out.push_str(&format!(
        "- Corpus: **{corpus_name}** ({} golden queries: {} eval / {} dev-threshold)\n",
        golden.queries.len(),
        golden.counts().0,
        golden.counts().1,
    ));
    out.push_str("- Search: cosine, threshold −1.0, top-50 raw; metrics computed file-level (deduped by best rank), eval split only\n");
    out.push_str("- Thresholds: selected on the dev split, F1 evaluated on eval; `@0.40` is the shipped default for comparison\n\n");

    out.push_str("## Index sanity (must match across models)\n\n");
    out.push_str(
        "| Model | Files | Blocks | Index time (s) | dim | n_ctx |\n|---|---|---|---|---|---|\n",
    );
    for m in metrics {
        let ctx = results
            .iter()
            .find(|r| r.slug == name_slug(metrics, &m.name))
            .and_then(|r| r.n_ctx)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "—".into());
        out.push_str(&format!(
            "| {} | {} | {} | {:.1} | {} | {} |\n",
            m.name, m.files_processed, m.blocks, m.index_secs, m.dimension, ctx
        ));
    }
    let blocks: Vec<usize> = metrics.iter().map(|m| m.blocks).collect();
    if blocks.iter().max() != blocks.iter().min() {
        out.push_str("\n> ⚠️ block counts differ across models — chunking is deterministic, so a run is broken; do not compare.\n");
    }
    out.push('\n');

    out.push_str("## Retrieval quality\n\n");
    out.push_str("| Model | R@1 | R@5 | R@10 | MRR@10 | nDCG@10 | ROC-AUC | gap | thr(dev) | F1@thr | F1@0.40 | p50 ms | p95 ms |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for m in metrics {
        out.push_str(&format!(
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {} | {} | {} | {:.2} | {:.0} | {:.0} |\n",
            m.name,
            m.recall_at_1,
            m.recall_at_5,
            m.recall_at_10,
            m.mrr_at_10,
            m.ndcg_at_10,
            opt3(m.roc_auc),
            opt3(m.score_gap),
            m.best_threshold
                .map(|(t, _)| format!("{t:.2}"))
                .unwrap_or_else(|| "—".into()),
            opt3(m.eval_f1_at_recommended),
            m.eval_f1_at_default,
            m.latency_p50_ms,
            m.latency_p95_ms,
        ));
    }
    out.push_str("\nRaw cosine scores are NOT comparable across models — compare models via rank metrics, AUC and gap.\n");
    out.push_str("Query-prefixed variants (`+q`) embed queries with the model's instruction template; documents are always embedded raw, matching the shipped CLI.\n\n");

    if !bootstrap.is_empty() {
        out.push_str(&format!(
            "## Paired Δ recall@{} vs baseline (bootstrap 95% CI)\n\n",
            TOP_K
        ));
        for (name, delta) in bootstrap {
            match delta {
                Some((point, lo, hi)) => out.push_str(&format!(
                    "- {name}: {point:+.3} [{lo:+.3}, {hi:+.3}] {}\n",
                    if *lo > 0.0 {
                        "(significant +)"
                    } else if *hi < 0.0 {
                        "(significant −)"
                    } else {
                        ""
                    }
                )),
                None => out.push_str(&format!("- {name}: not enough paired queries\n")),
            }
        }
        out.push('\n');
    }

    out.push_str("\n## --directory-filtered queries\n\n");
    if metrics.iter().any(|m| m.dir_queries > 0) {
        out.push_str(&format!(
            "Searched with a golden directory prefix (server filter + client post-filter, product path). {} queries.\n\n",
            metrics.first().map(|m| m.dir_queries).unwrap_or(0)
        ));
        out.push_str("| Model | R@1 | R@5 | MRR@10 |\n|---|---|---|---|\n");
        for m in metrics {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                m.name,
                opt3(m.dir_recall_at_1),
                opt3(m.dir_recall_at_5),
                opt3(m.dir_mrr),
            ));
        }
    } else {
        out.push_str("None in this golden set.\n");
    }
    out.push('\n');

    out.push_str("## Recall@10 by category\n\n");
    let mut categories: Vec<String> = metrics
        .first()
        .map(|m| {
            m.per_category_recall_at_10
                .iter()
                .map(|(c, _, _)| c.clone())
                .collect()
        })
        .unwrap_or_default();
    categories.sort();
    if !categories.is_empty() {
        out.push_str("| Category |");
        for m in metrics {
            out.push_str(&format!(" {} |", m.name));
        }
        out.push_str("\n|---|");
        for _ in metrics {
            out.push_str("---|");
        }
        out.push('\n');
        for cat in categories {
            out.push_str(&format!("| {cat} |"));
            for m in metrics {
                let v = m
                    .per_category_recall_at_10
                    .iter()
                    .find(|(c, _, _)| *c == cat)
                    .map(|(_, v, _)| *v);
                out.push_str(&format!(
                    " {} |",
                    v.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into())
                ));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## Weakest queries per model (bottom 3 by AP@10)\n\n");
    for m in metrics {
        out.push_str(&format!("### {}\n\n", m.name));
        for (qid, query, expected, got) in &m.worst_queries {
            out.push_str(&format!(
                "- `{qid}` “{query}”\n  - expected: `{expected}`\n  - top-1: `{got}`\n"
            ));
        }
        out.push('\n');
    }
    out
}

fn name_slug(metrics: &[ModelMetrics], name: &str) -> String {
    metrics
        .iter()
        .find(|m| m.name == name)
        .map(|_| name.to_lowercase().replace([' ', '.'], "-"))
        .unwrap_or_default()
}

fn opt3(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.3}")).unwrap_or_else(|| "—".into())
}

/// Loads `bench/results/<corpus>/*.json` for the given slugs (all files when empty).
pub fn load_results(
    bench_dir: &Path,
    corpus_slug: &str,
    slugs: &[String],
) -> anyhow::Result<Vec<ModelResults>> {
    let dir = bench_dir.join("results").join(corpus_slug);
    let mut out = Vec::new();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for path in paths {
        if !slugs.is_empty() {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if !slugs.iter().any(|s| s == stem) {
                continue;
            }
        }
        let raw = std::fs::read_to_string(&path)?;
        out.push(serde_json::from_str(&raw)?);
    }
    if out.is_empty() {
        anyhow::bail!(
            "no results in {} — run the `run` subcommand first",
            dir.display()
        );
    }
    Ok(out)
}

pub fn paired_recall(
    results: &[ModelResults],
    golden: &GoldenSet,
) -> Vec<(String, HashMap<String, bool>)> {
    results
        .iter()
        .map(|r| (r.name.clone(), per_query_recall_at_10(r, golden)))
        .collect()
}
