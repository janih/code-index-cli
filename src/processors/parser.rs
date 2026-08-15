//! Line-based code parser.
//!
//! Port of `src/processors/parser.ts`. Tree-sitter is not used (same as the
//! TS version, which ships `web-tree-sitter` but never calls it).
//!
//! Deviation note: block sizes are measured in bytes (`str::len`), where the
//! TS version counts UTF-16 code units. Equal for ASCII; chunk boundaries may
//! differ slightly for non-ASCII content. Hashes and line numbers are
//! unaffected by this choice in deterministic ways only via content.

use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::log;
use crate::shared::constants::{MAX_BLOCK_CHARS, MAX_CHARS_TOLERANCE_FACTOR, MIN_BLOCK_CHARS};
use crate::shared::supported_extensions::is_supported_extension;
use crate::traits::{CodeBlock, CodeParser, ParseOptions};

/// Simplified code parser that uses line-based chunking.
pub struct LineCodeParser;

impl LineCodeParser {
    pub fn new() -> Self {
        Self
    }

    fn create_file_hash(content: &str) -> String {
        hex::encode(Sha256::digest(content.as_bytes()))
    }

    fn segment_hash(file_path: &str, start_line: usize, content: &str) -> String {
        hex::encode(Sha256::digest(
            format!("{file_path}:{start_line}:{content}").as_bytes(),
        ))
    }

    /// Parses file content into code blocks (markdown = section-based,
    /// everything else = line-based).
    fn parse_content(file_path: &str, content: &str, file_hash: String) -> Vec<CodeBlock> {
        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mut seen_segment_hashes = HashSet::new();

        if ext == "md" || ext == "markdown" {
            Self::parse_markdown_content(file_path, content, file_hash, &mut seen_segment_hashes)
        } else {
            Self::perform_fallback_chunking(file_path, content, file_hash, &mut seen_segment_hashes)
        }
    }

    fn push_block(
        results: &mut Vec<CodeBlock>,
        seen_segment_hashes: &mut HashSet<String>,
        file_path: &str,
        block_lines: &[String],
        start_line: usize,
        file_hash: &str,
    ) {
        if block_lines.is_empty() {
            return;
        }
        let block_content = block_lines.join("\n");
        if block_content.trim().len() < MIN_BLOCK_CHARS {
            return;
        }
        let segment_hash = Self::segment_hash(file_path, start_line, &block_content);
        if seen_segment_hashes.insert(segment_hash.clone()) {
            results.push(CodeBlock {
                file_path: file_path.to_string(),
                content: block_content,
                start_line,
                end_line: start_line + block_lines.len() - 1,
                segment_hash,
                file_hash: file_hash.to_string(),
            });
        }
    }

    /// Splits content into blocks of MAX_BLOCK_CHARS.
    fn perform_fallback_chunking(
        file_path: &str,
        content: &str,
        file_hash: String,
        seen_segment_hashes: &mut HashSet<String>,
    ) -> Vec<CodeBlock> {
        let mut results = Vec::new();
        let lines: Vec<&str> = content.split('\n').collect();

        let mut current_block_lines: Vec<String> = Vec::new();
        let mut current_block_start_line = 1usize;
        let mut current_size = 0usize;

        for (i, line) in lines.iter().enumerate() {
            let line_size = line.len() + 1; // +1 for newline

            if current_size + line_size > MAX_BLOCK_CHARS && current_size >= MIN_BLOCK_CHARS {
                Self::push_block(
                    &mut results,
                    seen_segment_hashes,
                    file_path,
                    &current_block_lines,
                    current_block_start_line,
                    &file_hash,
                );

                current_block_lines = Vec::new();
                current_block_start_line = i + 2; // 1-indexed
                current_size = 0;
            }

            current_block_lines.push(line.to_string());
            current_size += line_size;
        }

        // Handle remaining lines
        Self::push_block(
            &mut results,
            seen_segment_hashes,
            file_path,
            &current_block_lines,
            current_block_start_line,
            &file_hash,
        );

        results
    }

