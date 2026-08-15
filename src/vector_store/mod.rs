//! Vector store backends.
//!
//! Qdrant is the only supported backend,
//! accessed over its REST API via `reqwest`.

pub mod qdrant;

pub use qdrant::QdrantVectorStore;
