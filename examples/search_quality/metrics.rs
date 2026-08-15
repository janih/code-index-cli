//! Retrieval metrics computed from raw results + golden labels.
//!
//! All ranking metrics are file-level: hits are deduplicated by file (best
//! rank kept) because golden labels are per-file. Score-based metrics (gap,
//! AUC) operate on raw block scores — the only quantities comparable within
//! one model, never across models.

use std::collections::HashMap;

use serde::Serialize;

use crate::golden::GoldenSet;
use crate::types::{Hit, ModelResults};

pub const TOP_K: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct ModelMetrics {
    pub name: String,
    pub dimension: usize,
    pub recall_at_1: f64,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr_at_10: f64,
    pub ndcg_at_10: f64,
    pub roc_auc: Option<f64>,
    /// Mean (relevant − irrelevant) score among top-10 unique files.
    pub score_gap: Option<f64>,
    /// Threshold maximizing macro-F1 at file level.
    pub best_threshold: Option<(f64, f64)>, // (threshold, F1)
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub index_secs: f64,
    pub files_processed: usize,
    pub blocks: usize,
    pub per_category_recall_at_10: Vec<(String, f64, usize)>,
    /// (qid, query, expected, got-top1) for the weakest queries.
    pub worst_queries: Vec<(String, String, String, String)>,
}

/// Per-query binary recall@10, paired for bootstrap deltas by qid.
pub fn per_query_recall_at_10(results: &ModelResults, golden: &GoldenSet) -> HashMap<String, bool> {
    results
        .queries
        .iter()
        .filter_map(|q| golden.find(&q.qid).map(|g| (q, g)))
        .map(|(q, g)| {
            let files = file_ranking(&q.hits);
            let hit = files
                .iter()
                .take(TOP_K)
                .any(|(path, _, _)| g.grade(path) > 0);
            (q.qid.clone(), hit)
        })
        .collect()
}

