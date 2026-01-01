/// Utilities for working with WebAssembly custom sections
///
/// This module provides functionality to read and write custom sections
/// in WASM components, particularly for plugin configuration schemas.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use wasmparser::{Parser, Payload};

/// Name of the custom section that contains plugin config schema
pub const CONFIG_SCHEMA_SECTION: &str = "plugin-config-schema";

/// Configuration schema extracted from a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfigSchema {
    /// Unique plugin identifier
    pub plugin_id: String,
    /// JSON Schema for the plugin's configuration
    pub json_schema: String,
    /// Human-readable description
    pub description: Option<String>,
}

/// Extract the plugin config schema from a WASM component
///
/// This reads the custom section without actually loading/instantiating the plugin
pub fn extract_plugin_schema(wasm_bytes: &[u8]) -> Result<Option<PluginConfigSchema>> {
    let parser = Parser::new(0);

    for payload in parser.parse_all(wasm_bytes) {
        let payload = payload.context("Failed to parse WASM payload")?;

        if let Payload::CustomSection(custom) = payload {
            if custom.name() == CONFIG_SCHEMA_SECTION {
                // Parse the JSON schema from the custom section data
                let schema: PluginConfigSchema = serde_json::from_slice(custom.data())
                    .context("Failed to parse plugin config schema from custom section")?;
                return Ok(Some(schema));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_missing_schema() {
        // Create a minimal WASM module without custom sections
        let wasm = wat::parse_str(
            r#"
            (module
                (func (export "test"))
            )
            "#,
        )
        .unwrap();

        let schema = extract_plugin_schema(&wasm).unwrap();
        assert!(schema.is_none());
    }

    #[test]
    fn test_extract_plugin_schema() {
        // Create a WASM module with a custom section containing schema
        let schema_data = PluginConfigSchema {
            plugin_id: "com.example.test".to_string(),
            json_schema: r#"{"type": "object", "properties": {"enabled": {"type": "boolean"}}}"#
                .to_string(),
            description: Some("Test plugin".to_string()),
        };

        let schema_json = serde_json::to_vec(&schema_data).unwrap();

        // Build a WASM module with custom section
        let mut module = wasm_encoder::Module::new();

        // Add a simple type section (required for valid module)
        let mut types = wasm_encoder::TypeSection::new();
        types.ty().function([], []);
        module.section(&types);

        // Add function section
        let mut functions = wasm_encoder::FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        // Add export section
        let mut exports = wasm_encoder::ExportSection::new();
        exports.export("test", wasm_encoder::ExportKind::Func, 0);
        module.section(&exports);

        // Add code section
        let mut code = wasm_encoder::CodeSection::new();
        let mut func = wasm_encoder::Function::new([]);
        func.instruction(&wasm_encoder::Instruction::End);
        code.function(&func);
        module.section(&code);

        // Add custom section with schema
        module.section(&wasm_encoder::CustomSection {
            name: CONFIG_SCHEMA_SECTION.into(),
            data: schema_json.into(),
        });

        let wasm_bytes = module.finish();

        // Extract and verify
        let extracted = extract_plugin_schema(&wasm_bytes).unwrap();
        assert!(extracted.is_some());

        let extracted = extracted.unwrap();
        assert_eq!(extracted.plugin_id, "com.example.test");
        assert!(extracted.json_schema.contains("enabled"));
    }
}
