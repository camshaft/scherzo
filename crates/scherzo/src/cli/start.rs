use crate::{
    config::Config,
    plugin::{PluginManager, PluginRegistry},
};
use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;
use wasmtime::{
    Config as WasmtimeConfig, Engine,
    component::{Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

#[derive(Args)]
pub struct StartArgs {
    /// Path to the configuration file (TOML or JSON).
    pub config: PathBuf,
}

/// State for the print job environment
pub struct JobState {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for JobState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl StartArgs {
    pub fn run(&self) -> Result<()> {
        // Initialize tracing
        tracing_subscriber::fmt::init();

        // Load and parse the config file
        let config = Config::from_file(&self.config)?;
        config.validate()?;

        tracing::info!("Starting scherzo with config: {}", self.config.display());
        tracing::info!(
            "Server will bind to {}:{}",
            config.server.host,
            config.server.port
        );

        // Set up wasmtime configuration
        let mut wasmtime_config = WasmtimeConfig::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.async_support(false);

        let engine = Engine::new(&wasmtime_config).context("failed to create wasmtime engine")?;

        // Create plugin manager
        let mut plugin_manager = PluginManager::new(engine.clone());

        // Load boot plugins if specified in config
        for plugin_path in &config.plugins {
            // TODO: Load plugin-specific config from main config
            let plugin_config = "{}"; // Empty JSON object for now
            match plugin_manager.load_plugin(plugin_path, plugin_config) {
                Ok(info) => {
                    tracing::info!("Loaded plugin: {} v{}", info.name, info.version);
                }
                Err(e) => {
                    tracing::error!("Failed to load plugin {}: {}", plugin_path, e);
                    // Continue loading other plugins instead of failing completely
                }
            }
        }

        // Log registered schemas and handlers
        let registry = plugin_manager.registry();
        let schemas = registry.get_config_schemas();
        let handlers = registry.get_command_handlers();
        tracing::info!("Registered {} config schemas", schemas.len());
        tracing::info!("Registered {} command handlers", handlers.len());

        // Create print job environment
        let _job_linker = create_job_linker(&engine)?;

        tracing::info!("Scherzo runtime initialized");

        // Extract registry before moving plugin_manager
        let registry = plugin_manager.registry().clone();

        // Start the HTTP server with the plugin registry
        start_server(config, registry)
    }
}

/// Start the HTTP server
#[tokio::main]
async fn start_server(config: Config, plugin_registry: PluginRegistry) -> Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind to {}", addr))?;

    tracing::info!("Server listening on {}", addr);

    // Create app state and router
    let state = crate::server::AppState::new(config, plugin_registry)?;
    let app = crate::server::create_router(state);

    // Run the server
    axum::serve(listener, app).await.context("server error")?;

    Ok(())
}

/// Create a linker for print jobs with command dispatch support
fn create_job_linker(engine: &Engine) -> Result<Linker<JobState>> {
    let linker = Linker::new(engine);

    // TODO: Add command dispatch interface for jobs

    Ok(linker)
}
