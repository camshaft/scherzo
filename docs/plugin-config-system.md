# Plugin Configuration Registration System

This document describes the enhanced plugin configuration system implemented for Scherzo.

## Overview

The plugin configuration system allows plugins to declare their configuration schemas statically, which are then validated and merged by the host. This ensures that:

1. **Type safety**: Configuration is strongly typed and validated before plugins are initialized
2. **Conflict detection**: The host detects incompatible schema requirements between plugins
3. **Shared config space**: Plugins can read from the same configuration fields (no namespacing)
4. **Early validation**: Configuration errors are caught before plugin initialization

## Architecture

### WIT Interface Changes

The `crates/scherzo/wit/plugin.wit` file defines the contract between plugins and the host:

#### Static Schema Function

```wit
/// Get the configuration schema for this plugin
/// This is a static function called before initialization
/// Returns the schema that describes expected configuration fields
get-config-schema: func() -> schema;
```

This function is called by the host **before** initialization to collect all plugin schemas.

#### Plugin Instance Resource

```wit
/// Resource representing a plugin instance
/// Returned by init and tied to the plugin+config combination
resource plugin-instance {
    /// Get the plugin ID for this instance
    get-id: func() -> string;
}
```

The `init` function now returns a `plugin-instance` resource:

```wit
/// Initialize the plugin with validated configuration
/// The config is provided as JSON matching the registered schema
/// Returns a plugin instance resource
init: func(config: string) -> result<plugin-instance, string>;
```

#### Registry Interface Simplified

The registry interface no longer includes `register-config-schema` since schemas are retrieved statically via `get-config-schema`:

```wit
interface registry {
    use types.{command-handler};
    
    /// Register a command handler
    register-command-handler: func(handler: command-handler) -> result<u32, string>;
    
    /// Unregister a command handler by ID
    unregister-command-handler: func(handler-id: u32) -> result<_, string>;
}
```

### Host Implementation

#### Schema Storage and Merging

The `PluginRegistry` struct maintains schemas from all plugins:

- **No namespacing**: Schemas are stored by plugin ID but contribute to a shared config space
- **Conflict detection**: When a new schema is registered, it's checked against existing schemas
- **Schema merging**: All plugin schemas are merged into a single unified schema

```rust
pub struct PluginRegistry {
    /// Registered config schemas by plugin ID
    config_schemas: Arc<RwLock<HashMap<String, Schema>>>,
    /// Merged configuration schema from all plugins
    merged_schema: Arc<RwLock<Option<Schema>>>,
    // ... other fields
}
```

#### Schema Conflict Detection

When a plugin registers its schema, the host checks for conflicts:

```rust
fn check_schema_compatibility(
    new_schema: &serde_json::Value,
    existing_schema: &serde_json::Value,
    existing_plugin: &str,
    new_plugin: &str,
) -> Result<()>
```

Currently, this checks that:
- Fields with the same name have compatible types
- Future enhancements could check: enum values, number ranges, string patterns, etc.

#### Plugin Loading Flow

The new plugin loading sequence:

1. **Instantiate** the plugin WebAssembly component
2. **Call `get-info`** to retrieve plugin metadata (ID, name, version)
3. **Call `get-config-schema`** to retrieve the plugin's configuration schema
4. **Register schema** with conflict detection
5. **Validate config** against the merged schema
6. **Call `init`** with validated config, receiving a plugin instance resource
7. **Register plugin** in the plugin registry

```rust
pub fn load_plugin(&mut self, path: &str, config_json: &str) -> Result<PluginInfo> {
    // ... instantiate component ...
    
    // Get plugin info
    let wit_info = lifecycle.call_get_info(&mut store)?;
    
    // Get config schema
    let wit_schema = lifecycle.call_get_config_schema(&mut store)?;
    
    // Register schema (with conflict detection)
    self.registry.register_config_schema(info.id.clone(), schema)?;
    
    // Validate config
    self.registry.validate_config(config_json)?;
    
    // Initialize with validated config
    let _plugin_instance = lifecycle.call_init(&mut store, config_json)?;
    
    // ... register plugin ...
}
```

## Configuration Validation

The host provides basic configuration validation:

```rust
pub fn validate_config(&self, config_json: &str) -> Result<()>
```

