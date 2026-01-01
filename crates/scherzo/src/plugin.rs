/// Plugin loading and management system
///
/// This module handles loading WebAssembly plugins, managing their lifecycle,
/// and maintaining registries for config schemas and command handlers.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use wasmtime::{
    Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};

// Generate WIT bindings using wasmtime's bindgen! macro
wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
});

// Re-export types from the generated bindings for the host side
pub use scherzo::plugin::types::{
    CommandHandler as WitCommandHandler, FieldDef as WitFieldDef, FieldType as WitFieldType,
    Schema as WitSchema,
};

/// Plugin metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

/// Schema definition for configuration or command parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// JSON Schema as a string
    pub json_schema: String,
    /// Human-readable description
    pub description: Option<String>,
}

impl From<WitSchema> for Schema {
    fn from(schema: WitSchema) -> Self {
        Self {
            json_schema: schema.json_schema,
            description: schema.description,
        }
    }
}

/// Field type for command parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldType {
    Int,
    Float,
    String,
    Bool,
    ListInt,
    ListFloat,
    ListString,
}

impl From<WitFieldType> for FieldType {
    fn from(ft: WitFieldType) -> Self {
        match ft {
            WitFieldType::Integer => FieldType::Int,
            WitFieldType::Floating => FieldType::Float,
            WitFieldType::Text => FieldType::String,
            WitFieldType::Boolean => FieldType::Bool,
            WitFieldType::ListInteger => FieldType::ListInt,
            WitFieldType::ListFloating => FieldType::ListFloat,
            WitFieldType::ListText => FieldType::ListString,
        }
    }
}

/// Field definition for a command parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

impl From<WitFieldDef> for FieldDef {
    fn from(fd: WitFieldDef) -> Self {
        Self {
            name: fd.name,
            field_type: fd.field_type.into(),
            required: fd.required,
            description: fd.description,
            default_value: fd.default_value,
        }
    }
}

/// Handler for a G-code command or high-level command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHandler {
    pub command: String,
    pub params: Vec<FieldDef>,
    pub description: Option<String>,
    pub scheduling_class: String,
}

impl From<WitCommandHandler> for CommandHandler {
    fn from(ch: WitCommandHandler) -> Self {
        Self {
            command: ch.command,
            params: ch.params.into_iter().map(Into::into).collect(),
            description: ch.description,
            scheduling_class: ch.scheduling_class,
        }
    }
}

/// Registry for plugin-provided schemas and handlers
#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    /// Registered config schemas by namespace
    config_schemas: Arc<RwLock<HashMap<String, Schema>>>,
    /// Registered command handlers by handler ID
    command_handlers: Arc<RwLock<HashMap<u32, CommandHandler>>>,
    /// Next handler ID to assign
    #[allow(dead_code)] // Used by register_command_handler
    next_handler_id: Arc<RwLock<u32>>,
    /// Loaded plugins by plugin ID
    plugins: Arc<RwLock<HashMap<String, PluginInfo>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a configuration schema
    #[allow(dead_code)] // Part of public plugin API, will be used by WIT bindings
    pub fn register_config_schema(&self, namespace: String, schema: Schema) -> Result<()> {
        let mut schemas = self.config_schemas.write().unwrap();
        if schemas.contains_key(&namespace) {
            bail!(
                "Config schema for namespace '{}' already registered",
                namespace
            );
        }
        schemas.insert(namespace, schema);
        Ok(())
    }

    /// Register a command handler
    #[allow(dead_code)] // Part of public plugin API, will be used by WIT bindings
    pub fn register_command_handler(&self, handler: CommandHandler) -> Result<u32> {
        let mut handlers = self.command_handlers.write().unwrap();
        let mut next_id = self.next_handler_id.write().unwrap();

        let handler_id = *next_id;
        *next_id += 1;

        handlers.insert(handler_id, handler);
        Ok(handler_id)
    }

    /// Unregister a command handler
    #[allow(dead_code)] // Part of public plugin API, will be used by WIT bindings
    pub fn unregister_command_handler(&self, handler_id: u32) -> Result<()> {
        let mut handlers = self.command_handlers.write().unwrap();
        if handlers.remove(&handler_id).is_none() {
            bail!("Command handler {} not found", handler_id);
        }
        Ok(())
    }

    /// Register a plugin
    pub fn register_plugin(&self, info: PluginInfo) -> Result<()> {
        let mut plugins = self.plugins.write().unwrap();
        if plugins.contains_key(&info.id) {
            bail!("Plugin '{}' already registered", info.id);
        }
        plugins.insert(info.id.clone(), info);
        Ok(())
    }

    /// Get all registered config schemas
    pub fn get_config_schemas(&self) -> HashMap<String, Schema> {
        self.config_schemas.read().unwrap().clone()
    }

    /// Get all registered command handlers
    pub fn get_command_handlers(&self) -> HashMap<u32, CommandHandler> {
        self.command_handlers.read().unwrap().clone()
    }

    /// Get all loaded plugins
    #[allow(dead_code)] // Part of public plugin API, may be used for introspection
    pub fn get_plugins(&self) -> HashMap<String, PluginInfo> {
        self.plugins.read().unwrap().clone()
    }
}

