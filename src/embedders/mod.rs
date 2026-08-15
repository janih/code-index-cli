//! Embedding provider implementations.
//!
//! All providers are plain HTTPS APIs called with `reqwest`; provider-specific
//! auth/payload shaping lives in each module. Bedrock is deferred (needs AWS
//! SigV4) — see AGENTS.md.

pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod openai_compatible;
pub mod simple_http;

pub use gemini::GeminiEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiEmbedder;
pub use openai_compatible::OpenAiCompatibleEmbedder;
pub use simple_http::SimpleHttpEmbedder;
