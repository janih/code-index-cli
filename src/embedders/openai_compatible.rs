//! OpenAI-compatible embedder (custom base URL, e.g. llama-server, vLLM,
//! LiteLLM, LM Studio).
//!
//! Port of `src/embedders/openai-compatible.ts`. Key differences from the
//! bare OpenAI provider: no internal batching (one request per call),
//! query prefix applied without the per-item token-limit check, and
//! validation probes a minimal embedding request instead of GET /models.

use async_trait::async_trait;

use crate::shared::embedding_models::{get_model_query_prefix, EmbedderProvider};
use crate::traits::{Embedder, EmbedderInfo, EmbeddingResponse, ValidationResult};

use super::openai::{call_embeddings_with_retry, EmbeddingBatchResponse};

const DEFAULT_MODEL: &str = "text-embedding-3-small";

/// OpenAI-compatible implementation of the embedder interface.
pub struct OpenAiCompatibleEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model_id: String,
}

impl OpenAiCompatibleEmbedder {
    pub fn new(base_url: String, api_key: String, model_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model_id: model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        }
    }

    /// Sends the whole input in a single request (no batching in TS either).
    async fn embeddings_call(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<EmbeddingBatchResponse, (Option<u16>, String)> {
        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model, "input": texts }))
            .send()
            .await
            .map_err(|err| (None, err.to_string()))?;

        let status = response.status().as_u16();
        if status == 429 || status >= 500 {
            let body = response.text().await.unwrap_or_default();
            return Err((Some(status), body));
        }
        if !(200..300).contains(&status) {
            let body = response.text().await.unwrap_or_default();
            return Err((
                Some(status),
                format!("OpenAI Compatible API error ({status}): {body}"),
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|err| (Some(status), format!("Invalid response body: {err}")))?;
        Ok(super::openai::parse_openai_response(&body))
    }
}

#[async_trait]
impl Embedder for OpenAiCompatibleEmbedder {
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse> {
        let model_to_use = model.unwrap_or(&self.default_model_id).to_string();

        // TS applies the prefix without the MAX_ITEM_TOKENS guard here
        let processed_texts = if is_query {
            match get_model_query_prefix(EmbedderProvider::OpenAiCompatible, &model_to_use) {
                Some(prefix) => texts
                    .iter()
                    .map(|text| {
                        if text.starts_with(prefix) {
                            text.clone()
                        } else {
                            format!("{prefix}{text}")
                        }
                    })
                    .collect(),
                None => texts.to_vec(),
            }
        } else {
            texts.to_vec()
        };

        let response =
            call_embeddings_with_retry(|| self.embeddings_call(&model_to_use, &processed_texts))
                .await?;
        Ok(EmbeddingResponse {
            embeddings: response.embeddings,
            usage: response.usage,
        })
    }

    async fn validate_configuration(&self) -> ValidationResult {
        // Try a minimal embedding request
        match self
            .embeddings_call(&self.default_model_id, &["test".to_string()])
            .await
        {
            Ok(_) => ValidationResult::ok(),
            Err((_, message)) => ValidationResult::invalid(message),
        }
    }

    fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: EmbedderProvider::OpenAiCompatible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_trailing_slashes_are_trimmed() {
        let embedder = OpenAiCompatibleEmbedder::new(
            "http://localhost:8089/v1/".to_string(),
            "test".to_string(),
            None,
        );
        assert_eq!(embedder.base_url, "http://localhost:8089/v1");
    }

    #[test]
    fn default_model_matches_ts() {
        let embedder = OpenAiCompatibleEmbedder::new("http://x".to_string(), "k".to_string(), None);
        assert_eq!(embedder.default_model_id, "text-embedding-3-small");
    }
}
