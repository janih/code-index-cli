//! code-index — standalone CLI for codebase indexing with semantic search.
//!
//! Rust rewrite of the TypeScript implementation on branch `first-version-build`.
//! See REWRITE-PLAN.md for the porting phases.

mod cache;
mod cli;
mod commands;
mod config;
mod core;
mod embedders;
mod log;
mod processors;
mod shared;
mod traits;
mod vector_store;

fn main() -> std::process::ExitCode {
    match cli::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            log::error(&err.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}