    fn is_markdown_header(line: &str) -> bool {
        // Matches the TS regex /^#{1,6}\s/
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        if hashes == 0 || hashes > 6 {
            return false;
        }
        line[hashes..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    }

    /// Parses markdown content into sections based on headers.
    fn parse_markdown_content(
        file_path: &str,
        content: &str,
        file_hash: String,
        seen_segment_hashes: &mut HashSet<String>,
    ) -> Vec<CodeBlock> {
        let mut results = Vec::new();
        let lines: Vec<&str> = content.split('\n').collect();
        let max_section_size = (MAX_BLOCK_CHARS as f64 * MAX_CHARS_TOLERANCE_FACTOR) as usize;

        let mut current_section: Vec<String> = Vec::new();
        let mut current_start_line = 1usize;

        for (i, line) in lines.iter().enumerate() {
            let is_header = Self::is_markdown_header(line);

            if is_header && !current_section.is_empty() {
                Self::push_block(
                    &mut results,
                    seen_segment_hashes,
                    file_path,
                    &current_section,
                    current_start_line,
                    &file_hash,
                );

                current_section = vec![line.to_string()];
                current_start_line = i + 1;
            } else {
                current_section.push(line.to_string());
            }

            // If section gets too large, split it
            if current_section.join("\n").len() > max_section_size {
                Self::push_block(
                    &mut results,
                    seen_segment_hashes,
                    file_path,
                    &current_section,
                    current_start_line,
                    &file_hash,
                );

                current_section = Vec::new();
                current_start_line = i + 2;
            }
        }

        // Handle remaining section
        Self::push_block(
            &mut results,
            seen_segment_hashes,
            file_path,
            &current_section,
            current_start_line,
            &file_hash,
        );

        results
    }
}

impl Default for LineCodeParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CodeParser for LineCodeParser {
    async fn parse_file(
        &self,
        file_path: &Path,
        options: Option<ParseOptions>,
    ) -> anyhow::Result<Vec<CodeBlock>> {
        let ext = format!(
            ".{}",
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase()
        );
        if !is_supported_extension(&ext) {
            return Ok(Vec::new());
        }

        let path_str = file_path.to_string_lossy().into_owned();

        let (content, file_hash) = match options.and_then(|o| o.content.map(|c| (c, o.file_hash))) {
            Some((content, file_hash)) => {
                let hash = file_hash.unwrap_or_else(|| Self::create_file_hash(&content));
                (content, hash)
            }
            None => match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let hash = Self::create_file_hash(&content);
                    (content, hash)
                }
                Err(err) => {
                    log::error(&format!("Error reading file {}: {}", path_str, err));
                    return Ok(Vec::new());
                }
            },
        };

