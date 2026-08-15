//! File processing pipeline: directory scanner, code parser, file watcher.
//!
//! Scanner walks with the `ignore` crate (gitignore-aware), parser does
//! line/markdown chunking, watcher uses `notify`.

pub mod parser;
pub mod scanner;
pub mod watcher;
