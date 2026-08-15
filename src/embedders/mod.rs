//! Embedding provider implementations.
//!
//! Port of `src/embedders/*`. All providers are plain HTTPS APIs called with
//! `reqwest`; provider-specific auth/payload shaping lives in each module.

pub mod openai;
pub mod openai_compatible;

pub use openai::OpenAiEmbedder;
pub use openai_compatible::OpenAiCompatibleEmbedder;
