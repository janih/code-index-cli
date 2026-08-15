//! code-index — standalone CLI for codebase indexing with semantic search.
//!
//! Rust rewrite of the TypeScript implementation on branch `first-version-build`.
//! See REWRITE-PLAN.md for the porting phases.

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
