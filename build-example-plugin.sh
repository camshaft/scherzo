#!/usr/bin/env bash
# Build the example plugin with config schema embedded

set -e

echo "Building example plugin..."

# Build the WASM module
cargo build -p example-plugin --target wasm32-wasip2

# Get the output path
PLUGIN_WASM="target/wasm32-wasip2/debug/example_plugin.wasm"
OUTPUT_DIR="target/plugins"
mkdir -p "$OUTPUT_DIR"
OUTPUT_WASM="$OUTPUT_DIR/example-plugin.wasm"

# Create the schema JSON
SCHEMA_JSON=$(cat << 'EOF'
{
  "plugin_id": "com.example.demo",
  "json_schema": "{\"type\":\"object\",\"properties\":{\"enabled\":{\"type\":\"boolean\",\"description\":\"Whether the demo plugin is enabled\",\"default\":true},\"message\":{\"type\":\"string\",\"description\":\"A custom message for the plugin\",\"default\":\"Hello from demo plugin!\"},\"interval_seconds\":{\"type\":\"number\",\"description\":\"Interval in seconds for some operation\",\"default\":60,\"minimum\":1}}}",
  "description": "Configuration for the demo plugin"
}
EOF
)

# Save schema to temp file
SCHEMA_FILE=$(mktemp)
echo "$SCHEMA_JSON" > "$SCHEMA_FILE"

# Add custom section using Python (since wasm-tools doesn't support arbitrary custom sections easily)
python3 - "$PLUGIN_WASM" "$SCHEMA_FILE" "$OUTPUT_WASM" << 'PYTHON_SCRIPT'
import sys

# Read the WASM file
with open(sys.argv[1], 'rb') as f:
    wasm_data = bytearray(f.read())

# Read the custom section data
with open(sys.argv[2], 'rb') as f:
    custom_data = f.read()

section_name = b"plugin-config-schema"

# Create the custom section
section_id = 0
name_len_bytes = bytes([len(section_name)])
section_content = name_len_bytes + section_name + custom_data
section_size = len(section_content)

# Encode size as LEB128
def encode_leb128(value):
    result = []
    while True:
        byte = value & 0x7f
        value >>= 7
        if value != 0:
            byte |= 0x80
        result.append(byte)
        if value == 0:
            break
    return bytes(result)

size_bytes = encode_leb128(section_size)

# Insert after WASM header (8 bytes)
insert_pos = 8

# Build the new WASM file
new_wasm = wasm_data[:insert_pos]
new_wasm.append(section_id)
new_wasm.extend(size_bytes)
new_wasm.extend(section_content)
new_wasm.extend(wasm_data[insert_pos:])

# Write output
with open(sys.argv[3], 'wb') as f:
    f.write(new_wasm)

print(f"✓ Added custom section ({len(custom_data)} bytes)")
PYTHON_SCRIPT

# Clean up
rm "$SCHEMA_FILE"

echo "✓ Built plugin with config schema: $OUTPUT_WASM"
echo "  Size: $(wc -c < "$OUTPUT_WASM") bytes"
echo ""
echo "To test schema extraction:"
echo "  cargo test -p scherzo --test plugin_config_test"
echo ""
echo "To use in config, add to example.toml:"
echo "  plugins = [\"$OUTPUT_WASM\"]"
