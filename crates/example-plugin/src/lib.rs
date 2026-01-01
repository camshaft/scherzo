// Example plugin demonstrating configuration registration
//
// This plugin shows how to:
// 1. Define a configuration schema in a custom WASM section
// 2. Export plugin metadata via get-info
// 3. Receive parsed configuration via init

wit_bindgen::generate!({
    world: "plugin",
    path: "../scherzo/wit",
});

use exports::scherzo::plugin::lifecycle::{Guest, PluginInfo};

struct Component;

export!(Component);

impl Guest for Component {
    fn get_info() -> PluginInfo {
        PluginInfo {
            id: "com.example.demo".to_string(),
            name: "Demo Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("A simple demonstration plugin".to_string()),
        }
    }

    fn init(config: String) -> Result<(), String> {
        // In a real plugin, we would parse and use the config here
        // For now, just log that we received it
        eprintln!("Demo plugin initialized with config: {}", config);
        Ok(())
    }

    fn cleanup() {
        eprintln!("Demo plugin cleanup");
    }
}
