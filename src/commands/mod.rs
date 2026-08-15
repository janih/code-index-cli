//! CLI command handlers.
//!
//! Port of `src/commands/*` from the TypeScript version. Each function here
//! corresponds to one commander action; behavior is stubbed until the
//! phases in REWRITE-PLAN.md wire the real services in.

use std::path::PathBuf;

use crate::cli::{EmbedderArgs, OutputFormat, QdrantArgs};
use crate::log;

pub fn init(workspace: PathBuf, force: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !force || workspace.exists(),
        "workspace does not exist: {}",
        workspace.display()
    );
    log::info(&format!(
        "init: not implemented yet (workspace: {})",
        workspace.display()
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn index(
    workspace: PathBuf,
    config: Option<PathBuf>,
    _embedder: EmbedderArgs,
    _qdrant: QdrantArgs,
    _batch_size: Option<u32>,
    _dry_run: bool,
) -> anyhow::Result<()> {
    log::info(&format!(
        "index: not implemented yet (workspace: {}, config: {:?})",
        workspace.display(),
        config
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn search(
    workspace: PathBuf,
    _config: Option<PathBuf>,
    query: String,
    _limit: Option<u32>,
    _format: OutputFormat,
    _directory: Option<String>,
    _embedder: EmbedderArgs,
    _qdrant: QdrantArgs,
) -> anyhow::Result<()> {
    log::info(&format!(
        "search: not implemented yet (workspace: {}, query: {query:?})",
        workspace.display()
    ));
    Ok(())
}

pub fn watch(
    workspace: PathBuf,
    _config: Option<PathBuf>,
    _embedder: EmbedderArgs,
    _qdrant: QdrantArgs,
    _batch_size: Option<u32>,
) -> anyhow::Result<()> {
    log::info(&format!(
        "watch: not implemented yet (workspace: {})",
        workspace.display()
    ));
    Ok(())
}

pub fn status(workspace: PathBuf, _config: Option<PathBuf>) -> anyhow::Result<()> {
    log::info(&format!(
        "status: not implemented yet (workspace: {})",
        workspace.display()
    ));
    Ok(())
}

pub fn clear(workspace: PathBuf, _config: Option<PathBuf>) -> anyhow::Result<()> {
    log::info(&format!(
        "clear: not implemented yet (workspace: {})",
        workspace.display()
    ));
    Ok(())
}
