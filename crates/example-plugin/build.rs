use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Define the plugin's configuration schema
    let schema = serde_json::json!({
        "plugin_id": "com.example.demo",
        "json_schema": serde_json::json!({
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
        }).to_string(),
        "description": "Configuration for the demo plugin"
    });

    // Write the schema to a file that can be read at link time
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let schema_path = out_dir.join("plugin_schema.json");
    fs::write(&schema_path, serde_json::to_string_pretty(&schema).unwrap())
        .expect("Failed to write schema file");

    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=PLUGIN_SCHEMA_PATH={}",
        schema_path.display()
    );
}