        Ok(Self::parse_content(&path_str, &content, file_hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn parser() -> LineCodeParser {
        LineCodeParser::new()
    }

    /// Generates non-trivial content of at least `min_len` chars across lines.
    fn content_with_lines(min_lines: usize, line_len: usize) -> String {
        (1..=min_lines)
            .map(|i| format!("const value{i} = {}", "x".repeat(line_len)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn returns_empty_for_unsupported_extensions() {
        let blocks = parser()
            .parse_file(
                Path::new("/tmp/file.xyz"),
                Some(ParseOptions {
                    content: Some(
                        "hello world this is enough content to pass the minimum block size limit"
                            .into(),
                    ),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert!(blocks.is_empty());
    }

    #[tokio::test]
    async fn returns_empty_for_empty_and_whitespace_files() {
        for content in ["", "   \n\t  \n  "] {
            let blocks = parser()
                .parse_file(
                    Path::new("test.ts"),
                    Some(ParseOptions {
                        content: Some(content.into()),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            assert!(blocks.is_empty());
        }
    }

    #[tokio::test]
    async fn parses_typescript_files() {
        let content = content_with_lines(10, 20);
        let blocks = parser()
            .parse_file(
                Path::new("src/index.ts"),
                Some(ParseOptions {
                    content: Some(content.clone()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, content);
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 10);
        assert_eq!(blocks[0].file_path, "src/index.ts");
        assert!(!blocks[0].segment_hash.is_empty());
        assert!(!blocks[0].file_hash.is_empty());
    }

    #[tokio::test]
    async fn splits_large_files_into_multiple_blocks() {
        // 100 lines * ~60 chars ≈ 6000 chars > MAX_BLOCK_CHARS (1000)
        let content = content_with_lines(100, 50);
        let blocks = parser()
            .parse_file(
                Path::new("src/big.ts"),
                Some(ParseOptions {
                    content: Some(content),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert!(blocks.len() > 1);
        for block in &blocks {
            assert!(block.end_line >= block.start_line);
            assert!(block.content.trim().len() >= MIN_BLOCK_CHARS);
        }
        // Blocks are in order and non-overlapping
        for window in blocks.windows(2) {
            assert!(window[1].start_line > window[0].end_line);
        }
    }

    #[tokio::test]
    async fn accepts_content_via_options_without_reading_file() {
        let content = content_with_lines(5, 40);
        let blocks = parser()
            .parse_file(
                Path::new("src/nonexistent-on-disk.py"),
                Some(ParseOptions {
                    content: Some(content),
                    file_hash: Some("custom-hash".into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].file_hash, "custom-hash");
    }

    #[tokio::test]
    async fn generates_consistent_hashes_for_same_content() {
        let content = content_with_lines(5, 40);
        let parse = |c: &str| {
            let parser = parser();
            let content = c.to_string();
            async move {
                parser
                    .parse_file(
                        Path::new("src/a.ts"),
                        Some(ParseOptions {
                            content: Some(content),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap()
            }
        };
        let first = parse(&content).await;
        let second = parse(&content).await;
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn different_paths_produce_different_segment_hashes() {
        let content = content_with_lines(5, 40);
        let parse_at = |p: &str| {
            let parser = parser();
            let path = PathBuf::from(p);
            let content = content.clone();
            async move {
                parser
                    .parse_file(
                        &path,
                        Some(ParseOptions {
                            content: Some(content),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap()
            }
        };
        let a = parse_at("src/a.ts").await;
        let b = parse_at("src/b.ts").await;
        assert_eq!(a[0].content, b[0].content);
        assert_ne!(a[0].segment_hash, b[0].segment_hash);
        assert_eq!(a[0].file_hash, b[0].file_hash);
    }

    #[tokio::test]
    async fn returns_empty_for_nonexistent_files() {
        let blocks = parser()
            .parse_file(Path::new("/no/such/file.ts"), None)
            .await
            .unwrap();
        assert!(blocks.is_empty());
    }

    #[tokio::test]
    async fn parses_common_languages() {
        let content = content_with_lines(6, 30);
        for ext in ["py", "js", "json", "go", "java", "rs"] {
            let path = format!("src/file.{ext}");
            let blocks = parser()
                .parse_file(
                    Path::new(&path),
                    Some(ParseOptions {
                        content: Some(content.clone()),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            assert_eq!(blocks.len(), 1, "extension {ext} should parse");
        }
    }

    #[tokio::test]
    async fn parses_real_file_from_disk() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("code-index-parser-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("real.ts");
        let content = content_with_lines(8, 30);
        std::fs::write(&file, &content).unwrap();

        let blocks = parser().parse_file(&file, None).await.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, content);
        assert_eq!(
            blocks[0].file_hash,
            hex::encode(Sha256::digest(content.as_bytes()))
        );
    }

    #[tokio::test]
    async fn markdown_files_split_by_sections() {
        let markdown = (1..=5)
            .map(|i| {
                format!(
                    "# Section {i}\n\n{}\n",
                    "Some descriptive prose for this section. ".repeat(3)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let blocks = parser()
            .parse_file(
                Path::new("README.md"),
                Some(ParseOptions {
                    content: Some(markdown),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(blocks.len(), 5);
        assert!(blocks[0].content.starts_with("# Section 1"));
        assert!(blocks[4].content.starts_with("# Section 5"));
    }

    #[tokio::test]
    async fn markdown_oversized_sections_are_split() {
        // One section larger than MAX_BLOCK_CHARS * 1.15
        let prose = "word ".repeat(400); // ~2000 chars
        let markdown = format!(
            "# Big\n\n{prose}\n# After\n\n{}",
            "tail content ".repeat(10)
        );
        let blocks = parser()
            .parse_file(
                Path::new("big.md"),
                Some(ParseOptions {
                    content: Some(markdown),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert!(blocks.len() >= 2);
        assert!(blocks.iter().any(|b| b.content.starts_with("# Big")));
    }

    #[test]
    fn markdown_header_detection_matches_regex() {
        assert!(LineCodeParser::is_markdown_header("# Title"));
        assert!(LineCodeParser::is_markdown_header("###### Deep"));
        assert!(!LineCodeParser::is_markdown_header("####### Too many"));
        assert!(!LineCodeParser::is_markdown_header("#NoSpace"));
        assert!(!LineCodeParser::is_markdown_header("not a header"));
    }
}
