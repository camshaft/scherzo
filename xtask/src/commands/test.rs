use anyhow::Result;
use clap::Args;
use xshell::{Shell, cmd};

#[derive(Args)]
pub struct Test {
    #[arg(long, default_value = "dev")]
    profile: String,
}

impl Test {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        // Run Rust tests
        let cargo = cmd!(sh, "cargo test").arg("--profile").arg(&self.profile);
        cargo.run()?;

        Ok(())
    }
}
