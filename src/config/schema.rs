//! Configuration schema and defaults.
//!
//! serde types + `Default` impls provide the fallbacks.

use serde::{Deserialize, Serialize};

use crate::shared::embedding_models::EmbedderProvider;

/// Root configuration (`.code-index.json` structure).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub enabled: bool,
    pub embedder: EmbedderConfig,
    pub qdrant: QdrantConfig,
    pub search: SearchConfig,
    pub indexing: IndexingConfig,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            embedder: EmbedderConfig::default(),
            qdrant: QdrantConfig::default(),
            search: SearchConfig::default(),
            indexing: IndexingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmbedderConfig {
    pub provider: EmbedderProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_dimension: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    // OpenAI Compatible specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_api_key: Option<String>,
    // Bedrock specific
    pub bedrock_region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bedrock_profile: Option<String>,
    // OpenRouter specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_router_provider: Option<String>,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            provider: EmbedderProvider::OpenAi,
            model_id: None,
            model_dimension: None,
            api_key: None,
            base_url: None,
            compatible_base_url: None,
            compatible_api_key: None,
            bedrock_region: "us-east-1".to_string(),
            bedrock_profile: None,
            open_router_provider: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QdrantConfig {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6333".to_string(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchConfig {
    pub min_score: f64,
    pub max_results: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            min_score: crate::shared::constants::DEFAULT_SEARCH_MIN_SCORE,
            max_results: crate::shared::constants::DEFAULT_SEARCH_RESULTS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IndexingConfig {
    pub batch_size: u32,
    /// Largest indexed file (default 1 MiB).
    pub max_file_size_bytes: u64,
    pub exclude_patterns: Vec<String>,
    /// Accepted for config compatibility but NOT wired into scanning in
    /// either version — the scanner/parser use the built-in extension list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_extensions: Option<Vec<String>>,
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            batch_size: 60,
            max_file_size_bytes: crate::shared::constants::MAX_FILE_SIZE_BYTES,
            exclude_patterns: Vec::new(),
            include_extensions: None,
        }
    }
}

impl CliConfig {
    /// Range validations that serde types alone cannot express (zod parity).
    ///
    /// Returns the list of `path: message` issues; empty if valid.
    pub fn validation_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if let Some(dimension) = self.embedder.model_dimension {
            if dimension == 0 {
                issues.push("embedder.modelDimension: must be a positive number".to_string());
            }
        }
        if !(0.0..=1.0).contains(&self.search.min_score) {
            issues.push("search.minScore: must be between 0 and 1".to_string());
        }
        if !(crate::shared::constants::MIN_SEARCH_RESULTS
            ..=crate::shared::constants::MAX_SEARCH_RESULTS)
            .contains(&self.search.max_results)
        {
            issues.push("search.maxResults: must be between 10 and 200".to_string());
        }
        if self.indexing.batch_size == 0 {
            issues.push("indexing.batchSize: must be a positive number".to_string());
        }
        if self.indexing.max_file_size_bytes == 0 {
            issues.push("indexing.maxFileSizeBytes: must be a positive number".to_string());
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts_default_config() {
        let config = CliConfig::default();
        assert!(config.enabled);
        assert_eq!(config.embedder.provider, EmbedderProvider::OpenAi);
        assert_eq!(config.embedder.bedrock_region, "us-east-1");
        assert_eq!(config.qdrant.url, "http://localhost:6333");
        assert_eq!(config.search.min_score, 0.4);
        assert_eq!(config.search.max_results, 50);
        assert_eq!(config.indexing.batch_size, 60);
        assert_eq!(config.indexing.max_file_size_bytes, 1024 * 1024);
        assert!(config.indexing.exclude_patterns.is_empty());
    }

    #[test]
    fn partial_json_fills_defaults() {
        let config: CliConfig = serde_json::from_str(
            r#"{"embedder": {"provider": "ollama", "baseUrl": "http://custom:11434"}}"#,
        )
        .expect("partial config parses");
        assert_eq!(config.embedder.provider, EmbedderProvider::Ollama);
        assert_eq!(
            config.embedder.base_url.as_deref(),
            Some("http://custom:11434")
        );
        assert_eq!(config.qdrant.url, "http://localhost:6333");
        assert_eq!(config.search.max_results, 50);
    }

    #[test]
    fn validation_reports_out_of_range_values() {
        let mut config = CliConfig::default();
        config.search.min_score = 1.5;
        config.search.max_results = 5;
        config.indexing.batch_size = 0;
        let issues = config.validation_issues();
        assert_eq!(issues.len(), 3);
        assert!(issues.iter().any(|i| i.starts_with("search.minScore")));
        assert!(issues.iter().any(|i| i.starts_with("search.maxResults")));
        assert!(issues.iter().any(|i| i.starts_with("indexing.batchSize")));
    }

    #[test]
    fn validation_accepts_defaults() {
        assert!(CliConfig::default().validation_issues().is_empty());
    }
}
