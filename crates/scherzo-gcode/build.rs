use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let roots = [manifest_dir.join("test-data")];

    let mut files = Vec::new();
    for root in roots {
        println!("cargo:rerun-if-changed={}", root.display());
        collect_files(&root, &mut files)?;
    }

    files.sort();

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let dest = out_dir.join("generated_tests.rs");

    let mut buffer = String::new();
    writeln!(buffer, "use insta::assert_snapshot;")?;
    writeln!(
        buffer,
        "use crate::testing::{{snapshot_from_str, snapshot_tokens_from_str}};"
    )?;
    writeln!(buffer)?;

    for path in files {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sample");
        let ident = sanitize_ident(stem);
        let rel = path.strip_prefix(&manifest_dir)?;
        let rel_str = to_unix_path(rel);
        let snapshot_dir = path
            .parent()
            .and_then(|p| p.strip_prefix(&manifest_dir).ok())
            .map(to_unix_path)
            .unwrap_or_else(|| "snapshots".to_string());

        // Token snapshot
        writeln!(buffer, "#[test]")?;
        writeln!(buffer, "fn snapshot_{}_tokens() {{", ident)?;
        writeln!(
            buffer,
            "    let input = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\"));",
            rel_str
        )?;
        writeln!(
            buffer,
            "    let snapshot = snapshot_tokens_from_str(input);"
        )?;
        writeln!(
            buffer,
            "    insta::with_settings!({{snapshot_path => concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\"), prepend_module_to_snapshot => false}}, {{",
            snapshot_dir
        )?;
        writeln!(
            buffer,
            "        assert_snapshot!(\"{}.tokens\", snapshot);",
            stem
        )?;
        writeln!(buffer, "    }});")?;
        writeln!(buffer, "}}")?;
        writeln!(buffer)?;

        // Parsed snapshot
        writeln!(buffer, "#[test]")?;
        writeln!(buffer, "fn snapshot_{}_parsed() {{", ident)?;
        writeln!(
            buffer,
            "    let input = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\"));",
            rel_str
        )?;
        writeln!(buffer, "    let snapshot = snapshot_from_str(input);")?;
        writeln!(
            buffer,
            "    insta::with_settings!({{snapshot_path => concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\"), prepend_module_to_snapshot => false}}, {{",
            snapshot_dir
        )?;
        writeln!(
            buffer,
            "        assert_snapshot!(\"{}.parsed\", snapshot);",
            stem
        )?;
        writeln!(buffer, "    }});")?;
        writeln!(buffer, "}}")?;
        writeln!(buffer)?;
    }

    fs::write(dest, buffer)?;
    Ok(())
}

fn sanitize_ident(stem: &str) -> String {
    let mut ident = String::new();
    for (idx, ch) in stem.chars().enumerate() {
        let c = if ch.is_ascii_alphanumeric() { ch } else { '_' };
        if idx == 0 && c.is_ascii_digit() {
            ident.push('_');
        }
        ident.push(c);
    }
    if ident.is_empty() {
        ident.push_str("sample");
    }
    ident
}

fn to_unix_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("gcode") | Some("test")
    )
}

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect_files(&path, out)?;
        } else if is_supported(&path) {
            println!("cargo:rerun-if-changed={}", path.display());
            out.push(path);
        }
    }
    Ok(())
}
