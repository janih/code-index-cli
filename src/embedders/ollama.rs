//! Ollama embedder (`/api/embed`, `/api/tags`).
//!
//! Port of `src/embedders/ollama.ts`.

use async_trait::async_trait;

use crate::shared::embedding_models::EmbedderProvider;
use crate::traits::{Embedder, EmbedderInfo, EmbeddingResponse, EmbeddingUsage, ValidationResult};

use super::openai::apply_query_prefix;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "nomic-embed-text:latest";
const EMBEDDING_TIMEOUT_SECS: u64 = 60;
const VALIDATION_TIMEOUT_SECS: u64 = 30;

/// Ollama implementation of the embedder interface.
pub struct OllamaEmbedder {
    client: reqwest::Client,
    base_url: String,
    default_model_id: String,
}

impl OllamaEmbedder {
    pub fn new(base_url: Option<String>, model_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            default_model_id: model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        }
    }

    async fn embed_call(&self, model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&serde_json::json!({ "model": model, "input": texts }))
            .timeout(std::time::Duration::from_secs(EMBEDDING_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() {
                    anyhow::anyhow!("Ollama embedding request timed out")
                } else {
                    anyhow::Error::from(err)
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama request failed ({}): {}", status, body);
        }

        let body: serde_json::Value = response.json().await?;
        Ok(body["embeddings"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|values| {
                values
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|n| n.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect())
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse> {
        let model_to_use = model.unwrap_or(&self.default_model_id).to_string();
        let processed =
            apply_query_prefix(EmbedderProvider::Ollama, &model_to_use, texts, is_query);
        let embeddings = self.embed_call(&model_to_use, &processed).await?;
        Ok(EmbeddingResponse {
            embeddings,
            usage: Some(EmbeddingUsage {
                prompt_tokens: 0,
                total_tokens: 0,
            }),
        })
    }

    async fn validate_configuration(&self) -> ValidationResult {
        let result = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(std::time::Duration::from_secs(VALIDATION_TIMEOUT_SECS))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => ValidationResult::ok(),
            Ok(response) => {
                ValidationResult::invalid(format!("Ollama returned status {}", response.status()))
            }
            Err(err) => ValidationResult::invalid(format!(
                "Cannot connect to Ollama at {}: {}",
                self.base_url, err
            )),
        }
    }

    fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: EmbedderProvider::Ollama,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts() {
        let embedder = OllamaEmbedder::new(None, None);
        assert_eq!(embedder.base_url, "http://localhost:11434");
        assert_eq!(embedder.default_model_id, "nomic-embed-text:latest");
    }

    #[test]
    fn base_url_trailing_slashes_trimmed() {
        let embedder = OllamaEmbedder::new(Some("http://ollama:11434//".to_string()), None);
        assert_eq!(embedder.base_url, "http://ollama:11434");
    }
}
