//! OpenAI embedder (`text-embedding-3-*`).
//!
//! Port of `src/embedders/openai.ts`. Calls the REST API directly with
//! `reqwest` — the surface is two endpoints (`POST /embeddings`,
//! `GET /models/{id}`), so pulling in an OpenAI SDK crate is not worth it.

use async_trait::async_trait;

use crate::log;
use crate::shared::constants::{
    INITIAL_RETRY_DELAY_MS, MAX_BATCH_RETRIES, MAX_BATCH_TOKENS, MAX_ITEM_TOKENS,
};
use crate::shared::embedding_models::{get_model_query_prefix, EmbedderProvider};
use crate::shared::validation::sanitize_error_message;
use crate::traits::{Embedder, EmbedderInfo, EmbeddingResponse, EmbeddingUsage, ValidationResult};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "text-embedding-3-small";

/// Token estimation used across providers: ~4 chars per token.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Applies the model's query prefix when `is_query` is set, skipping texts
/// that would exceed the per-item token limit once prefixed.
pub(crate) fn apply_query_prefix(
    provider: EmbedderProvider,
    model: &str,
    texts: &[String],
    is_query: bool,
) -> Vec<String> {
    if !is_query {
        return texts.to_vec();
    }
    let Some(prefix) = get_model_query_prefix(provider, model) else {
        return texts.to_vec();
    };
    texts
        .iter()
        .map(|text| {
            if text.starts_with(prefix) {
                return text.clone();
            }
            let prefixed = format!("{prefix}{text}");
            if estimate_tokens(&prefixed) > MAX_ITEM_TOKENS {
                log::warn(&format!(
                    "Text with prefix exceeds token limit ({})",
                    estimate_tokens(&prefixed)
                ));
                return text.clone();
            }
            prefixed
        })
        .collect()
}

/// Greedy batching plan: fills each batch up to MAX_BATCH_TOKENS and drops
/// items above MAX_ITEM_TOKENS (with a warning, like the TS version).
pub(crate) fn plan_batches(texts: &[String]) -> Vec<Vec<String>> {
    let mut remaining: Vec<String> = texts.to_vec();
    let mut batches = Vec::new();

    while !remaining.is_empty() {
        let mut current_batch: Vec<String> = Vec::new();
        let mut current_batch_tokens = 0usize;
        let mut processed_indices: Vec<usize> = Vec::new();

        for (i, text) in remaining.iter().enumerate() {
            let item_tokens = estimate_tokens(text);

            if item_tokens > MAX_ITEM_TOKENS {
                log::warn(&format!(
                    "Text at index {i} exceeds token limit ({item_tokens})"
                ));
                processed_indices.push(i);
                continue;
            }

            if current_batch_tokens + item_tokens > MAX_BATCH_TOKENS {
                break;
            }

            current_batch.push(text.clone());
            current_batch_tokens += item_tokens;
            processed_indices.push(i);
        }

        // Remove processed items from remaining, in reverse index order
        for &i in processed_indices.iter().rev() {
            remaining.remove(i);
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }
    }

    batches
}

/// Retries an embeddings call on 429 and 5xx with a linear backoff,
/// up to MAX_BATCH_RETRIES attempts (matches the TS `_callWithRetry`).
pub(crate) async fn call_embeddings_with_retry<F, Fut>(
    mut call: F,
) -> anyhow::Result<EmbeddingBatchResponse>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<EmbeddingBatchResponse, (Option<u16>, String)>> + Send,
{
    let mut retries_left = MAX_BATCH_RETRIES;
    loop {
        match call().await {
            Ok(response) => return Ok(response),
            Err((status, message)) => {
                let retryable = matches!(status, Some(429)) || status.is_some_and(|s| s >= 500);
                if retries_left > 0 && retryable {
                    let attempt = MAX_BATCH_RETRIES - retries_left + 1;
                    let delay = INITIAL_RETRY_DELAY_MS * attempt as u64;
                    log::warn(&format!(
                        "Retrying after {delay}ms (retries left: {retries_left})"
                    ));
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    retries_left -= 1;
                } else {
                    anyhow::bail!(sanitize_error_message(&message));
                }
            }
        }
    }
}

pub(crate) struct EmbeddingBatchResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub usage: Option<EmbeddingUsage>,
}

/// OpenAI implementation of the embedder interface.
pub struct OpenAiEmbedder {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model_id: String,
}

impl OpenAiEmbedder {
    pub fn new(api_key: String, model_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("embedding client with fixed timeout builds"),
            api_key,
            base_url: OPENAI_BASE_URL.to_string(),
            default_model_id: model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        }
    }

    async fn create_embeddings_batch(
        &self,
        model: &str,
        batch: &[String],
    ) -> Result<EmbeddingBatchResponse, (Option<u16>, String)> {
        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model, "input": batch }))
            .send()
            .await
            .map_err(|err| (None, sanitize_error_message(&err.to_string())))?;

        let status = response.status().as_u16();
        let body: serde_json::Value = response.json().await.map_err(|err| {
            (
                Some(status),
                sanitize_error_message(&format!("Invalid response body: {err}")),
            )
        })?;

        if !response_status_ok(status) {
            let message = body["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err((Some(status), sanitize_error_message(message)));
        }

        Ok(parse_openai_response(&body))
    }
}

