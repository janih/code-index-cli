//! Embedder providers: openai, ollama, openai-compatible, gemini, mistral,
//! vercel-ai-gateway, openrouter, and bedrock (behind the `bedrock` feature).
//!
//! Port of `src/embedders/*`. Shared HTTP base with retry + error
//! sanitization lands in Phase 1; providers fan out in Phase 2.
