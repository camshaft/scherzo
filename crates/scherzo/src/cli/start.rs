use crate::config::Config;
use anyhow::{Context, Result};
use clap::Args;
use std::{fs, path::PathBuf};
use wasmtime::{
    Config as WasmtimeConfig, Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(Args)]
pub struct StartArgs {
    /// Path to the configuration file (TOML or JSON).
    pub config: PathBuf,
}

/// State for the boot plugins environment
pub struct PluginState {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
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

        // Create boot plugin environment
        let plugin_linker = create_plugin_linker(&engine)?;

        // Create print job environment
        let _job_linker = create_job_linker(&engine)?;

        // Load boot plugins if specified in config
        for plugin_path in &config.plugins {
            load_boot_plugin(&engine, &plugin_linker, plugin_path)?;
        }

        tracing::info!("Scherzo runtime initialized");

        // Start the HTTP server
        start_server(config)
    }
}

/// Start the HTTP server
#[tokio::main]
async fn start_server(config: Config) -> Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind to {}", addr))?;

    tracing::info!("Server listening on {}", addr);

    // Create app state and router
    let state = crate::server::AppState::new(config)?;
    let app = crate::server::create_router(state);

    // Run the server
    axum::serve(listener, app).await.context("server error")?;

    Ok(())
}

/// Create a linker for boot plugins with WASI support
fn create_plugin_linker(engine: &Engine) -> Result<Linker<PluginState>> {
    let mut linker = Linker::new(engine);

    // Add WASI to the linker
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .context("failed to add WASI to plugin linker")?;

    // TODO: Add custom host functions for plugin system interaction

    Ok(linker)
}

/// Create a linker for print jobs with command dispatch support
fn create_job_linker(engine: &Engine) -> Result<Linker<JobState>> {
    let linker = Linker::new(engine);

    // TODO: Add command dispatch interface for jobs

    Ok(linker)
}

/// Load and initialize a boot plugin
fn load_boot_plugin(
    engine: &Engine,
    linker: &Linker<PluginState>,
    plugin_path: &str,
) -> Result<()> {
    println!("Loading boot plugin: {}", plugin_path);

    let wasm_bytes = fs::read(plugin_path)
        .with_context(|| format!("failed to read plugin file {}", plugin_path))?;

    let component = Component::from_binary(engine, &wasm_bytes)
        .with_context(|| format!("failed to compile plugin component {}", plugin_path))?;

    // Create store for this plugin
    let wasi = WasiCtxBuilder::new().inherit_stdio().inherit_env().build();
    let table = ResourceTable::new();
    let state = PluginState { wasi, table };
    let mut store = Store::new(engine, state);

    // Instantiate the component
    let _instance = linker
        .instantiate(&mut store, &component)
        .with_context(|| format!("failed to instantiate plugin {}", plugin_path))?;

    println!("Successfully loaded plugin: {}", plugin_path);

    // TODO: Call plugin initialization function

    Ok(())
}
