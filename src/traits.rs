//! Core traits shared across modules.
//!
//! Port of `src/interfaces/*`. Traits exist at exactly the seams the TS
//! test suite mocks: embedders, vector store, code parser, cache manager.

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::shared::embedding_models::EmbedderProvider;

// ---------------------------------------------------------------------------
// IEmbedder
// ---------------------------------------------------------------------------

pub struct EmbeddingResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub usage: Option<EmbeddingUsage>,
}

pub struct EmbeddingUsage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

pub struct EmbedderInfo {
    pub name: EmbedderProvider,
}

pub struct ValidationResult {
    pub valid: bool,
    pub error: Option<String>,
}

impl ValidationResult {
    pub fn ok() -> Self {
        Self {
            valid: true,
            error: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            error: Some(message.into()),
        }
    }
}

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Creates embeddings for the given texts.
    ///
    /// When `is_query` is true, model-specific query prefixes (e.g. for
    /// nomic-embed-code) are prepended.
    async fn create_embeddings(
        &self,
        texts: &[String],
        model: Option<&str>,
        is_query: bool,
    ) -> anyhow::Result<EmbeddingResponse>;

    /// Tests connectivity and credentials.
    async fn validate_configuration(&self) -> ValidationResult;

    fn embedder_info(&self) -> EmbedderInfo;
}

// ---------------------------------------------------------------------------
// IVectorStore
// ---------------------------------------------------------------------------

pub struct PointStruct {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: serde_json::Map<String, serde_json::Value>,
}

/// Payload stored with each point (`filePath` / `codeChunk` / `startLine` /
/// `endLine` plus provider extras).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub file_path: String,
    pub code_chunk: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorStoreSearchResult {
    /// Qdrant point id — a UUID string in our usage, but the API also allows ints.
    pub id: serde_json::Value,
    pub score: f32,
    pub payload: Option<Payload>,
}

#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Returns true if a new collection was created.
    async fn initialize(&self) -> anyhow::Result<bool>;
    async fn upsert_points(&self, points: Vec<PointStruct>) -> anyhow::Result<()>;
    async fn search(
        &self,
        query_vector: &[f32],
        directory_prefix: Option<&str>,
        min_score: Option<f32>,
        max_results: Option<u32>,
    ) -> anyhow::Result<Vec<VectorStoreSearchResult>>;
    async fn delete_points_by_file_path(&self, file_path: &str) -> anyhow::Result<()>;
    async fn delete_points_by_multiple_file_paths(
        &self,
        file_paths: &[String],
    ) -> anyhow::Result<()>;
    async fn clear_collection(&self) -> anyhow::Result<()>;
    async fn delete_collection(&self) -> anyhow::Result<()>;
    async fn collection_exists(&self) -> anyhow::Result<bool>;
    async fn has_indexed_data(&self) -> anyhow::Result<bool>;
    async fn mark_indexing_complete(&self) -> anyhow::Result<()>;
    async fn mark_indexing_incomplete(&self) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// ICodeParser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    pub min_block_lines: Option<usize>,
    pub max_block_lines: Option<usize>,
    /// When provided, parse this content instead of reading the file.
    pub content: Option<String>,
    pub file_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeBlock {
    pub file_path: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub segment_hash: String,
    pub file_hash: String,
}

#[async_trait]
pub trait CodeParser: Send + Sync {
    async fn parse_file(
        &self,
        file_path: &Path,
        options: Option<ParseOptions>,
    ) -> anyhow::Result<Vec<CodeBlock>>;
}

// ---------------------------------------------------------------------------
// ICacheManager
// ---------------------------------------------------------------------------

#[async_trait]
pub trait CacheManager: Send + Sync {
    fn get_hash(&self, file_path: &str) -> Option<String>;
    fn update_hash(&self, file_path: &str, hash: String);
    fn delete_hash(&self, file_path: &str);
    async fn flush(&self) -> anyhow::Result<()>;
    fn get_all_hashes(&self) -> HashMap<String, String>;
}