Current validation checks:
- Config is valid JSON
- Config is a JSON object
- All required fields are present

Future enhancements could include:
- Type checking for each field
- Range validation for numbers
- Pattern validation for strings
- Array item validation
- Full JSON Schema validation using a library like `jsonschema`

## Examples

### Example 1: Compatible Schemas

Two plugins that share a field with compatible types:

**Plugin A Schema:**
```json
{
  "type": "object",
  "properties": {
    "temperature": {"type": "number"},
    "speed": {"type": "number"}
  },
  "required": ["temperature"]
}
```

**Plugin B Schema:**
```json
{
  "type": "object",
  "properties": {
    "temperature": {"type": "number"},
    "pressure": {"type": "number"}
  },
  "required": ["pressure"]
}
```

**Merged Schema:**
```json
{
  "type": "object",
  "properties": {
    "temperature": {"type": "number"},
    "speed": {"type": "number"},
    "pressure": {"type": "number"}
  },
  "required": ["temperature", "pressure"]
}
```

Both plugins can read the `temperature` field, and the config must provide both required fields.

### Example 2: Conflicting Schemas

Two plugins with incompatible field types will fail to load:

**Plugin A Schema:**
```json
{
  "type": "object",
  "properties": {
    "port": {"type": "number"}
  }
}
```

**Plugin B Schema:**
```json
{
  "type": "object",
  "properties": {
    "port": {"type": "string"}
  }
}
```

**Result:** Error during plugin B loading:
```
Schema conflict detected: Plugin 'plugin-b' and 'plugin-a' have 
incompatible types for field 'port': "string" vs "number"
```

## Testing

The implementation includes comprehensive tests:

1. **Schema merging**: Verifies that schemas from multiple plugins are correctly merged
2. **Conflict detection (compatible)**: Tests that compatible field definitions are allowed
3. **Conflict detection (incompatible)**: Tests that incompatible field types are rejected
4. **Duplicate registration**: Ensures a plugin can't register its schema twice
5. **Config validation (valid)**: Tests successful validation of compliant configs
6. **Config validation (missing required)**: Tests detection of missing required fields
7. **Config validation (invalid JSON)**: Tests rejection of malformed JSON
8. **Config validation (not object)**: Tests rejection of non-object configs

Run the tests:
```bash
cargo test --package scherzo plugin::tests
```

## Future Enhancements

1. **Full JSON Schema validation**: Integrate a JSON Schema validation library
2. **Schema versioning**: Support schema evolution and migration
3. **Config defaults**: Apply default values from schemas
4. **Enhanced conflict detection**: Check enum values, ranges, patterns, etc.
5. **Resource tracking**: Store plugin instance resources for later interaction
6. **Hot reload**: Support updating plugin configs without restart
7. **Config documentation**: Generate documentation from schemas

## Plugin Developer Guide

To create a plugin with configuration:

1. **Implement `get-config-schema`** to return your schema:
```rust
fn get_config_schema() -> Schema {
    Schema {
        json_schema: r#"{
            "type": "object",
            "properties": {
                "port": {"type": "number"},
                "host": {"type": "string"}
            },
            "required": ["port"]
        }"#.to_string(),
        description: Some("HTTP server config".to_string()),
    }
}
```

2. **Implement `init`** to receive validated config:
```rust
fn init(config: String) -> Result<PluginInstance, String> {
    let config: MyConfig = serde_json::from_str(&config)
        .map_err(|e| e.to_string())?;
    
    // Use the strongly-typed config
    println!("Binding to {}:{}", config.host, config.port);
    
    Ok(PluginInstance::new(config))
}
```

3. **No manual validation needed**: The host validates config before calling `init`

4. **Share config fields**: Multiple plugins can read the same fields if types match

## Migration from Old System

The old system used namespaced config registration:

```rust
// Old approach (no longer supported)
register-config-schema: func(namespace: string, schema: schema) -> result<_, string>;
```

The new system uses static schema retrieval:

```rust
// New approach
get-config-schema: func() -> schema;
```

Benefits of the new approach:
- Schemas are declared upfront, not during runtime
- No namespace management needed
- Easier to reason about plugin requirements
- Conflicts detected before any initialization
