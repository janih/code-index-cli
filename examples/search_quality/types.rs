//! Shared types for the search-quality benchmark harness.

use serde::{Deserialize, Serialize};

/// Per-model run output, persisted to `bench/results/<corpus>/<slug>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResults {
    #[serde(default)]
    pub corpus: String,
    pub name: String,
    pub slug: String,
    pub gguf: String,
    pub dimension: usize,
    pub n_ctx: Option<u64>,
    /// Query instruction prefix prepended by the harness (None = raw query).
    pub query_prefix: Option<String>,
    pub index: IndexStats,
    pub queries: Vec<QueryResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub files_processed: usize,
    pub files_skipped: usize,
    pub blocks: usize,
    pub index_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub qid: String,
    /// embed + search wall time in milliseconds (warm-up excluded).
    pub latency_ms: f64,
    pub hits: Vec<Hit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    /// 1-based rank in the raw result list.
    pub rank: usize,
    /// Repo-relative file path.
    pub path: String,
    pub score: f32,
    pub start_line: u64,
    pub end_line: u64,
}
