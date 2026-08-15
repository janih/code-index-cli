//! Qdrant vector store over the REST API.
//!
//! Port of `src/vector-store/qdrant-client.ts`. Instead of the official
//! client we use plain HTTP via `reqwest` — the surface used here is small:
//! collection info/create/delete, payload indexes, point upsert/query/delete.

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::log;
use crate::shared::constants::{
    DEFAULT_SEARCH_MIN_SCORE, DEFAULT_SEARCH_RESULTS, QDRANT_CODE_BLOCK_NAMESPACE,
};
use crate::traits::{Payload, PointStruct, VectorStore, VectorStoreSearchResult};

const DISTANCE_METRIC: &str = "Cosine";
const HNSW_M: u32 = 64;
const HNSW_EF_CONSTRUCT: u32 = 512;

/// Qdrant implementation of the vector store interface.
pub struct QdrantVectorStore {
    client: reqwest::Client,
    base_url: String,
    collection_name: String,
    vector_size: usize,
}

/// Normalizes user input to a URL with scheme and no trailing slashes.
/// Matches `parseQdrantUrl` in the TS version.
fn normalize_qdrant_url(url: Option<&str>) -> String {
    let trimmed = url.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return "http://localhost:6333".to_string();
    }
    if !trimmed.contains("://") {
        let with_scheme = if trimmed.starts_with("http") {
            trimmed.to_string()
        } else {
            format!("http://{trimmed}")
        };
        return with_scheme.trim_end_matches('/').to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

/// Collection name for a workspace: `ws-<sha256(workspacePath)[..16]>`.
fn collection_name_for_workspace(workspace_path: &str) -> String {
    let hash = hex::encode(Sha256::digest(workspace_path.as_bytes()));
    format!("ws-{}", &hash[..16])
}

/// Deterministic point id for the indexing-metadata sentinel point.
fn metadata_point_id() -> Uuid {
    let namespace = Uuid::parse_str(QDRANT_CODE_BLOCK_NAMESPACE)
        .expect("namespace constant must be a valid UUID");
    Uuid::new_v5(&namespace, b"indexing-metadata")
}

/// Extracts the configured vector size from GET /collections/{name} info.
/// `params.vectors` may be a bare number (deprecated form) or an object.
fn extract_vector_size(collection_info: &Value) -> usize {
    let vectors = &collection_info["config"]["params"]["vectors"];
    if let Some(n) = vectors.as_u64() {
        return n as usize;
    }
    vectors["size"].as_u64().unwrap_or(0) as usize
}

/// Directory-prefixed searches over-fetch 4x to compensate for filtering
/// (matches the TS heuristic).
fn effective_search_limit(directory_prefix: Option<&str>, max_results: Option<u32>) -> u32 {
    let base = max_results.unwrap_or(DEFAULT_SEARCH_RESULTS);
    if directory_prefix.is_some() {
        base * 4
    } else {
        base
    }
}

impl QdrantVectorStore {
    pub fn new(
        workspace_path: &str,
        url: Option<&str>,
        vector_size: usize,
        api_key: Option<&str>,
    ) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("Code-Index-CLI"),
        );
        if let Some(key) = api_key {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(key) {
                headers.insert("api-key", value);
            }
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            base_url: normalize_qdrant_url(url),
            collection_name: collection_name_for_workspace(workspace_path),
            vector_size,
        }
    }

    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    fn collections_url(&self) -> String {
        format!("{}/collections/{}", self.base_url, self.collection_name)
    }

    /// GET collection info; returns None when missing or on any error
    /// (matches the TS catch-all in `getCollectionInfo`).
    async fn get_collection_info(&self) -> Option<Value> {
        let response = self.client.get(self.collections_url()).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Value>().await.ok()?["result"].take().into()
    }

    async fn create_collection(&self) -> anyhow::Result<()> {
        self.put_json(
            &self.collections_url(),
            json!({
                "vectors": {
                    "size": self.vector_size,
                    "distance": DISTANCE_METRIC,
                    "on_disk": true,
                },
                "hnsw_config": {
                    "m": HNSW_M,
                    "ef_construct": HNSW_EF_CONSTRUCT,
                    "on_disk": true,
                },
            }),
        )
        .await?;
        Ok(())
    }

    /// Creates the payload indexes; failures are ignored since the index may
    /// already exist (same as the TS version).
    async fn create_payload_indexes(&self) {
        let url = format!("{}/index", self.collections_url());
        for (field_name, field_schema) in [
            ("filePath", "keyword"),
            ("filePath", "text"),
            ("segmentHash", "keyword"),
        ] {
            let _ = self
                .put_json(
                    &url,
                    json!({ "field_name": field_name, "field_schema": field_schema }),
                )
                .await;
        }
    }

    fn metadata_point(&self, status: &str) -> Value {
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .unwrap_or_default();
        json!({
            "id": metadata_point_id().to_string(),
            "vector": vec![0.0f32; self.vector_size],
            "payload": {
                "type": "metadata",
                "indexingStatus": status,
                "timestamp": timestamp,
            },
        })
    }

    async fn mark_indexing(&self, status: &str) {
        let url = format!("{}/points?wait=true", self.collections_url());
        let point = self.metadata_point(status);
        if let Err(err) = self.put_json(&url, json!({ "points": [point] })).await {
            log::warn(&format!("Failed to mark indexing {}: {}", status, err));
        }
    }

    /// Sends PUT and expects an "ok" envelope; errors include the response body.
    async fn put_json(&self, url: &str, body: Value) -> anyhow::Result<Value> {
        self.send_checked(self.client.put(url).json(&body)).await
    }

    /// Sends POST and expects an "ok" envelope; errors include the response body.
    async fn post_json(&self, url: &str, body: Value) -> anyhow::Result<Value> {
        self.send_checked(self.client.post(url).json(&body)).await
    }

    async fn send_checked(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("Qdrant request failed ({status}): {body}");
        }
        Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
    }
}

