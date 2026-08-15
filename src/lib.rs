//! code-index — standalone CLI for codebase indexing with semantic search.
//!
//! Rust rewrite of the TypeScript implementation on branch `first-version-build`
//! (behavioral reference). Intentional differences are listed in AGENTS.md.

pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod core;
pub mod embedders;
pub mod log;
pub mod processors;
pub mod shared;
pub mod traits;
pub mod vector_store;
