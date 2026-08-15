//! `bench/models.json` — the model registry + llama-server location.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    /// Path to the llama-server binary. Optional and machine-specific:
    /// when absent here and in `models.local.json`, `llama-server` is
    /// looked up on `$PATH`. Keep the committed file portable.
    #[serde(default)]
    pub llama_server: Option<String>,
    /// Directory bare GGUF filenames in `models[].gguf` resolve against.
    /// Machine-specific: configure via `bench/models.local.json`.
    /// Absolute gguf paths are used as-is.
    #[serde(default)]
    pub models_dir: Option<PathBuf>,
    #[serde(default)]
    pub corpora: Vec<CorpusSpec>,
    #[serde(default)]
    pub models: Vec<ModelSpec>,
}

/// Machine-local overrides, `bench/models.local.json` (gitignored):
/// `llamaServer` and `modelsDir` for this user's machine.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConfig {
    pub llama_server: Option<String>,
    pub models_dir: Option<PathBuf>,
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
    /// Resolved llama-server path: configured value, else `$PATH` lookup.
    /// Called at run time (not load time) so `analyze`/`clean` work
    /// without any machine config.
    pub fn resolve_llama_server(&self) -> anyhow::Result<PathBuf> {
        if let Some(configured) = self.llama_server.as_deref() {
            if !configured.trim().is_empty() {
                return Ok(PathBuf::from(configured));
            }
        }
        find_on_path("llama-server").ok_or_else(|| {
            anyhow::anyhow!(
                "llama-server not found on $PATH.\n\
                 Set \"llamaServer\" (and \"modelsDir\" for your GGUFs) in \
                 bench/models.local.json — see bench/README.md.\
                 "
            )
        })
    }

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
    let mut config: BenchConfig = serde_json::from_str(&raw)
        .map_err(|err| anyhow::anyhow!("invalid {}: {err}", path.display()))?;

    // Machine-local overlay (gitignored): llamaServer + modelsDir.
    let local_path = bench_dir.join("models.local.json");
    if local_path.exists() {
        let local_raw = std::fs::read_to_string(&local_path)
            .map_err(|err| anyhow::anyhow!("cannot read {}: {err}", local_path.display()))?;
        let local: LocalConfig = serde_json::from_str(&local_raw)
            .map_err(|err| anyhow::anyhow!("invalid {}: {err}", local_path.display()))?;
        if local.llama_server.is_some() {
            config.llama_server = local.llama_server;
        }
        if local.models_dir.is_some() {
            config.models_dir = local.models_dir;
        }
    }

    // Resolve bare GGUF filenames against modelsDir; absolute paths pass
    // through. Missing files are tolerated here and reported per model at
    // run time with an actionable message.
    if let Some(dir) = &config.models_dir {
        for model in &mut config.models {
            let gguf = Path::new(&model.gguf);
            if !gguf.is_absolute() {
                model.gguf = dir.join(gguf).to_string_lossy().into_owned();
            }
        }
    }

    if config.models.is_empty() {
        anyhow::bail!("{} lists no models", path.display());
    }
    Ok(config)
}

/// First executable-looking `name` found on `$PATH`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
