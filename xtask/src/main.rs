use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::Command;

#[derive(Parser)]
#[command(author, version, about = "Project automation commands", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run cargo nextest with default configuration
    Nextest {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        release: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Nextest { profile, release } => run_nextest(profile, release)?,
    }
    Ok(())
}

fn run_nextest(profile: Option<String>, release: bool) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("nextest").arg("run");
    if let Some(profile) = profile {
        cmd.arg("--profile").arg(profile);
    }
    if release {
        cmd.arg("--release");
    }
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("cargo nextest run failed");
    }
    Ok(())
}
