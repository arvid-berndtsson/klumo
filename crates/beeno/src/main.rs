mod runtime;

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use runtime::BeenoRuntime;

#[derive(Debug, Parser)]
#[command(name = "beeno", version, about = "Beeno runtime (M0 bootstrap)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a JavaScript file in Beeno.
    Run {
        /// Path to a JavaScript file.
        file: PathBuf,
    },
    /// Evaluate a JavaScript expression/script.
    Eval {
        /// Inline JavaScript source.
        code: String,
    },
    /// Start an interactive JavaScript REPL.
    Repl,
}

fn run_file(path: PathBuf) -> Result<()> {
    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed reading script file {}", path.display()))?;

    let mut runtime = BeenoRuntime::new();
    if let Some(value) = runtime.eval_script(&source, &path.display().to_string())? {
        println!("{value}");
    }

    Ok(())
}

fn eval_inline(code: String) -> Result<()> {
    let mut runtime = BeenoRuntime::new();
    if let Some(value) = runtime.eval_script(&code, "<eval>")? {
        println!("{value}");
    }
    Ok(())
}

fn repl() -> Result<()> {
    let mut runtime = BeenoRuntime::new();
    let mut line = String::new();

    println!("Beeno REPL (M0). Type .exit to quit.");

    loop {
        line.clear();
        print!("beeno> ");
        io::stdout().flush().context("failed flushing stdout")?;

        let bytes_read = io::stdin()
            .read_line(&mut line)
            .context("failed reading REPL input")?;
        if bytes_read == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == ".exit" {
            break;
        }

        match runtime.eval_script(trimmed, "<repl>") {
            Ok(Some(value)) => println!("{value}"),
            Ok(None) => {}
            Err(err) => eprintln!("error: {err:#}"),
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { file } => run_file(file),
        Commands::Eval { code } => eval_inline(code),
        Commands::Repl => repl(),
    }
}
