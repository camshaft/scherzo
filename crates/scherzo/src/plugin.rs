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

/// Check if two schemas are compatible
/// Returns an error if they define conflicting requirements for the same fields
fn check_schema_compatibility(
    new_schema: &serde_json::Value,
    existing_schema: &serde_json::Value,
    existing_plugin: &str,
    new_plugin: &str,
) -> Result<()> {
    // Get properties from both schemas
    let new_props = new_schema.get("properties").and_then(|p| p.as_object());
    let existing_props = existing_schema.get("properties").and_then(|p| p.as_object());
    
    if let (Some(new_props), Some(existing_props)) = (new_props, existing_props) {
        // Check for overlapping fields
        for (field_name, new_field_def) in new_props {
            if let Some(existing_field_def) = existing_props.get(field_name) {
                // Field exists in both schemas - check if they're compatible
                
                // Check if types match
                let new_type = new_field_def.get("type");
                let existing_type = existing_field_def.get("type");
                
                if new_type != existing_type {
                    bail!(
                        "Plugin '{}' and '{}' have incompatible types for field '{}': {:?} vs {:?}",
                        new_plugin,
                        existing_plugin,
                        field_name,
                        new_type,
                        existing_type
                    );
                }
                
                // For more complex checks, we could also validate:
                // - enum values
                // - number ranges (minimum, maximum)
                // - string patterns
                // - array item types
                // For now, we keep it simple and just check the type matches
            }
        }
    }
    
    Ok(())
}

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
    /// Registered config schemas by plugin ID (no namespacing)
    /// All plugins contribute to a shared config space
    config_schemas: Arc<RwLock<HashMap<String, Schema>>>,
    /// Registered command handlers by handler ID
    command_handlers: Arc<RwLock<HashMap<u32, CommandHandler>>>,
    /// Next handler ID to assign
    #[allow(dead_code)] // Used by register_command_handler
    next_handler_id: Arc<RwLock<u32>>,
    /// Loaded plugins by plugin ID
    plugins: Arc<RwLock<HashMap<String, PluginInfo>>>,
    /// Merged configuration schema from all plugins
    merged_schema: Arc<RwLock<Option<Schema>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a configuration schema from a plugin
    /// Schemas are not namespaced - all plugins share the config space
    /// This method checks for conflicts and merges schemas
    pub fn register_config_schema(&self, plugin_id: String, schema: Schema) -> Result<()> {
        let mut schemas = self.config_schemas.write().unwrap();
        
        // Check if this plugin already registered a schema
        if schemas.contains_key(&plugin_id) {
            bail!(
                "Plugin '{}' already registered a config schema",
                plugin_id
            );
        }
        
        // Parse the new schema to detect conflicts with existing schemas
        let new_schema_value: serde_json::Value = serde_json::from_str(&schema.json_schema)
            .context("Failed to parse plugin config schema as JSON")?;
        
        // Check for conflicts with existing schemas
        for (existing_plugin_id, existing_schema) in schemas.iter() {
            let existing_value: serde_json::Value = serde_json::from_str(&existing_schema.json_schema)
                .context("Failed to parse existing schema as JSON")?;
            
            // Detect schema conflicts
            if let Err(e) = check_schema_compatibility(&new_schema_value, &existing_value, existing_plugin_id, &plugin_id) {
                bail!("Schema conflict detected: {}", e);
            }
        }
        
        // Add the schema
        schemas.insert(plugin_id, schema);
        
        // Invalidate merged schema to trigger re-merge
        *self.merged_schema.write().unwrap() = None;
        
        Ok(())
    }
    
    /// Get or build the merged configuration schema from all plugins
    #[allow(dead_code)] // Will be used for config validation
    pub fn get_merged_schema(&self) -> Result<Schema> {
        // Check if we have a cached merged schema
        {
            let merged = self.merged_schema.read().unwrap();
            if let Some(schema) = &*merged {
                return Ok(schema.clone());
            }
        }
        
        // Build merged schema
        let schemas = self.config_schemas.read().unwrap();
        
        if schemas.is_empty() {
            // No schemas registered, return empty object schema
            return Ok(Schema {
                json_schema: r#"{"type": "object", "properties": {}}"#.to_string(),
                description: Some("Empty configuration (no plugins registered)".to_string()),
            });
        }
        
        // Merge all schemas into one
        let mut merged_properties = serde_json::Map::new();
        let mut merged_required = Vec::new();
        
        for schema in schemas.values() {
            let schema_value: serde_json::Value = serde_json::from_str(&schema.json_schema)
                .context("Failed to parse schema as JSON")?;
            
            if let Some(obj) = schema_value.as_object() {
                // Merge properties
                if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                    for (key, value) in props {
                        merged_properties.insert(key.clone(), value.clone());
                    }
                }
                
                // Merge required fields
                if let Some(req) = obj.get("required").and_then(|r| r.as_array()) {
                    for item in req {
                        if let Some(field) = item.as_str() {
                            if !merged_required.contains(&field.to_string()) {
                                merged_required.push(field.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        // Build the merged schema object
        let mut merged_obj = serde_json::Map::new();
        merged_obj.insert("type".to_string(), serde_json::json!("object"));
        merged_obj.insert("properties".to_string(), serde_json::Value::Object(merged_properties));
        if !merged_required.is_empty() {
            merged_obj.insert("required".to_string(), serde_json::json!(merged_required));
        }
        
        let merged_schema = Schema {
            json_schema: serde_json::to_string(&merged_obj)?,
            description: Some("Merged configuration schema from all plugins".to_string()),
        };
        
        // Cache the merged schema
        *self.merged_schema.write().unwrap() = Some(merged_schema.clone());
        
        Ok(merged_schema)
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

// TODO: Implement host-side registry interface when needed
// For now, plugins don't need to call registry functions during loading
// They only export lifecycle functions which the host calls

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
    /// This implements the new plugin loading flow:
    /// 1. Instantiate the plugin
    /// 2. Call get-info to get plugin metadata
    /// 3. Call get-config-schema to get the plugin's config schema
    /// 4. Register the schema (with conflict detection)
    /// 5. Validate and merge the config
    /// 6. Call init with validated config to get a plugin instance resource
    pub fn load_plugin(&mut self, path: &str, config_json: &str) -> Result<PluginInfo> {
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
        let instance = Plugin::instantiate(&mut store, &component, &linker)
            .with_context(|| format!("Failed to instantiate plugin: {}", path))?;

        // Call get-info to get plugin metadata
        let lifecycle = instance.scherzo_plugin_lifecycle();
        let wit_info = lifecycle.call_get_info(&mut store)
            .context("Failed to call get-info on plugin")?;
        
        let info = PluginInfo {
            id: wit_info.id.clone(),
            name: wit_info.name,
            version: wit_info.version,
            description: wit_info.description,
        };
        
        tracing::info!("Plugin info: {} v{}", info.name, info.version);

        // Call get-config-schema to get the plugin's config schema
        let wit_schema = lifecycle.call_get_config_schema(&mut store)
            .context("Failed to call get-config-schema on plugin")?;
        
        let schema = Schema::from(wit_schema);
        tracing::debug!("Plugin {} config schema: {}", info.id, schema.json_schema);

        // Register the schema (this will check for conflicts)
        self.registry.register_config_schema(info.id.clone(), schema)
            .with_context(|| format!("Failed to register config schema for plugin {}", info.id))?;

        // Validate config against the merged schema
        // For now, we just pass through the config as-is
        // In a full implementation, we'd validate against the merged JSON schema
        let validated_config = config_json.to_string();

        // Call init with validated config to get plugin instance resource
        let _plugin_instance = lifecycle.call_init(&mut store, &validated_config)
            .with_context(|| format!("Failed to initialize plugin {}", info.id))?
            .map_err(|e| anyhow::anyhow!("Plugin init failed: {}", e))?;
        
        tracing::info!("Plugin {} initialized successfully", info.id);
        
        // Note: The plugin instance resource is owned by the WASM component
        // We don't need to track it on the host side for now
        // In a full implementation, we might want to store the Store and instance
        // to be able to call methods on the plugin later

        // Register the plugin
        self.registry.register_plugin(info.clone())?;

        tracing::info!("Successfully loaded plugin: {} v{}", info.name, info.version);
        Ok(info)
    }

    /// Create a linker for plugins with host functions
    fn create_plugin_linker(&self) -> Result<Linker<PluginState>> {
        let mut linker = Linker::new(&self.engine);

        // Add WASI support
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .context("Failed to add WASI to plugin linker")?;

        // TODO: Add registry host functions when plugins need to call them
        // For now, plugins only export lifecycle functions, they don't import registry

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

    #[test]
    fn test_schema_merging() {
        let registry = PluginRegistry::new();

        // Register first plugin schema with "temp" field
        let schema1 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "temp": {"type": "number"},
                    "speed": {"type": "number"}
                },
                "required": ["temp"]
            }"#.to_string(),
            description: Some("Plugin 1 schema".to_string()),
        };
        registry.register_config_schema("plugin1".to_string(), schema1).unwrap();

        // Register second plugin schema with "pressure" field
        let schema2 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "pressure": {"type": "number"},
                    "flow": {"type": "number"}
                },
                "required": ["pressure"]
            }"#.to_string(),
            description: Some("Plugin 2 schema".to_string()),
        };
        registry.register_config_schema("plugin2".to_string(), schema2).unwrap();

        // Get merged schema
        let merged = registry.get_merged_schema().unwrap();
        let merged_value: serde_json::Value = serde_json::from_str(&merged.json_schema).unwrap();

        // Check that all properties are merged
        let props = merged_value["properties"].as_object().unwrap();
        assert!(props.contains_key("temp"));
        assert!(props.contains_key("speed"));
        assert!(props.contains_key("pressure"));
        assert!(props.contains_key("flow"));

        // Check that required fields are merged
        let required = merged_value["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&serde_json::json!("temp")));
        assert!(required.contains(&serde_json::json!("pressure")));
    }

    #[test]
    fn test_schema_conflict_detection_same_field_compatible() {
        let registry = PluginRegistry::new();

        // Register first plugin with "temp" as number
        let schema1 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "temp": {"type": "number"}
                }
            }"#.to_string(),
            description: Some("Plugin 1".to_string()),
        };
        registry.register_config_schema("plugin1".to_string(), schema1).unwrap();

        // Register second plugin with same "temp" field as number - should work
        let schema2 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "temp": {"type": "number"}
                }
            }"#.to_string(),
            description: Some("Plugin 2".to_string()),
        };
        let result = registry.register_config_schema("plugin2".to_string(), schema2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_schema_conflict_detection_incompatible_types() {
        let registry = PluginRegistry::new();

        // Register first plugin with "temp" as number
        let schema1 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "temp": {"type": "number"}
                }
            }"#.to_string(),
            description: Some("Plugin 1".to_string()),
        };
        registry.register_config_schema("plugin1".to_string(), schema1).unwrap();

        // Try to register second plugin with "temp" as string - should fail
        let schema2 = Schema {
            json_schema: r#"{
                "type": "object",
                "properties": {
                    "temp": {"type": "string"}
                }
            }"#.to_string(),
            description: Some("Plugin 2".to_string()),
        };
        let result = registry.register_config_schema("plugin2".to_string(), schema2);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("incompatible types"));
    }

    #[test]
    fn test_schema_duplicate_plugin_registration() {
        let registry = PluginRegistry::new();

        let schema = Schema {
            json_schema: r#"{"type": "object", "properties": {}}"#.to_string(),
            description: Some("Test".to_string()),
        };

        // First registration should succeed
        assert!(registry.register_config_schema("plugin1".to_string(), schema.clone()).is_ok());

        // Second registration with same plugin ID should fail
        let result = registry.register_config_schema("plugin1".to_string(), schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already registered"));
    }
}
