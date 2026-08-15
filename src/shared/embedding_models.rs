//! Embedding model profiles (dimensions, score thresholds, query prefixes).
//!
//! Port of `src/shared/embedding-models.ts` — data preserved 1:1.

use std::fmt;

/// Embedder providers supported by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmbedderProvider {
    OpenAi,
    Ollama,
    OpenAiCompatible,
    Gemini,
    Mistral,
    VercelAiGateway,
    Bedrock,
    OpenRouter,
}

impl EmbedderProvider {
    /// All providers that have model profiles (parity with the TS type union).
    pub const ALL: &'static [EmbedderProvider] = &[
        Self::OpenAi,
        Self::Ollama,
        Self::OpenAiCompatible,
        Self::Gemini,
        Self::Mistral,
        Self::VercelAiGateway,
        Self::Bedrock,
        Self::OpenRouter,
    ];

    /// The string form used in config files and API payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Gemini => "gemini",
            Self::Mistral => "mistral",
            Self::VercelAiGateway => "vercel-ai-gateway",
            Self::Bedrock => "bedrock",
            Self::OpenRouter => "openrouter",
        }
    }

    /// Parses the config/API string form.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.as_str() == value)
    }

    /// Known model profiles for this provider (`model_id` → profile).
    pub fn profiles(self) -> &'static [(&'static str, ModelProfile)] {
        match self {
            Self::OpenAi => OPENAI_MODELS,
            Self::Ollama => OLLAMA_MODELS,
            Self::OpenAiCompatible => OPENAI_COMPATIBLE_MODELS,
            Self::Gemini => GEMINI_MODELS,
            Self::Mistral => MISTRAL_MODELS,
            Self::VercelAiGateway => VERCEL_AI_GATEWAY_MODELS,
            Self::Bedrock => BEDROCK_MODELS,
            Self::OpenRouter => OPENROUTER_MODELS,
        }
    }
}

impl fmt::Display for EmbedderProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelProfile {
    pub dimension: usize,
    pub score_threshold: f64,
    pub query_prefix: Option<&'static str>,
}

const fn profile(dimension: usize, score_threshold: f64) -> ModelProfile {
    ModelProfile {
        dimension,
        score_threshold,
        query_prefix: None,
    }
}

const NOMIC_EMBED_CODE_QUERY_PREFIX: &str = "Represent this query for searching relevant code: ";
const NOMIC_EMBED_CODE_PROFILE: ModelProfile = ModelProfile {
    dimension: 3584,
    score_threshold: 0.15,
    query_prefix: Some(NOMIC_EMBED_CODE_QUERY_PREFIX),
};

const OPENAI_MODELS: &[(&str, ModelProfile)] = &[
    ("text-embedding-3-small", profile(1536, 0.4)),
    ("text-embedding-3-large", profile(3072, 0.4)),
    ("text-embedding-ada-002", profile(1536, 0.4)),
];

const OLLAMA_MODELS: &[(&str, ModelProfile)] = &[
    ("nomic-embed-text", profile(768, 0.4)),
    ("nomic-embed-code", NOMIC_EMBED_CODE_PROFILE),
    ("mxbai-embed-large", profile(1024, 0.4)),
    ("all-minilm", profile(384, 0.4)),
];

const OPENAI_COMPATIBLE_MODELS: &[(&str, ModelProfile)] = &[
    ("text-embedding-3-small", profile(1536, 0.4)),
    ("text-embedding-3-large", profile(3072, 0.4)),
    ("text-embedding-ada-002", profile(1536, 0.4)),
    ("nomic-embed-code", NOMIC_EMBED_CODE_PROFILE),
];

const GEMINI_MODELS: &[(&str, ModelProfile)] = &[
    ("gemini-embedding-001", profile(3072, 0.4)),
    ("text-embedding-004", profile(3072, 0.4)),
];

const MISTRAL_MODELS: &[(&str, ModelProfile)] = &[("codestral-embed-2505", profile(1536, 0.4))];

const VERCEL_AI_GATEWAY_MODELS: &[(&str, ModelProfile)] = &[
    ("openai/text-embedding-3-small", profile(1536, 0.4)),
    ("openai/text-embedding-3-large", profile(3072, 0.4)),
    ("openai/text-embedding-ada-002", profile(1536, 0.4)),
    ("cohere/embed-v4.0", profile(1024, 0.4)),
    ("google/gemini-embedding-001", profile(3072, 0.4)),
    ("google/text-embedding-005", profile(768, 0.4)),
    ("google/text-multilingual-embedding-002", profile(768, 0.4)),
    ("amazon/titan-embed-text-v2", profile(1024, 0.4)),
    ("mistral/codestral-embed", profile(1536, 0.4)),
    ("mistral/mistral-embed", profile(1024, 0.4)),
];

const BEDROCK_MODELS: &[(&str, ModelProfile)] = &[
    ("amazon.titan-embed-text-v1", profile(1536, 0.4)),
    ("amazon.titan-embed-text-v2:0", profile(1024, 0.4)),
    ("amazon.titan-embed-image-v1", profile(1024, 0.4)),
    (
        "amazon.nova-2-multimodal-embeddings-v1:0",
        profile(1024, 0.4),
    ),
    ("cohere.embed-v4:0", profile(1536, 0.4)),
    ("cohere.embed-english-v3", profile(1024, 0.4)),
    ("cohere.embed-multilingual-v3", profile(1024, 0.4)),
];

