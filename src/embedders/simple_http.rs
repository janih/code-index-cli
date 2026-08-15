//! Single-request embedders sharing the OpenAI response shape: Mistral,
//! Vercel AI Gateway, OpenRouter.
//!
//! None of them batch or retry — a shared parameterized client keeps the
//! behavior without copy-pasting.

use async_trait::async_trait;

use crate::shared::embedding_models::EmbedderProvider;
use crate::shared::validation::sanitize_error_message;
use crate::traits::{Embedder, EmbedderInfo, EmbeddingResponse, ValidationResult};

use super::openai::parse_openai_response;

/// Configuration for one single-request provider.
pub struct SimpleHttpEmbedder {
    provider: EmbedderProvider,
    client: reqwest::Client,
    embeddings_url: String,
    api_key: String,
    default_model_id: String,
    error_label: &'static str,
    /// OpenRouter sets attribution headers only when a specific provider is
    /// configured.
    specific_provider: Option<String>,
}

impl SimpleHttpEmbedder {
    pub fn mistral(api_key: String, model_id: Option<String>) -> Self {
        Self::build(
            EmbedderProvider::Mistral,
            "https://api.mistral.ai/v1/embeddings",
            "codestral-embed-2505",
            "Mistral API",
            api_key,
            model_id,
            None,
        )
    }

    pub fn vercel_ai_gateway(api_key: String, model_id: Option<String>) -> Self {
        Self::build(
            EmbedderProvider::VercelAiGateway,
            "https://sdk.vercel.ai/api/embed",
            "openai/text-embedding-3-large",
            "Vercel AI Gateway",
            api_key,
            model_id,
            None,
        )
    }

    pub fn openrouter(
        api_key: String,
        model_id: Option<String>,
        specific_provider: Option<String>,
    ) -> Self {
        // Note the singular `embed` resource (not /embeddings).
        Self::build(
            EmbedderProvider::OpenRouter,
            "https://openrouter.ai/api/v1/embeddings",
            "openai/text-embedding-3-large",
            "OpenRouter API",
            api_key,
            model_id,
            specific_provider,
        )
    }

    fn build(
        provider: EmbedderProvider,
        base_url: &str,
        default_model: &str,
        error_label: &'static str,
        api_key: String,
        model_id: Option<String>,
        specific_provider: Option<String>,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            provider,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("embedding client with fixed timeout builds"),
            embeddings_url: base_url,
            api_key,
            default_model_id: model_id.unwrap_or_else(|| default_model.to_string()),
            error_label,
            specific_provider,
        }
    }

    fn request(&self, model: &str, texts: &[String]) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.embeddings_url)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model, "input": texts }));
        if self.specific_provider.is_some() {
            request = request
                .header("HTTP-Referer", "https://github.com/janih/code-index-cli")
                .header("X-Title", "Code Index CLI");
        }
        request
    }

    async fn call(&self, model: &str, texts: &[String]) -> anyhow::Result<EmbeddingResponse> {
        let response = self.request(model, texts).send().await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "{}",
                sanitize_error_message(&format!(
                    "{} error ({}): {}",
                    self.error_label, status, body
                ))
            );
        }

        let body: serde_json::Value = response.json().await.map_err(|err| {
            anyhow::anyhow!(
                "Invalid response body: {}",
                sanitize_error_message(&err.to_string())
            )
        })?;
        let parsed = parse_openai_response(&body);
        Ok(EmbeddingResponse {
            embeddings: parsed.embeddings,
            usage: parsed.usage,
        })
    }
}

#[async_trait]
impl Embedder for SimpleHttpEmbedder {
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        _is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse> {
        let model_to_use = model.unwrap_or(&self.default_model_id).to_string();
        self.call(&model_to_use, texts).await
    }

    async fn validate_configuration(&self) -> ValidationResult {
        match self
            .call(&self.default_model_id.clone(), &["test".to_string()])
            .await
        {
            Ok(_) => ValidationResult::ok(),
            Err(err) => ValidationResult::invalid(sanitize_error_message(&err.to_string())),
        }
    }

    fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: self.provider,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts() {
        assert_eq!(
            SimpleHttpEmbedder::mistral("k".into(), None).default_model_id,
            "codestral-embed-2505"
        );
        assert_eq!(
            SimpleHttpEmbedder::vercel_ai_gateway("k".into(), None).default_model_id,
            "openai/text-embedding-3-large"
        );
        assert_eq!(
            SimpleHttpEmbedder::openrouter("k".into(), None, None).default_model_id,
            "openai/text-embedding-3-large"
        );
        assert_eq!(
            SimpleHttpEmbedder::mistral("k".into(), None).embeddings_url,
            "https://api.mistral.ai/v1/embeddings"
        );
        assert_eq!(
            SimpleHttpEmbedder::vercel_ai_gateway("k".into(), None).embeddings_url,
            "https://sdk.vercel.ai/api/embed"
        );
        assert_eq!(
            SimpleHttpEmbedder::openrouter("k".into(), None, None).embeddings_url,
            "https://openrouter.ai/api/v1/embeddings"
        );
    }
}
