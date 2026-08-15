//! Golden set loading and validation (`bench/golden/*.jsonl`).
//!
//! Format (one JSON object per line):
//! `{"id":"q01","query":"...","category":"config",
//!   "relevant":{"src/config/loader.rs":2}}`
//!
//! Grades: 2 = the file that answers the query, 1 = partial/supporting.
//! Paths are repo-relative (matching what the runner stores in results).

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GoldenQuery {
    pub id: String,
    pub query: String,
    pub category: String,
    pub relevant: HashMap<String, u8>,
    /// "eval" (default) or "dev". Dev queries tune thresholds;
    /// headline metrics report the eval split only.
    #[serde(default = "default_split")]
    pub split: String,
}

fn default_split() -> String {
    "eval".to_string()
}

#[derive(Debug, Clone)]
pub struct GoldenSet {
    pub queries: Vec<GoldenQuery>,
}

impl GoldenQuery {
    /// Highest relevance grade for a repo-relative path (0 = irrelevant).
    pub fn grade(&self, path: &str) -> u8 {
        self.relevant.get(path).copied().unwrap_or(0)
    }
}

impl GoldenSet {
    pub fn load(bench_dir: &Path) -> anyhow::Result<Self> {
        let golden_dir = bench_dir.join("golden");
        let mut queries = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&golden_dir)
            .map_err(|err| anyhow::anyhow!("cannot read {}: {err}", golden_dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        entries.sort();

        for path in entries {
            let raw = std::fs::read_to_string(&path)?;
            for (lineno, line) in raw.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let q: GoldenQuery = serde_json::from_str(line).map_err(|err| {
                    anyhow::anyhow!(
                        "{}:{}: invalid golden entry: {err}",
                        path.display(),
                        lineno + 1
                    )
                })?;
                if q.query.trim().is_empty() || q.relevant.is_empty() {
                    anyhow::bail!(
                        "{}:{}: query needs text and >=1 relevant file",
                        path.display(),
                        lineno + 1
                    );
                }
                queries.push(q);
            }
        }

        let mut seen = std::collections::HashSet::new();
        for q in &queries {
            if !seen.insert(q.id.as_str()) {
                anyhow::bail!("duplicate golden query id: {}", q.id);
            }
        }
        if queries.is_empty() {
            anyhow::bail!("golden set is empty");
        }
        Ok(Self { queries })
    }

    /// Warns (does not fail) when a labeled path no longer exists — labels
    /// can go stale after refactors.
    pub fn check_paths(&self, repo_root: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        for q in &self.queries {
            for path in q.relevant.keys() {
                if !repo_root.join(path).exists() {
                    warnings.push(format!("{} ({}): {} does not exist", q.id, q.query, path));
                }
            }
        }
        warnings
    }

    pub fn find(&self, qid: &str) -> Option<&GoldenQuery> {
        self.queries.iter().find(|q| q.id == qid)
    }

    /// (eval count, dev count).
    pub fn counts(&self) -> (usize, usize) {
        (
            self.queries.iter().filter(|q| q.split == "eval").count(),
            self.queries.iter().filter(|q| q.split == "dev").count(),
        )
    }

    pub fn has_dev(&self) -> bool {
        self.queries.iter().any(|q| q.split == "dev")
    }
}
