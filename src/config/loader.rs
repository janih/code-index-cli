//! Layered configuration loading.
//!
//! Port of `src/config/config-loader.ts`. Priority (low to high):
//! defaults → user config (`~/.config/code-index/config.json`) → project
//! config (`.code-index.json` / `code-index.config.json`) → environment
//! variables (`CODE_INDEX_CLI_*`) → CLI flags.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::log;

use super::schema::CliConfig;

const PROJECT_CONFIG_FILENAMES: [&str; 2] = [".code-index.json", "code-index.config.json"];

/// Environment variable mapping for config values (TS: `ENV_MAP`).
const ENV_MAP: [(&str, &str); 15] = [
    ("CODE_INDEX_CLI_EMBEDDER_PROVIDER", "embedder.provider"),
    ("CODE_INDEX_CLI_EMBEDDER_API_KEY", "embedder.apiKey"),
    ("CODE_INDEX_CLI_EMBEDDER_MODEL_ID", "embedder.modelId"),
    ("CODE_INDEX_CLI_EMBEDDER_BASE_URL", "embedder.baseUrl"),
    (
        "CODE_INDEX_CLI_EMBEDDER_MODEL_DIMENSION",
        "embedder.modelDimension",
    ),
    (
        "CODE_INDEX_CLI_EMBEDDER_COMPATIBLE_BASE_URL",
        "embedder.compatibleBaseUrl",
    ),
    (
        "CODE_INDEX_CLI_EMBEDDER_COMPATIBLE_API_KEY",
        "embedder.compatibleApiKey",
    ),
    ("CODE_INDEX_CLI_QDRANT_URL", "qdrant.url"),
    ("CODE_INDEX_CLI_QDRANT_API_KEY", "qdrant.apiKey"),
    ("CODE_INDEX_CLI_SEARCH_MIN_SCORE", "search.minScore"),
    ("CODE_INDEX_CLI_SEARCH_MAX_RESULTS", "search.maxResults"),
    ("CODE_INDEX_CLI_BATCH_SIZE", "indexing.batchSize"),
    ("CODE_INDEX_CLI_BEDROCK_REGION", "embedder.bedrockRegion"),
    ("CODE_INDEX_CLI_BEDROCK_PROFILE", "embedder.bedrockProfile"),
    (
        "CODE_INDEX_CLI_OPENROUTER_PROVIDER",
        "embedder.openRouterProvider",
    ),
];

/// Optional CLI flag overrides (TS: `CliFlags`). Highest priority layer.
#[derive(Debug, Default, Clone)]
pub struct CliFlags {
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub api_key: Option<String>,
    pub qdrant_url: Option<String>,
    pub qdrant_api_key: Option<String>,
    pub batch_size: Option<u32>,
    pub min_score: Option<f64>,
    pub max_results: Option<u32>,
    pub base_url: Option<String>,
    pub model_dimension: Option<u32>,
    pub bedrock_region: Option<String>,
    pub bedrock_profile: Option<String>,
    pub compatible_base_url: Option<String>,
    pub compatible_api_key: Option<String>,
    pub open_router_provider: Option<String>,
}

/// Path to the user-level config file (`getUserConfigPath` in TS).
pub fn user_config_path() -> PathBuf {
    user_config_dir().join("config.json")
}

fn user_config_dir() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("code-index")
}

/// Path to the project config file — the existing one if present, otherwise
/// the default `.code-index.json` location (`getProjectConfigPath` in TS).
pub fn project_config_path(workspace_path: &Path) -> PathBuf {
    find_project_config(workspace_path)
        .unwrap_or_else(|| workspace_path.join(PROJECT_CONFIG_FILENAMES[0]))
}

/// Finds the project config file in the workspace, preferring
/// `.code-index.json` over `code-index.config.json`.
fn find_project_config(workspace_path: &Path) -> Option<PathBuf> {
    PROJECT_CONFIG_FILENAMES
        .iter()
        .map(|filename| workspace_path.join(filename))
        .find(|path| path.exists())
}