#[async_trait]
impl VectorStore for QdrantVectorStore {
    async fn initialize(&self) -> anyhow::Result<bool> {
        let result = async {
            let mut created = false;
            match self.get_collection_info().await {
                None => {
                    self.create_collection().await?;
                    created = true;
                }
                Some(info) => {
                    let existing_vector_size = extract_vector_size(&info);
                    if existing_vector_size != self.vector_size {
                        log::info(&format!(
							"Vector size mismatch (existing: {}, required: {}). Recreating collection...",
							existing_vector_size, self.vector_size
						));
                        self.delete_collection().await?;
                        self.create_collection().await?;
                        created = true;
                    }
                }
            }
            self.create_payload_indexes().await;
            Ok(created)
        };
        match result.await {
            Ok(created) => Ok(created),
            Err(err) => {
                log::error(&format!("Failed to initialize Qdrant: {err}"));
                Err(err)
            }
        }
    }

    async fn upsert_points(&self, points: Vec<PointStruct>) -> anyhow::Result<()> {
        let url = format!("{}/points?wait=true", self.collections_url());
        let body = json!({
            "points": points.into_iter().map(|p| json!({
                "id": p.id,
                "vector": p.vector,
                "payload": p.payload,
            })).collect::<Vec<_>>(),
        });
        self.put_json(&url, body).await.map(|_| ()).map_err(|err| {
            log::error(&format!("Failed to upsert points: {err}"));
            err
        })
    }

    async fn search(
        &self,
        query_vector: &[f32],
        directory_prefix: Option<&str>,
        min_score: Option<f32>,
        max_results: Option<u32>,
    ) -> anyhow::Result<Vec<VectorStoreSearchResult>> {
        let result = async {
            let mut body = json!({
                "query": query_vector,
                "limit": effective_search_limit(directory_prefix, max_results),
                "score_threshold": min_score.unwrap_or(DEFAULT_SEARCH_MIN_SCORE as f32),
                "with_payload": true,
            });
            if let Some(prefix) = directory_prefix {
                body["filter"] = json!({
                    "must": [{ "key": "filePath", "match": { "text": prefix } }],
                });
            }

            let url = format!("{}/points/query", self.collections_url());
            let response = self.post_json(&url, body).await?;
            let points = response["result"]["points"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            Ok(points
                .into_iter()
                .filter(|point| point["payload"]["type"] != json!("metadata"))
                .map(|point| VectorStoreSearchResult {
                    id: point["id"].clone(),
                    score: point["score"].as_f64().unwrap_or(0.0) as f32,
                    payload: serde_json::from_value::<Payload>(point["payload"].clone()).ok(),
                })
                .collect::<Vec<_>>())
        };
        match result.await {
            Ok(results) => Ok(results),
            Err(err) => {
                log::error(&format!("Search failed: {err}"));
                Err(err)
            }
        }
    }

    async fn delete_points_by_file_path(&self, file_path: &str) -> anyhow::Result<()> {
        let result = async {
            let url = format!("{}/points/delete?wait=true", self.collections_url());
            self.post_json(
                &url,
                json!({
                    "filter": {
                        "must": [{ "key": "filePath", "match": { "value": file_path } }],
                    },
                }),
            )
            .await?;
            Ok(())
        };
        match result.await {
            Ok(()) => Ok(()),
            Err(err) => {
                log::error(&format!(
                    "Failed to delete points for {}: {}",
                    file_path, err
                ));
                Err(err)
            }
        }
    }

    async fn delete_points_by_multiple_file_paths(
        &self,
        file_paths: &[String],
    ) -> anyhow::Result<()> {
        let result = async {
            let url = format!("{}/points/delete?wait=true", self.collections_url());
            self.post_json(
                &url,
                json!({
                    "filter": {
                        "should": file_paths.iter().map(|fp| json!({
                            "key": "filePath", "match": { "value": fp },
                        })).collect::<Vec<_>>(),
                    },
                }),
            )
            .await?;
            Ok(())
        };
        match result.await {
            Ok(()) => Ok(()),
            Err(err) => {
                log::error(&format!("Failed to delete points: {err}"));
                Err(err)
            }
        }
    }

    async fn clear_collection(&self) -> anyhow::Result<()> {
        let result = async {
            let url = format!("{}/points/delete?wait=true", self.collections_url());
            self.post_json(&url, json!({ "filter": { "match_all": {} } }))
                .await?;
            Ok(())
        };
        match result.await {
            Ok(()) => Ok(()),
            Err(err) => {
                log::error(&format!("Failed to clear collection: {err}"));
                Err(err)
            }
        }
    }

    async fn delete_collection(&self) -> anyhow::Result<()> {
        let response = self.client.delete(self.collections_url()).send().await;
        match response {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let err = anyhow::anyhow!("Qdrant request failed ({status}): {body}");
                log::error(&format!("Failed to delete collection: {err}"));
                Err(err)
            }
            Err(err) => {
                let err = anyhow::Error::from(err);
                log::error(&format!("Failed to delete collection: {err}"));
                Err(err)
            }
        }
    }

