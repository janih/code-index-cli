//! Google Gemini embedder (`:batchEmbedContents`, `x-goog-api-key`).
//!
//! Port of `src/embedders/gemini.ts`. Processes texts sequentially — same as
//! the TS version (one network round-trip per text).

use async_trait::async_trait;

use crate::shared::embedding_models::{get_model_query_prefix, EmbedderProvider};
use crate::traits::{Embedder, EmbedderInfo, EmbeddingResponse, EmbeddingUsage, ValidationResult};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_MODEL: &str = "gemini-embedding-001";

/// Google Gemini implementation of the embedder interface.
pub struct GeminiEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model_id: String,
}

impl GeminiEmbedder {
    pub fn new(api_key: String, model_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key,
            default_model_id: model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        }
    }

    /// Single-text batchEmbedContents call (TS calls the batch endpoint with
    /// one request entry per text, sequentially).
    async fn embed_one(&self, model: &str, text: &str) -> anyhow::Result<Vec<f32>> {
        let url = format!("{}/models/{}:batchEmbedContents", self.base_url, model);
        let response = self
            .client
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&serde_json::json!({
                "requests": [{
                    "model": format!("models/{model}"),
                    "content": { "parts": [{ "text": text }] },
                }],
            }))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API error ({}): {}", status, body);
        }

        let body: serde_json::Value = response.json().await?;
        Ok(body["embeddings"]
            .get(0)
            .and_then(|e| e["values"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[async_trait]
impl Embedder for GeminiEmbedder {
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse> {
        let model_to_use = model.unwrap_or(&self.default_model_id).to_string();
        let query_prefix = if is_query {
            get_model_query_prefix(EmbedderProvider::Gemini, &model_to_use)
        } else {
            None
        };

        let mut all_embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            // Gemini processes one text at a time (TS loop)
            let processed = match query_prefix {
                Some(prefix) if !text.starts_with(prefix) => format!("{prefix}{text}"),
                _ => text.clone(),
            };
            let embedding = self.embed_one(&model_to_use, &processed).await?;
            all_embeddings.push(embedding);
        }

        Ok(EmbeddingResponse {
            embeddings: all_embeddings,
            // TS reports zeroed usage for Gemini
            usage: Some(EmbeddingUsage {
                prompt_tokens: 0,
                total_tokens: 0,
            }),
        })
    }

    async fn validate_configuration(&self) -> ValidationResult {
        let result = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("x-goog-api-key", &self.api_key)
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => ValidationResult::ok(),
            Ok(response) => ValidationResult::invalid(format!(
                "Gemini API returned status {}",
                response.status()
            )),
            Err(err) => ValidationResult::invalid(err.to_string()),
        }
    }

    fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: EmbedderProvider::Gemini,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts() {
        let embedder = GeminiEmbedder::new("k".to_string(), None);
        assert_eq!(embedder.default_model_id, "gemini-embedding-001");
        assert_eq!(embedder.base_url, DEFAULT_BASE_URL);
    }
}