pub fn compute(results: &ModelResults, golden: &GoldenSet) -> ModelMetrics {
    let mut recalls1 = Vec::new();
    let mut recalls5 = Vec::new();
    let mut recalls10 = Vec::new();
    let mut rrs = Vec::new();
    let mut ndcgs = Vec::new();
    let mut rel_scores = Vec::new();
    let mut irr_scores = Vec::new();
    let mut latencies = Vec::new();
    let mut aps: Vec<(String, f64)> = Vec::new();
    let mut category: Vec<(String, bool)> = Vec::new();
    let mut auc_pairs: Vec<(f64, bool)> = Vec::new();

    for q in &results.queries {
        let Some(g) = golden.find(&q.qid) else {
            continue;
        };
        latencies.push(q.latency_ms);

        let files = file_ranking(&q.hits);
        for (path, _, score) in &files {
            auc_pairs.push((*score as f64, g.grade(path) > 0));
        }

        let first_rel = files.iter().position(|(path, _, _)| g.grade(path) > 0);
        recalls1.push(first_rel.is_some_and(|i| i < 1));
        recalls5.push(first_rel.is_some_and(|i| i < 5));
        recalls10.push(first_rel.is_some_and(|i| i < TOP_K));
        rrs.push(
            first_rel
                .filter(|i| *i < TOP_K)
                .map(|i| 1.0 / (i + 1) as f64)
                .unwrap_or(0.0),
        );

        // nDCG@10 with graded gains (2^grade − 1)
        let mut dcg = 0.0;
        for (i, (path, _, _)) in files.iter().take(TOP_K).enumerate() {
            let grade = g.grade(path);
            if grade > 0 {
                dcg += (2f64.powi(grade as i32) - 1.0) / (i as f64 + 2.0).log2();
            }
        }
        let mut ideal: Vec<u8> = g.relevant.values().copied().collect();
        ideal.sort_unstable_by(|a, b| b.cmp(a));
        let idcg: f64 = ideal
            .iter()
            .take(TOP_K)
            .enumerate()
            .map(|(i, &grade)| (2f64.powi(grade as i32) - 1.0) / (i as f64 + 2.0).log2())
            .sum();
        ndcgs.push(if idcg > 0.0 { dcg / idcg } else { 0.0 });

        // AP@10 (binary, file level) for the worst-query listing
        let mut hits_seen = 0usize;
        let mut ap_sum = 0.0;
        for (i, (path, _, _)) in files.iter().take(TOP_K).enumerate() {
            if g.grade(path) > 0 {
                hits_seen += 1;
                ap_sum += hits_seen as f64 / (i + 1) as f64;
            }
        }
        let total_rel = g.relevant.len().min(TOP_K);
        aps.push((q.qid.clone(), ap_sum / total_rel.max(1) as f64));

        // Score gap among top-10 unique files
        for (path, _, score) in files.iter().take(TOP_K) {
            if g.grade(path) > 0 {
                rel_scores.push(*score as f64);
            } else {
                irr_scores.push(*score as f64);
            }
        }
        category.push((g.category.clone(), first_rel.is_some_and(|i| i < TOP_K)));
    }

    // Threshold sweep (file level): a file is retrieved when any of its
    // blocks scores ≥ t within the returned top-50.
    let mut best = None;
    for step in 1..=19 {
        let t = step as f64 * 0.05;
        let (precision, recall, f1) = sweep_at(results, golden, t);
        if f1 > 0.0 && best.is_none_or(|(_, bf1)| f1 > bf1) {
            best = Some((t, f1));
        }
        let _ = (precision, recall);
    }

    let mut cat_map: HashMap<String, Vec<bool>> = HashMap::new();
    for (cat, hit) in category {
        cat_map.entry(cat).or_default().push(hit);
    }
    let mut per_category: Vec<_> = cat_map
        .into_iter()
        .map(|(cat, v)| {
            let n = v.len();
            (cat, v.iter().filter(|b| **b).count() as f64 / n as f64, n)
        })
        .collect();
    per_category.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut sorted_aps = aps;
    sorted_aps.sort_by(|a, b| a.1.total_cmp(&b.1));
    let worst = sorted_aps
        .iter()
        .take(3)
        .filter_map(|(qid, _)| {
            let q = &results.queries.iter().find(|q| &q.qid == qid)?;
            let g = golden.find(qid)?;
            let files = file_ranking(&q.hits);
            let expected = g
                .relevant
                .iter()
                .max_by_key(|(_, &grade)| grade)
                .map(|(p, _)| p.clone())
                .unwrap_or_default();
            let got = files
                .first()
                .map(|(p, _, _)| p.clone())
                .unwrap_or_else(|| "(nothing)".into());
            Some((qid.clone(), g.query.clone(), expected, got))
        })
        .collect();

    let mut latencies_sorted = latencies.clone();
    latencies_sorted.sort_by(|a, b| a.total_cmp(b));

    ModelMetrics {
        name: results.name.clone(),
        dimension: results.dimension,
        recall_at_1: mean0(&bools(&recalls1)),
        recall_at_5: mean0(&bools(&recalls5)),
        recall_at_10: mean0(&bools(&recalls10)),
        mrr_at_10: mean0(&rrs),
        ndcg_at_10: mean0(&ndcgs),
        roc_auc: roc_auc(&auc_pairs),
        score_gap: match (mean(&rel_scores), mean(&irr_scores)) {
            (Some(r), Some(i)) => Some(r - i),
            _ => None,
        },
        best_threshold: best,
        latency_p50_ms: percentile(&latencies_sorted, 50.0),
        latency_p95_ms: percentile(&latencies_sorted, 95.0),
        index_secs: results.index.index_secs,
        files_processed: results.index.files_processed,
        blocks: results.index.blocks,
        per_category_recall_at_10: per_category,
        worst_queries: worst,
    }
}

