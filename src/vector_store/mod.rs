//! Vector store backends.
//!
//! Port of `src/vector-store/*`. Qdrant is the only supported backend,
//! accessed over its REST API via `reqwest`.

pub mod qdrant;

pub use qdrant::QdrantVectorStore;
