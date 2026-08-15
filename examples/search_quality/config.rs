//! `bench/models.json` — the model registry + llama-server location.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    /// Path to the llama-server binary.
    pub llama_server: String,
    /// Indexed corpora (default: this repo only).
    #[serde(default)]
    pub corpora: Vec<CorpusSpec>,
    #[serde(default)]
    pub models: Vec<ModelSpec>,
}

/// One benchmark corpus: a directory of source code plus its golden set
/// (`bench/golden/<corpus-slug>.jsonl`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSpec {
    pub name: String,
    /// Corpus root, relative to the harness repo ("." = the repo itself).
    pub root: String,
    /// Extra gitignore-style excludes on top of the corpus's own .gitignore
    /// (e.g. `bench/` for the self corpus so golden files cannot self-match).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// When set, the corpus root is created as a `git worktree` of this
    /// branch if it does not exist yet.
    #[serde(default)]
    pub worktree_branch: Option<String>,
}

impl CorpusSpec {
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }
}

impl BenchConfig {
    /// Corpora with the implicit self-corpus default applied.
    pub fn effective_corpora(&self) -> Vec<CorpusSpec> {
        if self.corpora.is_empty() {
            vec![CorpusSpec {
                name: "code-index-cli".to_string(),
                root: ".".to_string(),
                exclude: vec!["bench/".to_string(), "/.code-index.json".to_string()],
                worktree_branch: None,
            }]
        } else {
            self.corpora.clone()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    /// Human-readable name (also the `--model` filter key).
    pub name: String,
    /// Absolute path to the GGUF file.
    pub gguf: String,
    /// Optional instruction prefix prepended to queries (model-card
    /// dependent). M1 runs set none, so every model embeds the raw query —
    /// the same condition as the shipped CLI.
    #[serde(default)]
    pub query_prefix: Option<String>,
    #[serde(default)]
    pub baseline: bool,
    /// Fixed dimension override; auto-detected from the server's
    /// `/v1/models` metadata when absent.
    #[serde(default)]
    pub dimension: Option<usize>,
    /// Model id sent to the embedder + used for product profile lookup.
    /// Defaults to the server-reported id (the GGUF path). Set this to the
    /// canonical model name (e.g. `Qwen3-Embedding-0.6B`) so product-side
    /// query-prefix profiles activate exactly as they would for a user
    /// configuring that modelId.
    #[serde(default)]
    pub model_id: Option<String>,
}

impl ModelSpec {
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }
}

pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .replace("--", "-")
}

pub fn load(bench_dir: &Path) -> anyhow::Result<BenchConfig> {
    let path = bench_dir.join("models.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|err| anyhow::anyhow!("cannot read {}: {err}", path.display()))?;
    let config: BenchConfig = serde_json::from_str(&raw)
        .map_err(|err| anyhow::anyhow!("invalid {}: {err}", path.display()))?;
    if config.models.is_empty() {
        anyhow::bail!("{} lists no models", path.display());
    }
    Ok(config)
}