/// Parses the OpenAI-style embeddings response shape
/// (`{data: [{embedding, index}], usage}`). Shared by the openai,
/// openai-compatible, mistral, vercel and openrouter providers.
pub(crate) fn parse_openai_response(body: &serde_json::Value) -> EmbeddingBatchResponse {
    let mut data: Vec<(usize, Vec<f32>)> = body["data"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let index = item["index"].as_u64().unwrap_or(0) as usize;
            let embedding = item["embedding"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();
            (index, embedding)
        })
        .collect();
    data.sort_by_key(|(index, _)| *index);

    let usage = if body["usage"].is_object() {
        Some(EmbeddingUsage {
            prompt_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            total_tokens: body["usage"]["total_tokens"].as_u64().unwrap_or(0),
        })
    } else {
        None
    };

    EmbeddingBatchResponse {
        embeddings: data.into_iter().map(|(_, embedding)| embedding).collect(),
        usage,
    }
}

fn response_status_ok(status: u16) -> bool {
    (200..300).contains(&status)
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse> {
        let model_to_use = model.unwrap_or(&self.default_model_id).to_string();
        let processed_texts =
            apply_query_prefix(EmbedderProvider::OpenAi, &model_to_use, texts, is_query);

        let mut all_embeddings: Vec<Vec<f32>> = Vec::new();
        let mut usage = EmbeddingUsage {
            prompt_tokens: 0,
            total_tokens: 0,
        };

        for batch in plan_batches(&processed_texts) {
            let response =
                call_embeddings_with_retry(|| self.create_embeddings_batch(&model_to_use, &batch))
                    .await?;
            all_embeddings.extend(response.embeddings);
            if let Some(batch_usage) = response.usage {
                usage.prompt_tokens += batch_usage.prompt_tokens;
                usage.total_tokens += batch_usage.total_tokens;
            }
        }

        Ok(EmbeddingResponse {
            embeddings: all_embeddings,
            usage: Some(usage),
        })
    }

    async fn validate_configuration(&self) -> ValidationResult {
        let result = self
            .client
            .get(format!(
                "{}/models/{}",
                self.base_url, self.default_model_id
            ))
            .bearer_auth(&self.api_key)
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => ValidationResult::ok(),
            Ok(response) => {
                let message = response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| body["error"]["message"].as_str().map(String::from))
                    .unwrap_or_else(|| "Validation failed".to_string());
                ValidationResult::invalid(sanitize_error_message(&message))
            }
            Err(err) => ValidationResult::invalid(sanitize_error_message(&err.to_string())),
        }
    }

    fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: EmbedderProvider::OpenAi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts_of_tokens(token_counts: &[usize]) -> Vec<String> {
        token_counts.iter().map(|&t| "x".repeat(t * 4)).collect()
    }

    #[test]
    fn token_estimation_is_ceil_len_over_4() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    // nomic-embed-code carries a query prefix under the ollama and
    // openai-compatible providers (not bare openai).
    const PREFIX_PROVIDER: EmbedderProvider = EmbedderProvider::Ollama;
    const PREFIX_MODEL: &str = "nomic-embed-code";

    #[test]
    fn query_prefix_applied_only_when_querying() {
        let texts = vec!["document body".to_string()];
        let as_query = apply_query_prefix(PREFIX_PROVIDER, PREFIX_MODEL, &texts, true);
        assert_eq!(
            as_query,
            vec!["Represent this query for searching relevant code: document body"]
        );

        let not_query = apply_query_prefix(PREFIX_PROVIDER, PREFIX_MODEL, &texts, false);
        assert_eq!(not_query, texts);

        // Models without a prefix pass through untouched
        let no_prefix = apply_query_prefix(
            EmbedderProvider::OpenAi,
            "text-embedding-3-small",
            &texts,
            true,
        );
        assert_eq!(no_prefix, texts);
    }

    #[test]
    fn query_prefix_not_doubled_when_already_present() {
        let texts = vec!["Represent this query for searching relevant code: already".to_string()];
        let prefixed = apply_query_prefix(PREFIX_PROVIDER, PREFIX_MODEL, &texts, true);
        assert_eq!(prefixed, texts);
    }

    #[test]
    fn batching_stays_under_token_budget() {
        // MAX_BATCH_TOKENS is 100k and items are capped at 8191 tokens:
        // 25 texts of 8000 tokens pack 12 per batch (12 * 8000 = 96000).
        let texts = texts_of_tokens(&[8000; 25]);
        let batches = plan_batches(&texts);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 12);
        assert_eq!(batches[1].len(), 12);
        assert_eq!(batches[2].len(), 1);
    }

    #[test]
    fn batching_packs_greedily() {
        // 13 x 8000 tokens: 12 fit in the first batch (96k), the last spills
        let texts = texts_of_tokens(&[8000; 13]);
        let batches = plan_batches(&texts);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 12);
        assert_eq!(batches[1].len(), 1);

        // Everything under the budget goes into a single batch
        let texts = texts_of_tokens(&[8000, 7000, 1000]);
        assert_eq!(plan_batches(&texts).len(), 1);
    }

    #[test]
    fn oversized_items_are_dropped() {
        let mut texts = texts_of_tokens(&[10]);
        texts.push("y".repeat((MAX_ITEM_TOKENS + 1) * 4));
        let batches = plan_batches(&texts);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
        assert!(!batches[0][0].contains('y'));
    }

    #[tokio::test]
    async fn non_retryable_errors_are_sanitized() {
        // 400 is not retryable -> immediate bail; the message must not echo
        // key-shaped strings (review #18 wiring).
        let secret = "sk-abcdefghijklmnopqrstuvwxyz123456";
        let err = match call_embeddings_with_retry(|| async {
            Err::<EmbeddingBatchResponse, _>((Some(400u16), format!("bad key {secret}")))
        })
        .await
        {
            Ok(_) => panic!("expected the non-retryable call to fail"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(message.contains("***REDACTED***"), "message: {message}");
        assert!(!message.contains(secret));
    }
}
