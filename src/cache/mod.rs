//! File-hash cache with debounced disk persistence.
//!
//! Port of `src/cache/cache-manager.ts`.

pub mod manager;

pub use manager::HashCacheManager;
