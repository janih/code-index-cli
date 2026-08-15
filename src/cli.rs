//! Command-line interface definitions (clap derive).
//!
//! Port of `src/index.ts` (commander) — commands and flags must stay
//! compatible with the TypeScript version.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands;

#[derive(Parser)]
#[command(
    name = "code-index",
    version,
    about = "Standalone CLI tool for codebase indexing with semantic search"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a configuration file in the current workspace
    Init {
        /// Workspace directory path
        #[arg(short, long, default_value = ".", value_name = "path")]
        workspace: PathBuf,

        /// Overwrite existing configuration file
        #[arg(long)]
        force: bool,
    },

    /// Scan workspace and index code files into Qdrant
    Index {
        #[command(flatten)]
        workspace: WorkspaceArgs,

        #[command(flatten)]
        embedder: EmbedderArgs,

        #[command(flatten)]
        qdrant: QdrantArgs,

        /// Batch size for embedding
        #[arg(long, value_name = "n")]
        batch_size: Option<u32>,

        /// Scan and parse only, no embedding
        #[arg(long)]
        dry_run: bool,
    },

    /// Search the indexed codebase using semantic search
    Search {
        #[command(flatten)]
        workspace: WorkspaceArgs,

        /// Search query
        #[arg(short, long, value_name = "text")]
        query: Option<String>,

        /// Maximum number of results
        #[arg(short = 'n', long, value_name = "n")]
        limit: Option<u32>,

        /// Output format: "text" or "json"
        #[arg(long, value_name = "format", default_value = "text")]
        format: OutputFormat,

        /// Filter by directory prefix
        #[arg(long, value_name = "prefix")]
        directory: Option<String>,

        #[command(flatten)]
        embedder: EmbedderArgs,

        #[command(flatten)]
        qdrant: QdrantArgs,
    },

    /// Index workspace and watch for file changes
    Watch {
        #[command(flatten)]
        workspace: WorkspaceArgs,

        #[command(flatten)]
        embedder: EmbedderArgs,

        #[command(flatten)]
        qdrant: QdrantArgs,

        /// Batch size for embedding
        #[arg(long, value_name = "n")]
        batch_size: Option<u32>,
    },

    /// Show current indexing status and configuration
    Status {
        #[command(flatten)]
        workspace: WorkspaceArgs,
    },

    /// Delete the indexed data and clear cache
    Clear {
        #[command(flatten)]
        workspace: WorkspaceArgs,
    },
}

/// Flags shared by all workspace-scoped commands.
///
/// Matches `-w/--workspace`, `-c/--config`, `-d/--debug` in the TS CLI.
/// (`init` is the exception: it takes workspace but no config/debug.)
#[derive(Args)]
pub struct WorkspaceArgs {
    /// Workspace directory path
    #[arg(short, long, default_value = ".", value_name = "path")]
    pub workspace: PathBuf,

    /// Path to configuration file
    #[arg(short, long, value_name = "path")]
    pub config: Option<PathBuf>,

    /// Enable verbose debug output
    #[arg(short, long)]
    pub debug: bool,
}

/// Embedder selection flags (`--provider`, `--model`, `--api-key`).
#[derive(Args)]
pub struct EmbedderArgs {
    /// Embedder provider (openai, ollama, gemini, etc.)
    #[arg(long, value_name = "provider")]
    pub provider: Option<String>,

    /// Embedding model ID
    #[arg(long = "model", value_name = "model-id")]
    pub model_id: Option<String>,

    /// API key for embedder provider
    #[arg(long, value_name = "key")]
    pub api_key: Option<String>,
}

/// Qdrant connection flags (`--qdrant-url`, `--qdrant-api-key`).
#[derive(Args)]
pub struct QdrantArgs {
    /// Qdrant server URL
    #[arg(long, value_name = "url")]
    pub qdrant_url: Option<String>,

    /// Qdrant API key
    #[arg(long, value_name = "key")]
    pub qdrant_api_key: Option<String>,
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Parses CLI arguments and dispatches to the command handlers.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(dispatch(cli))
}

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init { workspace, force } => {
            commands::init(workspace, force).await?;
        }
        Commands::Index {
            workspace,
            embedder,
            qdrant,
            batch_size,
            dry_run,
        } => {
            crate::log::set_debug(workspace.debug);
            commands::index(
                workspace.workspace,
                workspace.config,
                embedder,
                qdrant,
                batch_size,
                dry_run,
            )
            .await?;
        }
        Commands::Search {
            workspace,
            query,
            limit,
            format,
            directory,
            embedder,
            qdrant,
        } => {
            crate::log::set_debug(workspace.debug);
            let query = match query {
                Some(query) => query,
                None => {
                    anyhow::bail!("Search query is required. Use -q or --query.");
                }
            };
            commands::search(
                workspace.workspace,
                workspace.config,
                query,
                limit,
                format,
                directory,
                embedder,
                qdrant,
            )
            .await?;
        }
        Commands::Watch {
            workspace,
            embedder,
            qdrant,
            batch_size,
        } => {
            crate::log::set_debug(workspace.debug);
            commands::watch(
                workspace.workspace,
                workspace.config,
                embedder,
                qdrant,
                batch_size,
            )
            .await?;
        }
        Commands::Status { workspace } => {
            crate::log::set_debug(workspace.debug);
            commands::status(workspace.workspace, workspace.config).await?;
        }
        Commands::Clear { workspace } => {
            crate::log::set_debug(workspace.debug);
            commands::clear(workspace.workspace, workspace.config).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli_structure() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_search_with_all_flags() {
        let cli = Cli::try_parse_from([
            "code-index",
            "search",
            "-w",
            "/tmp/ws",
            "-c",
            "cfg.json",
            "-q",
            "how is auth done",
            "-n",
            "20",
            "--format",
            "json",
            "--directory",
            "src",
            "--provider",
            "openai",
            "--model",
            "text-embedding-3-small",
            "--api-key",
            "sk-test",
            "--qdrant-url",
            "http://localhost:6333",
            "--qdrant-api-key",
            "qk",
            "-d",
        ])
        .expect("search invocation should parse");

        let Commands::Search {
            workspace,
            query,
            limit,
            embedder,
            qdrant,
            ..
        } = cli.command
        else {
            panic!("expected search command");
        };

        assert_eq!(workspace.workspace, PathBuf::from("/tmp/ws"));
        assert!(workspace.debug);
        assert_eq!(query.as_deref(), Some("how is auth done"));
        assert_eq!(limit, Some(20));
        assert_eq!(embedder.provider.as_deref(), Some("openai"));
        assert_eq!(embedder.model_id.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(qdrant.qdrant_url.as_deref(), Some("http://localhost:6333"));
    }

    #[test]
    fn index_supports_dry_run_and_batch_size() {
        let cli = Cli::try_parse_from(["code-index", "index", "--dry-run", "--batch-size", "32"])
            .expect("index invocation should parse");

        let Commands::Index {
            batch_size,
            dry_run,
            ..
        } = cli.command
        else {
            panic!("expected index command");
        };

        assert_eq!(batch_size, Some(32));
        assert!(dry_run);
    }

    #[test]
    fn init_has_no_config_or_debug_flags() {
        // TS parity: `init` accepts only --workspace and --force.
        assert!(Cli::try_parse_from(["code-index", "init", "--debug"]).is_err());
        assert!(Cli::try_parse_from(["code-index", "init", "--config", "x"]).is_err());
        assert!(Cli::try_parse_from(["code-index", "init", "-w", "/tmp/ws", "--force"]).is_ok());
    }
}
