//! CLI command handlers.
//!
//! Port of `src/commands/*` from the TypeScript version. Each function
//! corresponds to one commander action; `cli::run` drives them on a tokio
//! runtime.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cache::HashCacheManager;
use crate::cli::{EmbedderArgs, OutputFormat, QdrantArgs};
use crate::config::loader::{load_config, CliFlags};
use crate::config::manager::ConfigManager;
use crate::core::state_manager::IndexingState;
use crate::core::{SearchService, ServiceFactory, StateManager};
use crate::traits::{CacheManager, VectorStoreSearchResult};

/// `code-index init`: writes a template `.code-index.json` (TS `init`).
pub async fn init(workspace: PathBuf, force: bool) -> anyhow::Result<()> {
    let config_path = crate::config::loader::project_config_path(&workspace);

    if config_path.exists() && !force {
        println!(
            "⚠️  Configuration file already exists at {}",
            config_path.display()
        );
        println!("   Use --force to overwrite.");
        return Ok(());
    }

    let template = serde_json::json!({
        "enabled": true,
        "embedder": {
            "provider": "openai",
        },
        "qdrant": {
            "url": "http://localhost:6333",
        },
        "search": {
            "minScore": 0.4,
            "maxResults": 50,
        },
        "indexing": {
            "batchSize": 60,
            "maxFileSizeBytes": 1048576,
            "excludePatterns": [],
        },
    });

    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&template)? + "\n",
    )?;

    println!("✅ Configuration file created at {}", config_path.display());
    println!();
    println!("Next steps:");
    println!("  1. Edit the config file to set your embedder provider and API key");
    println!("     (or set environment variables like CODE_INDEX_CLI_EMBEDDER_API_KEY)");
    println!("  2. Make sure Qdrant is running: docker run -p 6333:6333 qdrant/qdrant");
    println!("  3. Run: code-index index");
    Ok(())
}

/// `code-index index`: scan workspace and index code files (TS `index-cmd`).
pub async fn index(
    workspace: PathBuf,
    config_path: Option<PathBuf>,
    embedder: EmbedderArgs,
    qdrant: QdrantArgs,
    batch_size: Option<u32>,
    dry_run: bool,
) -> anyhow::Result<()> {
    // TS accepts --dry-run but never wires it into the pipeline; mirror with a
    // warning instead of silently misindexing.
    if dry_run {
        crate::log::warn("--dry-run is accepted for compatibility but not implemented yet");
    }

    println!("🔍 Starting codebase indexing...");
    println!("   Workspace: {}", workspace.display());

    let flags = CliFlags {
        provider: embedder.provider,
        model_id: embedder.model_id,
        api_key: embedder.api_key,
        qdrant_url: qdrant.qdrant_url,
        qdrant_api_key: qdrant.qdrant_api_key,
        batch_size,
        ..Default::default()
    };
    let config_manager = ConfigManager::new(load_config(
        &workspace,
        config_path.as_deref(),
        Some(&flags),
    )?);

    if !config_manager.is_feature_configured() {
        anyhow::bail!("Code index is not configured. Run 'code-index init' first.");
    }

    println!("   Provider: {}", config_manager.embedder_provider());
    println!(
        "   Model: {}",
        config_manager.model_id().unwrap_or("(default)")
    );
    println!("   Qdrant: {}", config_manager.qdrant_url());
    println!();

    let cache_manager = Arc::new(HashCacheManager::new(&workspace, None));
    cache_manager.initialize();

    let state_manager = Arc::new(StateManager::new());
    let factory = ServiceFactory::new(
        config_manager,
        workspace.clone(),
        Arc::clone(&cache_manager),
    );

    // Validate embedder
    println!("⏳ Validating embedder configuration...");
    let embedder = factory.create_embedder()?;
    let validation = factory.validate_embedder(embedder.as_ref()).await;
    if !validation.valid {
        anyhow::bail!(
            "Embedder validation failed: {}",
            validation.error.unwrap_or_default()
        );
    }
    println!("✅ Embedder configuration valid");
    println!();

    // Progress reporting like the TS progressUpdate handler
    state_manager.on_progress_update(Box::new(move |status| {
        if status.system_status == IndexingState::Indexing && status.total_items > 0 {
            let percent = status.processed_items * 100 / status.total_items.max(1);
            print!(
                "\r   Progress: {}/{} {} ({}%) - {}",
                status.processed_items,
                status.total_items,
                status.current_item_unit,
                percent,
                status.message
            );
        }
    }));

    let orchestrator = factory.create_orchestrator(Arc::clone(&state_manager))?;

    let result = orchestrator.start_indexing().await;
    cache_manager.flush().await.ok();
    println!();
    result?;
    println!("✅ Indexing complete!");
    Ok(())
}