    async fn collection_exists(&self) -> anyhow::Result<bool> {
        Ok(self.get_collection_info().await.is_some())
    }

    async fn has_indexed_data(&self) -> anyhow::Result<bool> {
        let Some(info) = self.get_collection_info().await else {
            return Ok(false);
        };
        Ok(info["points_count"].as_u64().unwrap_or(0) > 0)
    }

    async fn mark_indexing_complete(&self) -> anyhow::Result<()> {
        self.mark_indexing("complete").await;
        Ok(())
    }

    async fn mark_indexing_incomplete(&self) -> anyhow::Result<()> {
        self.mark_indexing("incomplete").await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_qdrant_urls() {
        assert_eq!(normalize_qdrant_url(None), "http://localhost:6333");
        assert_eq!(normalize_qdrant_url(Some("")), "http://localhost:6333");
        assert_eq!(normalize_qdrant_url(Some("  ")), "http://localhost:6333");
        assert_eq!(
            normalize_qdrant_url(Some("localhost:6333")),
            "http://localhost:6333"
        );
        assert_eq!(normalize_qdrant_url(Some("localhost")), "http://localhost");
        assert_eq!(
            normalize_qdrant_url(Some("https://qdrant.example.com")),
            "https://qdrant.example.com"
        );
        assert_eq!(
            normalize_qdrant_url(Some("http://a:6333/")),
            "http://a:6333"
        );
        assert_eq!(
            normalize_qdrant_url(Some("http://a:6333/prefix/")),
            "http://a:6333/prefix"
        );
    }

    #[test]
    fn collection_name_is_deterministic_and_scoped() {
        let a = collection_name_for_workspace("/tmp/ws-a");
        assert!(a.starts_with("ws-"));
        assert_eq!(a.len(), 3 + 16);
        assert_eq!(a, collection_name_for_workspace("/tmp/ws-a"));
        assert_ne!(a, collection_name_for_workspace("/tmp/ws-b"));
    }

    #[test]
    fn metadata_point_id_is_a_deterministic_uuid_v5() {
        let id = metadata_point_id();
        assert_eq!(id, metadata_point_id());
        assert_eq!(id.get_version(), Some(uuid::Version::Sha1));
        // Exact value from RFC-4122 v5 over the constants above — locked so a
        // Rust/TS index share the sentinel point id.
        assert_eq!(id.to_string(), "b033a1fc-43f2-55f9-a8b4-9ceb848d50e3");
    }

    #[test]
    fn extracts_vector_size_from_both_shapes() {
        let object_shape = json!({ "config": { "params": { "vectors": { "size": 1536 } } } });
        assert_eq!(extract_vector_size(&object_shape), 1536);

        let number_shape = json!({ "config": { "params": { "vectors": 768 } } });
        assert_eq!(extract_vector_size(&number_shape), 768);

        assert_eq!(extract_vector_size(&json!({})), 0);
    }

    #[test]
    fn search_limit_quadruples_for_directory_prefix() {
        assert_eq!(effective_search_limit(None, None), DEFAULT_SEARCH_RESULTS);
        assert_eq!(effective_search_limit(None, Some(10)), 10);
        assert_eq!(effective_search_limit(Some("src/"), Some(10)), 40);
    }

    #[test]
    fn store_derives_collection_name_and_url() {
        let store = QdrantVectorStore::new("/tmp/ws-a", Some("http://localhost:6333"), 1536, None);
        assert_eq!(
            store.collection_name(),
            collection_name_for_workspace("/tmp/ws-a")
        );
    }
}
