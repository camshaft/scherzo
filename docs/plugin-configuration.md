# Plugin Configuration System

This document explains Scherzo's plugin configuration system that allows plugins to statically define their configuration schemas.

## Overview

The system enables:

1. **Static Schema Discovery**: Configuration schemas are extracted from WASM files without loading/instantiating plugins
2. **Type-Safe Configuration**: Schemas are defined using JSON Schema and validated before being passed to plugins
3. **Flexible Config Format**: Configuration can be provided in TOML or JSON format
4. **Plugin-Specific Sections**: Each plugin's config is namespaced by its plugin ID

## Architecture

### Custom WASM Sections

Plugins embed their configuration schema as a custom WASM section named `"plugin-config-schema"`. The section contains a JSON object with:

```json
{
  "plugin_id": "com.example.plugin",
  "json_schema": "{...JSON Schema...}",
  "description": "Optional description"
}
```

### Schema Extraction

The `wasm_util::extract_plugin_schema()` function reads custom sections from WASM files using `wasmparser` without instantiating the component:

```rust
use scherzo::wasm_util::extract_plugin_schema;

let wasm_bytes = std::fs::read("plugin.wasm")?;
let schema = extract_plugin_schema(&wasm_bytes)?;
```

### Configuration Loading

When starting Scherzo:

1. **Extract schemas** from all plugin files listed in the config
2. **Merge schemas** into the overall configuration structure
3. **Parse config** file (TOML or JSON)
4. **Validate** plugin-specific configs against their schemas
5. **Load plugins** and pass their validated configs to `init()`

### Plugin Lifecycle

Plugins implement the `scherzo:plugin/lifecycle` interface:

```wit
interface lifecycle {
    record plugin-info {
        id: string,
        name: string,
        version: string,
        description: option<string>,
    }

    get-info: func() -> plugin-info;
    init: func(config: string) -> result<_, string>;
    cleanup: func();
}
```

## Creating a Plugin

See `crates/example-plugin` for a complete example.

### 1. Define the Plugin

```rust
wit_bindgen::generate!({
    world: "plugin",
    path: "../scherzo/wit",
});

use exports::scherzo::plugin::lifecycle::{Guest, PluginInfo};

struct MyPlugin;

export!(MyPlugin);

impl Guest for MyPlugin {
    fn get_info() -> PluginInfo {
        PluginInfo {
            id: "com.example.myplugin".to_string(),
            name: "My Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("A custom plugin".to_string()),
        }
    }

    fn init(config: String) -> Result<(), String> {
        // Parse and use config
        Ok(())
    }

    fn cleanup() {
        // Cleanup resources
    }
}
```

### 2. Create Config Schema

Define a JSON Schema for your configuration. For the example plugin:

```json
{
  "type": "object",
  "properties": {
    "enabled": {
      "type": "boolean",
      "default": true
    },
    "message": {
      "type": "string",
      "default": "Hello!"
    }
  }
}
```

### 3. Build with Schema

Use the provided build script or create your own:

```bash
./build-example-plugin.sh
```

This embeds the schema as a custom WASM section.

### 4. Configure and Load

Add to `config.toml`:

```toml
plugins = ["path/to/plugin.wasm"]

[com.example.myplugin]
enabled = true
message = "Custom message"
```

## Implementation Status

### ✅ Completed

- Custom section extraction from WASM components
- `PluginConfigSchema` type and extraction utilities
- `Config` struct supports dynamic plugin configurations
- `PluginManager::extract_schemas()` for static schema discovery
- Plugin-specific config lookup and passing to plugins
- Example plugin with config schema
- Build script for adding custom sections

### 🚧 In Progress

- Host-side registry interface implementation (host functions not yet linked)
- Full plugin instantiation with lifecycle calls (currently placeholder)
- Config schema merging and validation

### 📋 TODO

- Implement proper host function linking for registry interface
- Add JSON Schema validation using a validation library
- Merge extracted schemas into overall config schema
- Add comprehensive tests for config validation
- Error handling and diagnostics improvements
- Documentation for plugin authors

## Testing

Run tests:

```bash
# Build the example plugin first
./build-example-plugin.sh

# Run plugin config tests
cargo test -p scherzo --test plugin_config_test

# Test schema extraction
cargo test -p scherzo wasm_util
```

## Future Enhancements

- **Schema Validation**: Use `jsonschema` crate to validate configs
- **Default Values**: Auto-populate defaults from schemas
- **Config Hot-Reload**: Reload plugin configs without restart
- **Schema Documentation**: Auto-generate config documentation from schemas
- **Type Generation**: Generate typed config structs from schemas