/// Reads and parses a JSON config file. Returns `None` (with a warning) for
/// missing, unreadable, or malformed files — matching the TS behavior of
/// falling back gracefully.
fn read_config_file(file_path: &Path) -> Option<Map<String, Value>> {
    let content = match std::fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn(&format!(
                "Failed to read config file {}: {}",
                file_path.display(),
                err
            ));
            return None;
        }
    };

    match serde_json::from_str::<Value>(&content) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => {
            log::warn(&format!(
                "Failed to read config file {}: expected a JSON object",
                file_path.display()
            ));
            None
        }
        Err(err) => {
            log::warn(&format!(
                "Failed to read config file {}: {}",
                file_path.display(),
                err
            ));
            None
        }
    }
}

/// Sets a nested value in a JSON object using a dot-separated path, coercing
/// the string like the TS version: "true"/"false" → bool, parseable → number,
/// otherwise string.
fn set_nested_value(obj: &mut Map<String, Value>, key_path: &str, value: &str) {
    let keys: Vec<&str> = key_path.split('.').collect();
    let mut current = obj;
    for key in &keys[..keys.len() - 1] {
        current = current
            .entry((*key).to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("path component is an object (created above if missing)");
    }

    let coerced = if value == "true" {
        Value::Bool(true)
    } else if value == "false" {
        Value::Bool(false)
    } else if !value.trim().is_empty() {
        if let Ok(int) = value.parse::<i64>() {
            Value::Number(int.into())
        } else if let Ok(float) = value.parse::<f64>() {
            serde_json::Number::from_f64(float)
                .map_or(Value::String(value.to_string()), Value::Number)
        } else {
            Value::String(value.to_string())
        }
    } else {
        Value::String(value.to_string())
    };

    current.insert(keys[keys.len() - 1].to_string(), coerced);
}

/// Sets a nested JSON value to an already-typed value (used for CLI flags).
fn set_nested_typed(obj: &mut Map<String, Value>, key_path: &str, value: Value) {
    let keys: Vec<&str> = key_path.split('.').collect();
    let mut current = obj;
    for key in &keys[..keys.len() - 1] {
        current = current
            .entry((*key).to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("path component is an object (created above if missing)");
    }
    current.insert(keys[keys.len() - 1].to_string(), value);
}

/// Deep merges `override_` into `base`. Objects merge recursively; anything
/// else (arrays, scalars) is replaced. Matches TS `deepMerge`.
fn deep_merge(base: &mut Map<String, Value>, override_: Map<String, Value>) {
    for (key, override_value) in override_ {
        match (base.get(&key), &override_value) {
            (Some(Value::Object(_)), Value::Object(_)) => {
                let base_obj = base
                    .get_mut(&key)
                    .and_then(Value::as_object_mut)
                    .expect("checked above");
                let override_obj = match override_value {
                    Value::Object(map) => map,
                    _ => unreachable!("checked above"),
                };
                deep_merge(base_obj, override_obj);
            }
            _ => {
                base.insert(key, override_value);
            }
        }
    }
}

/// Env-var overrides as a nested JSON object.
fn collect_env_overrides(getenv: &dyn Fn(&str) -> Option<String>) -> Map<String, Value> {
    let mut overrides = Map::new();
    for (env_var, config_path) in ENV_MAP {
        if let Some(value) = getenv(env_var) {
            if !value.is_empty() {
                set_nested_value(&mut overrides, config_path, &value);
            }
        }
    }
    overrides
}

/// CLI flag overrides as a nested JSON object (TS: `applyCliFlags`).
fn collect_flag_overrides(flags: &CliFlags) -> Map<String, Value> {
    let mut overrides = Map::new();

    let mut set = |path: &str, value: Option<Value>| {
        if let Some(value) = value {
            set_nested_typed(&mut overrides, path, value);
        }
    };

    set(
        "embedder.provider",
        flags.provider.clone().map(Value::String),
    );
    set(
        "embedder.modelId",
        flags.model_id.clone().map(Value::String),
    );
    set("embedder.apiKey", flags.api_key.clone().map(Value::String));
    set("qdrant.url", flags.qdrant_url.clone().map(Value::String));
    set(
        "qdrant.apiKey",
        flags.qdrant_api_key.clone().map(Value::String),
    );
    set("indexing.batchSize", flags.batch_size.map(Into::into));
    set(
        "search.minScore",
        flags
            .min_score
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
    );
    set("search.maxResults", flags.max_results.map(Into::into));
    set(
        "embedder.baseUrl",
        flags.base_url.clone().map(Value::String),
    );
    set(
        "embedder.modelDimension",
        flags.model_dimension.map(Into::into),
    );
    set(
        "embedder.bedrockRegion",
        flags.bedrock_region.clone().map(Value::String),
    );
    set(
        "embedder.bedrockProfile",
        flags.bedrock_profile.clone().map(Value::String),
    );
    set(
        "embedder.compatibleBaseUrl",
        flags.compatible_base_url.clone().map(Value::String),
    );
    set(
        "embedder.compatibleApiKey",
        flags.compatible_api_key.clone().map(Value::String),
    );
    set(
        "embedder.openRouterProvider",
        flags.open_router_provider.clone().map(Value::String),
    );

    overrides
}

/// Loads configuration with layered priority:
/// defaults < user config < project config < env vars < CLI flags.
///
/// `config_path` overrides project-config discovery when given.
pub fn load_config(
    workspace_path: &Path,
    config_path: Option<&Path>,
    flags: Option<&CliFlags>,
) -> anyhow::Result<CliConfig> {
    let user_path = user_config_path();
    load_config_with(
        workspace_path,
        config_path,
        flags,
        &|key| std::env::var(key).ok(),
        &user_path,
    )
}

/// Inner loader with injectable env lookup and user-config path so tests stay
/// hermetic (no process env mutation, no ~/.config reads).
fn load_config_with(
    workspace_path: &Path,
    config_path: Option<&Path>,
    flags: Option<&CliFlags>,
    getenv: &dyn Fn(&str) -> Option<String>,
    user_config_path: &Path,
) -> anyhow::Result<CliConfig> {
    // 1. Defaults
    let mut config = serde_json::to_value(CliConfig::default())
        .and_then(serde_json::from_value::<Map<String, Value>>)
        .expect("CliConfig::default() serializes to a JSON object");

    // 2. User-level config
    if let Some(user_config) = read_config_file(user_config_path) {
        log::debug(&format!(
            "Loaded user config from {}",
            user_config_path.display()
        ));
        deep_merge(&mut config, user_config);
    }

    // 3. Project config (explicit path or discovered)
    let project_path = config_path
        .map(Path::to_path_buf)
        .or_else(|| find_project_config(workspace_path));
    if let Some(project_path) = project_path {
        if let Some(project_config) = read_config_file(&project_path) {
            log::debug(&format!(
                "Loaded project config from {}",
                project_path.display()
            ));
            deep_merge(&mut config, project_config);
        }
    }

    // 4. Environment variables
    let env_overrides = collect_env_overrides(getenv);
    if !env_overrides.is_empty() {
        log::debug("Applying environment variable overrides");
        deep_merge(&mut config, env_overrides);
    }

    // 5. CLI flags (highest priority)
    if let Some(flags) = flags {
        deep_merge(&mut config, collect_flag_overrides(flags));
    }

    // Deserialize + validate (TS: zod safeParse)
    let parsed: CliConfig = match serde_json::from_value(Value::Object(config)) {
        Ok(parsed) => parsed,
        Err(err) => {
            log::error(&format!("Configuration validation failed: {}", err));
            anyhow::bail!(
                "Invalid configuration. Run 'code-index init' to create a valid config file."
            )
        }
    };

    let issues = parsed.validation_issues();
    if !issues.is_empty() {
        log::error("Configuration validation failed:");
        for issue in &issues {
            log::error(&format!("  {}", issue));
        }
        anyhow::bail!("Invalid configuration. Run 'code-index init' to create a valid config file.")
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// No env vars set.
    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("code-index-config-test-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_user_config() -> PathBuf {
        temp_dir("user").join("config.json")
    }

    fn load(
        workspace: &Path,
        config_path: Option<&Path>,
        flags: Option<&CliFlags>,
        getenv: &dyn Fn(&str) -> Option<String>,
    ) -> CliConfig {
        load_config_with(workspace, config_path, flags, getenv, &fake_user_config())
            .expect("config loads")
    }

    #[test]
    fn returns_defaults_when_no_files_exist() {
        let dir = temp_dir("defaults");
        let config = load(&dir, None, None, &no_env);
        assert!(config.enabled);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::OpenAi
        );
        assert_eq!(config.qdrant.url, "http://localhost:6333");
    }

    #[test]
    fn loads_project_config_from_code_index_json() {
        let dir = temp_dir("project");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "ollama", "baseUrl": "http://custom-ollama:11434"}}"#,
        )
        .unwrap();
        let config = load(&dir, None, None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Ollama
        );
        assert_eq!(
            config.embedder.base_url.as_deref(),
            Some("http://custom-ollama:11434")
        );
    }

    #[test]
    fn loads_project_config_from_alternative_filename() {
        let dir = temp_dir("project-alt");
        std::fs::write(
            dir.join("code-index.config.json"),
            r#"{"embedder": {"provider": "gemini"}}"#,
        )
        .unwrap();
        let config = load(&dir, None, None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Gemini
        );
    }

    #[test]
    fn prefers_primary_project_config_filename() {
        let dir = temp_dir("project-pref");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "ollama"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("code-index.config.json"),
            r#"{"embedder": {"provider": "gemini"}}"#,
        )
        .unwrap();
        let config = load(&dir, None, None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Ollama
        );
    }

    #[test]
    fn explicit_config_path_takes_precedence() {
        let dir = temp_dir("explicit");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "ollama"}}"#,
        )
        .unwrap();
        let other = temp_dir("explicit-other").join("other.json");
        std::fs::write(&other, r#"{"embedder": {"provider": "mistral"}}"#).unwrap();
        let config = load(&dir, Some(&other), None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Mistral
        );
    }

    #[test]
    fn applies_env_var_overrides() {
        let dir = temp_dir("env");
        let env: HashMap<&str, &str> = [
            ("CODE_INDEX_CLI_EMBEDDER_PROVIDER", "ollama"),
            ("CODE_INDEX_CLI_QDRANT_URL", "http://custom-qdrant:6333"),
        ]
        .into_iter()
        .collect();
        let getenv = |key: &str| env.get(key).map(|v| v.to_string());
        let config = load(&dir, None, None, &getenv);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Ollama
        );
        assert_eq!(config.qdrant.url, "http://custom-qdrant:6333");
    }

    #[test]
    fn applies_numeric_env_vars() {
        let dir = temp_dir("env-num");
        let env: HashMap<&str, &str> = [
            ("CODE_INDEX_CLI_BATCH_SIZE", "100"),
            ("CODE_INDEX_CLI_SEARCH_MIN_SCORE", "0.6"),
        ]
        .into_iter()
        .collect();
        let getenv = |key: &str| env.get(key).map(|v| v.to_string());
        let config = load(&dir, None, None, &getenv);
        assert_eq!(config.indexing.batch_size, 100);
        assert_eq!(config.search.min_score, 0.6);
    }

    #[test]
    fn applies_cli_flag_overrides() {
        let dir = temp_dir("flags");
        let flags = CliFlags {
            provider: Some("gemini".to_string()),
            qdrant_url: Some("http://flag-qdrant:6333".to_string()),
            ..Default::default()
        };
        let config = load(&dir, None, Some(&flags), &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Gemini
        );
        assert_eq!(config.qdrant.url, "http://flag-qdrant:6333");
    }

    #[test]
    fn cli_flags_beat_env_vars() {
        let dir = temp_dir("flags-over-env");
        let env: HashMap<&str, &str> = [("CODE_INDEX_CLI_EMBEDDER_PROVIDER", "ollama")]
            .into_iter()
            .collect();
        let getenv = |key: &str| env.get(key).map(|v| v.to_string());
        let flags = CliFlags {
            provider: Some("gemini".to_string()),
            ..Default::default()
        };
        let config = load(&dir, None, Some(&flags), &getenv);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Gemini
        );
    }

    #[test]
    fn env_vars_beat_project_config() {
        let dir = temp_dir("env-over-project");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "ollama"}}"#,
        )
        .unwrap();
        let env: HashMap<&str, &str> = [("CODE_INDEX_CLI_EMBEDDER_PROVIDER", "mistral")]
            .into_iter()
            .collect();
        let getenv = |key: &str| env.get(key).map(|v| v.to_string());
        let config = load(&dir, None, None, &getenv);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Mistral
        );
    }

    #[test]
    fn rejects_invalid_config() {
        let dir = temp_dir("invalid");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"search": {"minScore": 5.0}}"#,
        )
        .unwrap();
        let result = load_config_with(&dir, None, None, &no_env, &fake_user_config());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid configuration"));
    }

    #[test]
    fn rejects_unknown_provider() {
        let dir = temp_dir("bad-provider");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "not-a-provider"}}"#,
        )
        .unwrap();
        let result = load_config_with(&dir, None, None, &no_env, &fake_user_config());
        assert!(result.is_err());
    }

    #[test]
    fn malformed_json_falls_back_to_defaults() {
        let dir = temp_dir("malformed");
        std::fs::write(dir.join(".code-index.json"), "{ not json !!!").unwrap();
        let config = load(&dir, None, None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::OpenAi
        );
    }

    #[test]
    fn deep_merges_project_config_with_defaults() {
        let dir = temp_dir("deep-merge");
        std::fs::write(
            dir.join(".code-index.json"),
            r#"{"embedder": {"provider": "ollama"}, "search": {"minScore": 0.6}}"#,
        )
        .unwrap();
        let config = load(&dir, None, None, &no_env);
        assert_eq!(
            config.embedder.provider,
            crate::shared::embedding_models::EmbedderProvider::Ollama
        );
        assert_eq!(config.search.min_score, 0.6);
        // Untouched defaults survive the merge
        assert_eq!(config.qdrant.url, "http://localhost:6333");
        assert_eq!(config.search.max_results, 50);
    }

    #[test]
    fn project_config_path_defaults_to_primary_filename() {
        let dir = temp_dir("paths");
        assert_eq!(project_config_path(&dir), dir.join(".code-index.json"));
    }

    #[test]
    fn project_config_path_finds_existing_files() {
        let dir = temp_dir("paths-existing");
        std::fs::write(dir.join("code-index.config.json"), "{}").unwrap();
        assert_eq!(
            project_config_path(&dir),
            dir.join("code-index.config.json")
        );
        std::fs::write(dir.join(".code-index.json"), "{}").unwrap();
        assert_eq!(project_config_path(&dir), dir.join(".code-index.json"));
    }

    #[test]
    fn user_config_path_is_in_config_dir() {
        let path = user_config_path();
        let s = path.to_string_lossy();
        assert!(s.contains(".config"));
        assert!(s.contains("code-index"));
        assert!(s.ends_with("config.json"));
    }

    #[test]
    fn set_nested_value_coerces_types() {
        let mut obj = Map::new();
        set_nested_value(&mut obj, "a.b.c", "42");
        set_nested_value(&mut obj, "a.b.d", "0.5");
        set_nested_value(&mut obj, "a.e", "true");
        set_nested_value(&mut obj, "f", "hello");
        assert_eq!(obj["a"]["b"]["c"], Value::from(42));
        assert_eq!(obj["a"]["b"]["d"], Value::from(0.5));
        assert_eq!(obj["a"]["e"], Value::Bool(true));
        assert_eq!(obj["f"], Value::from("hello"));
    }
}
