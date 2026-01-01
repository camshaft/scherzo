/// End-to-end test for plugin configuration system
use anyhow::Result;
use std::fs;
use std::path::Path;

#[test]
fn test_plugin_config_end_to_end() -> Result<()> {
    // Check if the example plugin is built
    // Tests run from crate directory, so we need to go up two levels
    let plugin_path = "../../target/plugins/example-plugin.wasm";
    let config_path = "../../example-with-plugin.toml";
    
    if !Path::new(plugin_path).exists() {
        eprintln!("Skipping test - example plugin not built");
        eprintln!("Run: ./build-example-plugin.sh from repository root");
        return Ok(());
    }

    // 1. Extract schema from plugin
    let wasm_bytes = fs::read(plugin_path)?;
    let schema = scherzo::wasm_util::extract_plugin_schema(&wasm_bytes)?;
    
    assert!(schema.is_some(), "Plugin should have embedded schema");
    let schema = schema.unwrap();
    
    assert_eq!(schema.plugin_id, "com.example.demo");
    assert!(schema.description.is_some());
    assert!(!schema.json_schema.is_empty());
    
    println!("✓ Schema extracted from plugin");
    
    // 2. Verify schema is valid JSON Schema
    let json_schema: serde_json::Value = serde_json::from_str(&schema.json_schema)?;
    
    assert_eq!(json_schema["type"], "object");
    assert!(json_schema["properties"].is_object());
    
    println!("✓ Schema is valid JSON Schema");
    
    // 3. Load config file
    let config = scherzo::config::Config::from_file(config_path)?;
    
    println!("Plugins: {:?}", config.plugins);
    println!("Plugin config keys: {:?}", config.plugin_config.keys().collect::<Vec<_>>());
    
    assert_eq!(config.plugins.len(), 1);
    assert!(config.plugins[0].contains("example-plugin.wasm"));
    
    println!("✓ Config loaded successfully");
    
    // 4. Verify plugin-specific config is present
    assert!(config.plugin_config.contains_key("com.example.demo"));
    let plugin_config = &config.plugin_config["com.example.demo"];
    
    // Verify config values
    assert_eq!(plugin_config["enabled"], true);
    assert_eq!(plugin_config["message"], "Hello from plugin configuration!");
    assert_eq!(plugin_config["interval_seconds"], 45);
    
    println!("✓ Plugin config correctly parsed");
    
    // 5. Extract schemas from all plugins (like the runtime would)
    // Adjust paths since test runs from crate directory
    let adjusted_paths: Vec<String> = config.plugins
        .iter()
        .map(|p| format!("../../{}", p))
        .collect();
    
    let schemas = scherzo::plugin::PluginManager::extract_schemas(&adjusted_paths)?;
    
    assert_eq!(schemas.len(), 1);
    assert!(schemas.contains_key("com.example.demo"));
    
    println!("✓ All plugin schemas extracted");
    
    // 6. Serialize plugin config as JSON string (what gets passed to plugin)
    let plugin_config_json = serde_json::to_string(&config.plugin_config["com.example.demo"])?;
    
    println!("✓ Plugin config serialized: {}", plugin_config_json);
    
    println!("\n🎉 End-to-end plugin configuration test passed!");
    
    Ok(())
}