/// `code-index search`: semantic search over the index (TS `search`).
#[allow(clippy::too_many_arguments)]
pub async fn search(
    workspace: PathBuf,
    config_path: Option<PathBuf>,
    query: String,
    limit: Option<u32>,
    format: OutputFormat,
    directory: Option<String>,
    embedder: EmbedderArgs,
    qdrant: QdrantArgs,
) -> anyhow::Result<()> {
    let flags = CliFlags {
        provider: embedder.provider,
        model_id: embedder.model_id,
        api_key: embedder.api_key,
        qdrant_url: qdrant.qdrant_url,
        qdrant_api_key: qdrant.qdrant_api_key,
        ..Default::default()
    };
    let config_manager = ConfigManager::new(load_config(
        &workspace,
        config_path.as_deref(),
        Some(&flags),
    )?);

    if !config_manager.is_feature_configured() {
        anyhow::bail!("Code index is not configured. Run 'code-index init' first.");
    }

    let cache_manager = Arc::new(HashCacheManager::new(&workspace, None));
    cache_manager.initialize();

    let state_manager = Arc::new(StateManager::new());
    state_manager.set_system_state(IndexingState::Indexed, Some("Ready for search"));

    let factory = ServiceFactory::new(
        config_manager.clone(),
        workspace.clone(),
        Arc::clone(&cache_manager),
    );
    let embedder = factory.create_embedder()?;
    let vector_store = factory.create_vector_store()?;
    vector_store.initialize().await?;

    let search_service = SearchService::new(config_manager, state_manager, embedder, vector_store);

    let results = search_service
        .search_index(&query, directory.as_deref(), limit)
        .await?;

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&results)?),
        OutputFormat::Text => display_results(&query, &results, limit),
    }
    Ok(())
}

/// Text output identical to the TS `displayResults` helper.
fn display_results(query: &str, results: &[VectorStoreSearchResult], limit: Option<u32>) {
    let display = results
        .iter()
        .take(limit.map(|l| l as usize).unwrap_or(results.len()))
        .collect::<Vec<_>>();

    println!();
    println!("🔍 Search results for: \"{query}\"");
    match limit {
        Some(_) => println!(
            "   Found {} results (showing {})",
            results.len(),
            display.len()
        ),
        None => println!("   Found {} results", results.len()),
    }
    println!();

    for (i, result) in display.iter().enumerate() {
        let score = result.score * 100.0;
        let file_path = result
            .payload
            .as_ref()
            .map(|p| p.file_path.as_str())
            .unwrap_or("unknown");
        println!("  {}. [{score:.1}%] {file_path}", i + 1);
        if let Some(payload) = &result.payload {
            if payload.start_line > 0 && payload.end_line > 0 {
                println!("     Lines {}-{}", payload.start_line, payload.end_line);
            }
            let preview = payload
                .code_chunk
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join("\n     ");
            if !preview.is_empty() {
                println!("     {preview}");
            }
        }
        println!();
    }
}

/// `code-index watch` — implemented in Phase 3.
pub async fn watch(
    _workspace: PathBuf,
    _config_path: Option<PathBuf>,
    _embedder: EmbedderArgs,
    _qdrant: QdrantArgs,
    _batch_size: Option<u32>,
) -> anyhow::Result<()> {
    anyhow::bail!("watch mode is not implemented yet in the Rust version (Phase 3)")
}

/// `code-index status`: show configuration and index status (TS `status`).
pub async fn status(workspace: PathBuf, config_path: Option<PathBuf>) -> anyhow::Result<()> {
    let config_manager = ConfigManager::new(load_config(&workspace, config_path.as_deref(), None)?);

    println!("📊 Codebase Index Status");
    println!();
    println!("  Workspace:      {}", workspace.display());
    println!(
        "  Configured:     {}",
        if config_manager.is_feature_configured() {
            "✅ Yes"
        } else {
            "❌ No"
        }
    );
    println!("  Provider:       {}", config_manager.embedder_provider());
    println!(
        "  Model:          {}",
        config_manager.model_id().unwrap_or("(default)")
    );
    println!("  Qdrant URL:     {}", config_manager.qdrant_url());
    println!("  Search Score:   {}", config_manager.search_min_score());
    println!("  Max Results:    {}", config_manager.search_max_results());
    println!("  Batch Size:     {}", config_manager.batch_size());
    println!();

    if config_manager.is_feature_configured() {
        let cache_manager = Arc::new(HashCacheManager::new(&workspace, None));
        cache_manager.initialize();

        let connect = async {
            let factory = ServiceFactory::new(
                config_manager,
                workspace.clone(),
                Arc::clone(&cache_manager),
            );
            let vector_store = factory.create_vector_store()?;

            if vector_store.collection_exists().await? {
                println!("  Collection:     ✅ Exists");
                let has_data = vector_store.has_indexed_data().await?;
                println!(
                    "  Has Data:       {}",
                    if has_data { "✅ Yes" } else { "❌ No" }
                );
                println!("  Cached Files:   {}", cache_manager.get_all_hashes().len());
            } else {
                println!("  Collection:     ❌ Not found");
            }
            Ok::<(), anyhow::Error>(())
        };

        if let Err(err) = connect.await {
            println!("  Qdrant:         ❌ Cannot connect ({err})");
        }
    }

    Ok(())
}

/// `code-index clear`: remove all index data (TS `clear`).
pub async fn clear(workspace: PathBuf, config_path: Option<PathBuf>) -> anyhow::Result<()> {
    println!("🗑️  Clearing index data...");

    let config_manager = ConfigManager::new(load_config(&workspace, config_path.as_deref(), None)?);

    let cache_manager = Arc::new(HashCacheManager::new(&workspace, None));
    cache_manager.initialize();

    if config_manager.is_feature_configured() {
        let state_manager = Arc::new(StateManager::new());
        let factory = ServiceFactory::new(
            config_manager,
            workspace.clone(),
            Arc::clone(&cache_manager),
        );
        let orchestrator = factory.create_orchestrator(state_manager)?;
        orchestrator.clear_index_data().await?;
    } else {
        cache_manager.clear_cache_file();
    }

    println!("✅ Index data cleared successfully.");
    Ok(())
}
