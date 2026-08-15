//! Supported file extensions for code indexing.

/// Extensions the scanner will look for.
pub const SCANNER_EXTENSIONS: &[&str] = &[
    // Web
    ".js",
    ".jsx",
    ".ts",
    ".tsx",
    ".mjs",
    ".cjs", //
    // Python
    ".py", //
    // Java / JVM
    ".java",
    ".kt",
    ".scala",
    ".groovy", //
    // C / C++
    ".c",
    ".h",
    ".cpp",
    ".hpp",
    ".cc",
    ".cxx", //
    // C#
    ".cs", //
    // Go
    ".go", //
    // Rust
    ".rs", //
    // Ruby
    ".rb", //
    // PHP
    ".php", //
    // Swift
    ".swift", //
    // Lua
    ".lua", //
    // Shell
    ".sh",
    ".bash",
    ".zsh", //
    // Config / Data
    ".json",
    ".yaml",
    ".yml",
    ".toml",
    ".xml",
    ".html",
    ".css",
    ".scss",
    ".less", //
    // Markdown
    ".md",
    ".markdown", //
    // Dart
    ".dart", //
    // Elixir
    ".ex",
    ".exs", //
    // Erlang
    ".erl", //
    // Haskell
    ".hs", //
    // R
    ".r", //
    // SQL
    ".sql", //
    // Visual Basic
    ".vb", //
    // Zig
    ".zig", //
    // Nix
    ".nix", //
    // Dockerfile
    ".dockerfile", //
    // Makefile
    ".makefile", //
    // Protobuf
    ".proto", //
    // GraphQL
    ".graphql",
    ".gql", //
    // Vue
    ".vue", //
    // Svelte
    ".svelte", //
    // Terraform
    ".tf",
    ".tfvars", //
    // CMake
    ".cmake", //
    // Gradle
    ".gradle", //
    // Properties / INI
    ".properties",
    ".ini",
    ".cfg",
    ".conf",
];

/// Extensions that always use fallback chunking instead of tree-sitter parsing.
pub const FALLBACK_EXTENSIONS: &[&str] = &[".vb", ".scala", ".swift"];

/// Whether a file extension should use fallback chunking.
pub fn should_use_fallback_chunking(extension: &str) -> bool {
    let extension = extension.to_lowercase();
    FALLBACK_EXTENSIONS.contains(&extension.as_str())
}

/// Whether a file extension is indexed by the scanner.
pub fn is_supported_extension(extension: &str) -> bool {
    let extension = extension.to_lowercase();
    SCANNER_EXTENSIONS.contains(&extension.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_extensions_is_non_empty() {
        assert!(!SCANNER_EXTENSIONS.is_empty());
    }

    #[test]
    fn scanner_extensions_all_start_with_dot() {
        for ext in SCANNER_EXTENSIONS {
            assert!(ext.starts_with('.'), "extension missing dot: {ext}");
        }
    }

    #[test]
    fn includes_common_web_extensions() {
        for ext in [".js", ".jsx", ".ts", ".tsx"] {
            assert!(SCANNER_EXTENSIONS.contains(&ext));
        }
    }

    #[test]
    fn includes_common_language_extensions() {
        for ext in [".py", ".java", ".go", ".rs", ".rb", ".php"] {
            assert!(SCANNER_EXTENSIONS.contains(&ext));
        }
    }

    #[test]
    fn includes_config_data_extensions() {
        for ext in [".json", ".yaml", ".yml", ".toml"] {
            assert!(SCANNER_EXTENSIONS.contains(&ext));
        }
    }

    #[test]
    fn includes_markdown_extensions() {
        for ext in [".md", ".markdown"] {
            assert!(SCANNER_EXTENSIONS.contains(&ext));
        }
    }

    #[test]
    fn has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for ext in SCANNER_EXTENSIONS {
            assert!(seen.insert(ext), "duplicate extension: {ext}");
        }
    }

    #[test]
    fn fallback_chunking_matches_case_insensitively() {
        assert!(should_use_fallback_chunking(".vb"));
        assert!(should_use_fallback_chunking(".VB"));
        assert!(should_use_fallback_chunking(".Scala"));
        assert!(!should_use_fallback_chunking(".rs"));
    }

    #[test]
    fn supported_extension_lookup_is_case_insensitive() {
        assert!(is_supported_extension(".RS"));
        assert!(is_supported_extension(".py"));
        assert!(!is_supported_extension(".exe"));
    }
}
