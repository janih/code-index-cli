//! File processing pipeline: directory scanner, code parser, file watcher.
//!
//! Port of `src/processors/*`. Scanner walks with the `ignore` crate
//! (gitignore-aware), parser ports the line/markdown chunking, watcher uses
//! `notify`.
