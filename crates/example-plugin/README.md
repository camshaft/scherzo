# Example Plugin

This is a demonstration plugin showing how to use Scherzo's plugin configuration system.

## Features

- **Static Configuration Schema**: The plugin's configuration schema is embedded in the WASM file as a custom section
- **Type-Safe Config**: Configuration is validated against the JSON schema before being passed to the plugin
- **Lifecycle Management**: Plugin exports `get-info`, `init`, and `cleanup` functions

## Building

Run the build script from the repository root:

```bash
./build-example-plugin.sh
```

This will:
1. Build the plugin as a WASM component
2. Embed the configuration schema as a custom section
3. Output the plugin to `target/plugins/example-plugin.wasm`

## Configuration Schema

The plugin expects configuration matching this JSON schema:

```json
{
  "type": "object",
  "properties": {
    "enabled": {
      "type": "boolean",
      "description": "Whether the demo plugin is enabled",
      "default": true
    },
    "message": {
      "type": "string",
      "description": "A custom message for the plugin",
      "default": "Hello from demo plugin!"
    },
    "interval_seconds": {
      "type": "number",
      "description": "Interval in seconds for some operation",
      "default": 60,
      "minimum": 1
    }
  }
}
```

## Using the Plugin

Add the plugin to your `config.toml`:

```toml
# List of plugins to load
plugins = ["target/plugins/example-plugin.wasm"]

# Plugin-specific configuration
[com.example.demo]
enabled = true
message = "Custom greeting!"
interval_seconds = 30
```

The Scherzo runtime will:
1. Extract the schema from the plugin before loading
2. Validate your configuration against the schema
3. Pass the validated config to the plugin's `init()` function

## Implementation

See `src/lib.rs` for the plugin implementation. Key components:

- **WIT bindings**: Generated from the plugin WIT interface
- **get-info()**: Returns plugin metadata (ID, name, version)
- **init(config)**: Receives validated configuration as JSON
- **cleanup()**: Called before plugin unload
