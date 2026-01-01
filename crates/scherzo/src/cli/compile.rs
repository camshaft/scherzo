use anyhow::{Context, Result};
use clap::Args;
use scherzo_compile::compile_gcode;
use std::{fs, path::PathBuf};

#[derive(Args)]
pub struct CompileArgs {
    /// Path to the input G-code file.
    pub input: PathBuf,

    /// Path where output artifacts will be written.
    ///
    /// Defaults to the input file name with a `wasm` extension.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

impl CompileArgs {
    pub fn run(&self) -> Result<()> {
        let source = fs::read_to_string(&self.input)
            .with_context(|| format!("failed to read input {}", self.input.display()))?;
        let compilation = compile_gcode(&source)?;

        let output = self.output.as_ref().cloned().unwrap_or_else(|| {
            let mut default_output = self.input.clone();
            default_output.set_extension("wasm");
            default_output
        });

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }

        fs::write(&output, &compilation.component)
            .with_context(|| format!("failed to write {}", output.display()))?;

        println!("Wrote component to {}", output.display());

        Ok(())
    }
}
