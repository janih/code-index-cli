//! Validation and error helpers — API key redaction and HTTP errors.
//!
//! Port of `src/shared/validation-helpers.ts`.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

static SK_KEY_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-[a-zA-Z0-9]{20,}").expect("valid regex"));
static BEARER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Bearer\s+[a-zA-Z0-9\-_.]{20,}").expect("valid regex"));
static API_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)api[_-]?key[=:]\s*["']?[a-zA-Z0-9]{20,}"#).expect("valid regex")
});

/// Removes potential API keys and tokens from an error message.
pub fn sanitize_error_message(message: &str) -> String {
    let message = SK_KEY_PATTERN.replace_all(message, "sk-***REDACTED***");
    let message = BEARER_PATTERN.replace_all(&message, "Bearer ***REDACTED***");
    let message = API_KEY_PATTERN.replace_all(&message, "api_key=***REDACTED***");
    message.into_owned()
}

/// Wraps a fallible operation, prefixing and sanitizing any error message.
pub fn with_validation_error_handling<T, E: fmt::Display>(
    operation: impl FnOnce() -> Result<T, E>,
    error_message: &str,
) -> anyhow::Result<T> {
    operation().map_err(|err| {
        anyhow::anyhow!(
            "{}: {}",
            error_message,
            sanitize_error_message(&err.to_string())
        )
    })
}

/// Formats an embedding error for display, sanitizing sensitive information.
pub fn format_embedding_error(provider: &str, error: &dyn fmt::Display) -> String {
    format!(
        "{} embedding error: {}",
        provider,
        sanitize_error_message(&error.to_string())
    )
}

/// HTTP error for API status codes (`HttpError` in TS).
#[derive(Debug)]
pub struct HttpError {
    pub status_code: u16,
    pub status_text: String,
    pub body: Option<String>,
}

impl HttpError {
    pub fn new(status_code: u16, status_text: impl Into<String>, body: Option<String>) -> Self {
        Self {
            status_code,
            status_text: status_text.into(),
            body,
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP {}: {}", self.status_code, self.status_text)
    }
}

impl std::error::Error for HttpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_normal_messages() {
        assert_eq!(
            sanitize_error_message("Something went wrong"),
            "Something went wrong"
        );
        assert_eq!(sanitize_error_message("File not found"), "File not found");
    }

    #[test]
    fn redacts_openai_style_keys() {
        let msg = "Error with key sk-abcdefghijklmnopqrstuvwxyz123456";
        assert_eq!(
            sanitize_error_message(msg),
            "Error with key sk-***REDACTED***"
        );
    }

    #[test]
    fn redacts_bearer_tokens() {
        let msg = "Authorization: Bearer abcdefghijklmnopqrstuvwxyz1234567890";
        assert_eq!(
            sanitize_error_message(msg),
            "Authorization: Bearer ***REDACTED***"
        );
    }

    #[test]
    fn redacts_api_key_formats() {
        assert!(
            sanitize_error_message("api_key=abcdefghijklmnopqrstuvwxyz1234567890")
                .contains("***REDACTED***")
        );
        assert!(
            sanitize_error_message("apiKey=abcdefghijklmnopqrstuvwxyz1234567890")
                .contains("***REDACTED***")
        );
        assert!(
            sanitize_error_message("api-key: abcdefghijklmnopqrstuvwxyz1234567890")
                .contains("***REDACTED***")
        );
    }

    #[test]
    fn handles_empty_and_multiple() {
        assert_eq!(sanitize_error_message(""), "");
        let msg = "Key sk-abcdefghijklmnopqrstuvwxyz123456 and Bearer abcdefghijklmnopqrstuvwxyz1234567890XYZ";
        let sanitized = sanitize_error_message(msg);
        assert!(sanitized.contains("***REDACTED***"));
        assert!(!sanitized.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    }

    #[test]
    fn with_validation_error_handling_success() {
        let result: anyhow::Result<i32> =
            with_validation_error_handling(|| Ok::<i32, &str>(42), "Operation failed");
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn with_validation_error_handling_wraps_and_sanitizes() {
        let err = with_validation_error_handling(
            || Err::<i32, _>("key sk-abcdefghijklmnopqrstuvwxyz123456789012345"),
            "Failed",
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("Failed: "));
        assert!(err.to_string().contains("***REDACTED***"));
    }

    #[test]
    fn format_embedding_error_sanitizes() {
        assert_eq!(
            format_embedding_error("OpenAI", &"timeout"),
            "OpenAI embedding error: timeout"
        );
        let result = format_embedding_error(
            "OpenAI",
            &"key sk-abcdefghijklmnopqrstuvwxyz123456789012345",
        );
        assert!(result.contains("***REDACTED***"));
    }

    #[test]
    fn http_error_carries_details() {
        let err = HttpError::new(404, "Not Found", Some("Response body".to_string()));
        assert_eq!(err.status_code, 404);
        assert_eq!(err.status_text, "Not Found");
        assert_eq!(err.body.as_deref(), Some("Response body"));
        assert_eq!(err.to_string(), "HTTP 404: Not Found");

        let err = HttpError::new(401, "Unauthorized", None);
        assert_eq!(err.body, None);
        assert_eq!(err.to_string(), "HTTP 401: Unauthorized");
    }
}