const OPENROUTER_MODELS: &[(&str, ModelProfile)] = &[
    ("openai/text-embedding-3-small", profile(1536, 0.4)),
    ("openai/text-embedding-3-large", profile(3072, 0.4)),
    ("openai/text-embedding-ada-002", profile(1536, 0.4)),
    ("google/gemini-embedding-001", profile(3072, 0.4)),
    ("mistralai/mistral-embed-2312", profile(1024, 0.4)),
    ("mistralai/codestral-embed-2505", profile(1536, 0.4)),
    ("qwen/qwen3-embedding-0.6b", profile(1024, 0.4)),
    ("qwen/qwen3-embedding-4b", profile(2560, 0.4)),
    ("qwen/qwen3-embedding-8b", profile(4096, 0.4)),
];

fn find_profile(provider: EmbedderProvider, model_id: &str) -> Option<ModelProfile> {
    provider
        .profiles()
        .iter()
        .find(|(id, _)| *id == model_id)
        .map(|(_, p)| *p)
}

pub fn get_model_dimension(provider: EmbedderProvider, model_id: &str) -> Option<usize> {
    find_profile(provider, model_id).map(|p| p.dimension)
}

pub fn get_model_score_threshold(provider: EmbedderProvider, model_id: &str) -> Option<f64> {
    find_profile(provider, model_id).map(|p| p.score_threshold)
}

pub fn get_model_query_prefix(provider: EmbedderProvider, model_id: &str) -> Option<&'static str> {
    find_profile(provider, model_id).and_then(|p| p.query_prefix)
}

pub fn get_default_model_id(provider: EmbedderProvider) -> &'static str {
    match provider {
        EmbedderProvider::OpenAi | EmbedderProvider::OpenAiCompatible => "text-embedding-3-small",
        EmbedderProvider::Ollama => OLLAMA_MODELS
            .first()
            .map(|(id, _)| *id)
            .unwrap_or("nomic-embed-text"),
        EmbedderProvider::Gemini => "gemini-embedding-001",
        EmbedderProvider::Mistral => "codestral-embed-2505",
        EmbedderProvider::VercelAiGateway => "openai/text-embedding-3-large",
        EmbedderProvider::Bedrock => "amazon.titan-embed-text-v2:0",
        EmbedderProvider::OpenRouter => "openai/text-embedding-3-large",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_providers_have_profiles() {
        for provider in EmbedderProvider::ALL {
            assert!(
                !provider.profiles().is_empty(),
                "no profiles for {provider}"
            );
        }
    }

    #[test]
    fn every_profile_has_valid_dimension_and_threshold() {
        for provider in EmbedderProvider::ALL {
            for (model_id, profile) in provider.profiles() {
                assert!(
                    profile.dimension > 0,
                    "{provider}/{model_id}: bad dimension"
                );
                assert!(
                    profile.score_threshold > 0.0,
                    "{provider}/{model_id}: bad threshold"
                );
                assert!(
                    profile.score_threshold <= 1.0,
                    "{provider}/{model_id}: bad threshold"
                );
            }
        }
    }

    #[test]
    fn ollama_nomic_embed_code_has_query_prefix() {
        let prefix = get_model_query_prefix(EmbedderProvider::Ollama, "nomic-embed-code");
        assert!(prefix.is_some());
        assert!(prefix.unwrap().contains("Represent this query"));
    }

    #[test]
    fn known_openai_dimensions() {
        assert_eq!(
            get_model_dimension(EmbedderProvider::OpenAi, "text-embedding-3-small"),
            Some(1536)
        );
        assert_eq!(
            get_model_dimension(EmbedderProvider::OpenAi, "text-embedding-3-large"),
            Some(3072)
        );
        assert_eq!(
            get_model_dimension(EmbedderProvider::OpenAi, "text-embedding-ada-002"),
            Some(1536)
        );
    }

    #[test]
    fn known_ollama_dimensions() {
        assert_eq!(
            get_model_dimension(EmbedderProvider::Ollama, "nomic-embed-text"),
            Some(768)
        );
        assert_eq!(
            get_model_dimension(EmbedderProvider::Ollama, "nomic-embed-code"),
            Some(3584)
        );
        assert_eq!(
            get_model_dimension(EmbedderProvider::Ollama, "mxbai-embed-large"),
            Some(1024)
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(
            get_model_dimension(EmbedderProvider::OpenAi, "no-such-model"),
            None
        );
        assert_eq!(
            get_model_score_threshold(EmbedderProvider::Gemini, "no-such-model"),
            None
        );
        assert_eq!(
            get_model_query_prefix(EmbedderProvider::Mistral, "no-such-model"),
            None
        );
    }

    #[test]
    fn score_thresholds_match_profiles() {
        assert_eq!(
            get_model_score_threshold(EmbedderProvider::Ollama, "nomic-embed-code"),
            Some(0.15)
        );
        assert_eq!(
            get_model_score_threshold(EmbedderProvider::OpenAi, "text-embedding-3-small"),
            Some(0.4)
        );
    }

    #[test]
    fn default_models_match_ts() {
        assert_eq!(
            get_default_model_id(EmbedderProvider::OpenAi),
            "text-embedding-3-small"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::OpenAiCompatible),
            "text-embedding-3-small"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::Ollama),
            "nomic-embed-text"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::Gemini),
            "gemini-embedding-001"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::Mistral),
            "codestral-embed-2505"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::VercelAiGateway),
            "openai/text-embedding-3-large"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::Bedrock),
            "amazon.titan-embed-text-v2:0"
        );
        assert_eq!(
            get_default_model_id(EmbedderProvider::OpenRouter),
            "openai/text-embedding-3-large"
        );
    }

    #[test]
    fn provider_string_roundtrip() {
        for provider in EmbedderProvider::ALL {
            assert_eq!(EmbedderProvider::parse(provider.as_str()), Some(*provider));
        }
        assert_eq!(
            EmbedderProvider::parse("openai-compatible"),
            Some(EmbedderProvider::OpenAiCompatible)
        );
        assert_eq!(EmbedderProvider::parse("no-such-provider"), None);
    }
}