/// State for plugin WASM instances
pub struct PluginState {
    wasi: WasiCtx,
    table: ResourceTable,
    #[allow(dead_code)] // Will be used by host function implementations
    registry: PluginRegistry,
}

impl PluginState {
    pub fn new(registry: PluginRegistry) -> Self {
        let wasi = WasiCtxBuilder::new().inherit_stdio().inherit_env().build();
        let table = ResourceTable::new();

        Self {
            wasi,
            table,
            registry,
        }
    }
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Plugin manager for loading and managing plugins
pub struct PluginManager {
    engine: Engine,
    registry: PluginRegistry,
}

impl PluginManager {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            registry: PluginRegistry::new(),
        }
    }

    /// Get a reference to the plugin registry
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// Load a plugin from a WebAssembly component file
    pub fn load_plugin(&mut self, path: &str, _config: &str) -> Result<PluginInfo> {
        tracing::info!("Loading plugin from: {}", path);

        // Read the plugin file
        let wasm_bytes =
            std::fs::read(path).with_context(|| format!("Failed to read plugin file: {}", path))?;

        // Compile the component
        let component = Component::from_binary(&self.engine, &wasm_bytes)
            .with_context(|| format!("Failed to compile plugin component: {}", path))?;

        // Create a linker with the registry interface
        let linker = self.create_plugin_linker()?;

        // Create store with plugin state
        let state = PluginState::new(self.registry.clone());
        let mut store = Store::new(&self.engine, state);

        // Instantiate the component
        let _instance = linker
            .instantiate(&mut store, &component)
            .with_context(|| format!("Failed to instantiate plugin: {}", path))?;

        // TODO: Call get-info to get plugin metadata
        // TODO: Call init with the config
        // For now, create placeholder info
        let info = PluginInfo {
            id: format!("plugin-{}", path),
            name: path.to_string(),
            version: "0.1.0".to_string(),
            description: Some(format!("Plugin loaded from {}", path)),
        };

        // Register the plugin
        self.registry.register_plugin(info.clone())?;

        tracing::info!("Successfully loaded plugin: {}", info.name);
        Ok(info)
    }

    /// Create a linker for plugins with host functions
    fn create_plugin_linker(&self) -> Result<Linker<PluginState>> {
        let mut linker = Linker::new(&self.engine);

        // Add WASI support
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .context("Failed to add WASI to plugin linker")?;

        // TODO: Add plugin registry functions
        // This will require using wasmtime's component model bindings
        // For now, we have the structure in place

        Ok(linker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_config_schema() {
        let registry = PluginRegistry::new();

        let schema = Schema {
            json_schema: r#"{"type": "object"}"#.to_string(),
            description: Some("Test schema".to_string()),
        };

        assert!(
            registry
                .register_config_schema("test".to_string(), schema.clone())
                .is_ok()
        );
        assert!(
            registry
                .register_config_schema("test".to_string(), schema)
                .is_err()
        );

        let schemas = registry.get_config_schemas();
        assert_eq!(schemas.len(), 1);
        assert!(schemas.contains_key("test"));
    }

    #[test]
    fn test_registry_command_handler() {
        let registry = PluginRegistry::new();

        let handler = CommandHandler {
            command: "G1".to_string(),
            params: vec![FieldDef {
                name: "X".to_string(),
                field_type: FieldType::Float,
                required: false,
                description: Some("X coordinate".to_string()),
                default_value: None,
            }],
            description: Some("Linear move".to_string()),
            scheduling_class: "rt".to_string(),
        };

        let id = registry.register_command_handler(handler).unwrap();
        assert_eq!(id, 0);

        let handlers = registry.get_command_handlers();
        assert_eq!(handlers.len(), 1);
        assert!(handlers.contains_key(&id));

        assert!(registry.unregister_command_handler(id).is_ok());
        assert!(registry.unregister_command_handler(id).is_err());
    }

    #[test]
    fn test_registry_plugin_info() {
        let registry = PluginRegistry::new();

        let info = PluginInfo {
            id: "com.example.test".to_string(),
            name: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("A test plugin".to_string()),
        };

        assert!(registry.register_plugin(info.clone()).is_ok());
        assert!(registry.register_plugin(info).is_err());

        let plugins = registry.get_plugins();
        assert_eq!(plugins.len(), 1);
        assert!(plugins.contains_key("com.example.test"));
    }
}
