//! Shared constants (index-format values; asserted at compile time below).

// Parser
pub const MAX_BLOCK_CHARS: usize = 1000;
pub const MIN_BLOCK_CHARS: usize = 50;
pub const MIN_CHUNK_REMAINDER_CHARS: usize = 200;
pub const MAX_CHARS_TOLERANCE_FACTOR: f64 = 1.15;

// Search
pub const MIN_SEARCH_RESULTS: u32 = 10;
pub const MAX_SEARCH_RESULTS: u32 = 200;
pub const DEFAULT_SEARCH_RESULTS: u32 = 50;
pub const SEARCH_RESULTS_STEP: u32 = 10;
pub const MIN_SEARCH_SCORE: f64 = 0.0;
pub const MAX_SEARCH_SCORE: f64 = 1.0;
pub const DEFAULT_SEARCH_MIN_SCORE: f64 = 0.4;
pub const SEARCH_SCORE_STEP: f64 = 0.05;

// File Watcher
pub const QDRANT_CODE_BLOCK_NAMESPACE: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
pub const MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024; // 1MB

// Directory Scanner
pub const MAX_LIST_FILES_LIMIT: u64 = 50_000;
pub const BATCH_SEGMENT_THRESHOLD: usize = 60;
pub const MAX_BATCH_RETRIES: u32 = 3;
pub const INITIAL_RETRY_DELAY_MS: u64 = 500;
pub const PARSING_CONCURRENCY: usize = 10;
pub const MAX_PENDING_BATCHES: usize = 20;

// OpenAI Embedder
pub const MAX_BATCH_TOKENS: usize = 100_000;
pub const MAX_ITEM_TOKENS: usize = 8191;
pub const BATCH_PROCESSING_CONCURRENCY: usize = 10;

// Gemini Embedder
pub const GEMINI_MAX_ITEM_TOKENS: usize = 2048;

// Compile-time invariants — a bad edit fails compilation.
const _: () = {
    // Invariant ordering
    assert!(MAX_BLOCK_CHARS > MIN_BLOCK_CHARS);
    assert!(DEFAULT_SEARCH_MIN_SCORE >= MIN_SEARCH_SCORE);
    assert!(DEFAULT_SEARCH_MIN_SCORE <= MAX_SEARCH_SCORE);
    assert!(MIN_SEARCH_RESULTS < DEFAULT_SEARCH_RESULTS);
    assert!(DEFAULT_SEARCH_RESULTS < MAX_SEARCH_RESULTS);

    // Pinned values (must match src/shared/constants.ts)
    assert!(MAX_BLOCK_CHARS == 1000 && MIN_BLOCK_CHARS == 50 && MIN_CHUNK_REMAINDER_CHARS == 200);
    assert!(MIN_SEARCH_RESULTS == 10 && MAX_SEARCH_RESULTS == 200 && DEFAULT_SEARCH_RESULTS == 50);
    assert!(DEFAULT_SEARCH_MIN_SCORE == 0.4);
    assert!(MAX_BATCH_RETRIES == 3 && INITIAL_RETRY_DELAY_MS == 500);
    assert!(PARSING_CONCURRENCY == 10 && MAX_PENDING_BATCHES == 20);
    assert!(
        MAX_BATCH_TOKENS == 100_000 && MAX_ITEM_TOKENS == 8191 && GEMINI_MAX_ITEM_TOKENS == 2048
    );
    assert!(
        MAX_FILE_SIZE_BYTES == 1_048_576
            && MAX_LIST_FILES_LIMIT == 50_000
            && BATCH_SEGMENT_THRESHOLD == 60
    );

    // Qdrant namespace for deterministic point IDs — changing it orphans
    // existing indexes.
    assert!(QDRANT_CODE_BLOCK_NAMESPACE.len() == 36);
    assert!(
        QDRANT_CODE_BLOCK_NAMESPACE.as_bytes()[8] == b'-'
            && QDRANT_CODE_BLOCK_NAMESPACE.as_bytes()[13] == b'-'
            && QDRANT_CODE_BLOCK_NAMESPACE.as_bytes()[18] == b'-'
            && QDRANT_CODE_BLOCK_NAMESPACE.as_bytes()[23] == b'-'
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qdrant_namespace_matches_ts_value() {
        assert_eq!(
            QDRANT_CODE_BLOCK_NAMESPACE,
            "f47ac10b-58cc-4372-a567-0e02b2c3d479"
        );
    }
}
