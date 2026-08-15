//! `bench/models.json` — the model registry + llama-server location.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    /// Path to the llama-server binary.
    pub llama_server: String,
    #[serde(default)]
    pub models: Vec<ModelSpec>,
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
}

impl ModelSpec {
    pub fn slug(&self) -> String {
        self.name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .replace("--", "-")
    }
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