/// File-level ranking: unique files in order of best (earliest) hit.
fn file_ranking(hits: &[Hit]) -> Vec<(String, usize, f32)> {
    let mut seen = std::collections::HashSet::new();
    let mut ranking = Vec::new();
    for hit in hits {
        if seen.insert(hit.path.clone()) {
            ranking.push((hit.path.clone(), hit.rank, hit.score));
        }
    }
    ranking
}

fn sweep_at(results: &ModelResults, golden: &GoldenSet, t: f64) -> (f64, f64, f64) {
    let mut precisions = Vec::new();
    let mut recalls = Vec::new();
    for q in &results.queries {
        let Some(g) = golden.find(&q.qid) else {
            continue;
        };
        let mut retrieved: Vec<&str> = Vec::new();
        for hit in &q.hits {
            if (hit.score as f64) >= t && !retrieved.contains(&hit.path.as_str()) {
                retrieved.push(&hit.path);
            }
        }
        let relevant: Vec<&str> = g.relevant.keys().map(|s| s.as_str()).collect();
        let inter = retrieved.iter().filter(|p| relevant.contains(p)).count();
        let precision = if retrieved.is_empty() {
            0.0
        } else {
            inter as f64 / retrieved.len() as f64
        };
        let recall = inter as f64 / relevant.len().max(1) as f64;
        precisions.push(precision);
        recalls.push(recall);
    }
    let p = mean(&precisions).unwrap_or(0.0);
    let r = mean(&recalls).unwrap_or(0.0);
    let f1 = if p + r > 0.0 {
        2.0 * p * r / (p + r)
    } else {
        0.0
    };
    (p, r, f1)
}

/// Paired bootstrap CI for the recall@10 delta vs baseline.
pub fn bootstrap_delta(
    model: &HashMap<String, bool>,
    baseline: &HashMap<String, bool>,
) -> Option<(f64, f64, f64)> {
    let ids: Vec<&String> = model
        .keys()
        .filter(|id| baseline.contains_key(*id))
        .collect();
    if ids.len() < 5 {
        return None;
    }
    let m: Vec<f64> = ids.iter().map(|id| model[*id] as u8 as f64).collect();
    let b: Vec<f64> = ids.iter().map(|id| baseline[*id] as u8 as f64).collect();
    let point = mean(&m).unwrap_or(0.0) - mean(&b).unwrap_or(0.0);

    let mut rng: u64 = 0x9E3779B97F4A7C15;
    let mut deltas = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let (mut dm, mut db) = (0.0f64, 0.0f64);
        for _ in 0..ids.len() {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let idx = ((rng >> 33) as usize) % ids.len();
            dm += m[idx];
            db += b[idx];
        }
        deltas.push(dm / ids.len() as f64 - db / ids.len() as f64);
    }
    deltas.sort_by(|a, b| a.total_cmp(b));
    let lo = percentile(&deltas, 2.5);
    let hi = percentile(&deltas, 97.5);
    Some((point, lo, hi))
}

/// Rank-based ROC-AUC (Mann-Whitney) over pooled (score, relevant?) pairs.
fn roc_auc(pairs: &[(f64, bool)]) -> Option<f64> {
    let n_pos = pairs.iter().filter(|(_, b)| *b).count();
    let n_neg = pairs.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return None;
    }
    let mut sorted: Vec<(f64, bool)> = pairs.to_vec();
    // ascending: standard Mann-Whitney — AUC = P(score_pos > score_neg)
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    // average ranks for ties
    let mut rank_sum_pos = 0.0f64;
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j < sorted.len() && sorted[j].0 == sorted[i].0 {
            j += 1;
        }
        let avg_rank = (i + 1 + j) as f64 / 2.0;
        for item in &sorted[i..j] {
            if item.1 {
                rank_sum_pos += avg_rank;
            }
        }
        i = j;
    }
    let u = rank_sum_pos - n_pos as f64 * (n_pos as f64 + 1.0) / 2.0;
    Some(u / (n_pos as f64 * n_neg as f64))
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(mean0(values))
    }
}

fn mean0(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn bools(v: &[bool]) -> Vec<f64> {
    v.iter().map(|b| *b as u8 as f64).collect()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
