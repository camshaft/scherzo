/// Integration test for plugin configuration system
use anyhow::Result;
use std::fs;

#[test]
fn test_extract_example_plugin_schema() -> Result<()> {
    // First, ensure the example plugin is built
    let plugin_wasm = "target/wasm32-wasip2/debug/example_plugin.wasm";
    
    if !std::path::Path::new(plugin_wasm).exists() {
        eprintln!("Skipping test - example plugin not built");
        eprintln!("Run: cargo build -p example-plugin --target wasm32-wasip2");
        return Ok(());
    }

    // Read the plugin
    let wasm_bytes = fs::read(plugin_wasm)?;
    
    // The raw plugin won't have a custom section yet
    // (we need to add it post-build)
    use scherzo::wasm_util::extract_plugin_schema;
    let schema = extract_plugin_schema(&wasm_bytes)?;
    
    // For now, just verify the extraction doesn't crash
    println!("Schema extracted: {:?}", schema.is_some());
    
    Ok(())
}

#[test]
fn test_plugin_manager_extract_schemas() -> Result<()> {
    // Test the extract_schemas static method
    let plugin_paths = vec![];  // Empty for now
    
    use scherzo::plugin::PluginManager;
    let schemas = PluginManager::extract_schemas(&plugin_paths)?;
    
    assert_eq!(schemas.len(), 0);
    
    Ok(())
}

