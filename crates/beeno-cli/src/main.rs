use anyhow::{Context, Result, anyhow};
use beeno_compiler::{CompilerRouter, FileCompileCache, SourceKind};
use beeno_core::{RunOptions, eval_inline, run_file};
use beeno_engine::{BoaEngine, JsEngine};
use beeno_llm::{
    LlmClient, LlmTranslateRequest, ProviderSelection, ProviderRouter, ReachabilityProbe,
};
use beeno_llm_ollama::OllamaClient;
use beeno_llm_openai::OpenAiCompatibleClient;
use clap::{Parser, Subcommand, ValueEnum};
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Auto,
    Ollama,
    Openai,
}

impl ProviderArg {
    fn as_selection(self) -> ProviderSelection {
        match self {
            ProviderArg::Auto => ProviderSelection::Auto,
            ProviderArg::Ollama => ProviderSelection::Ollama,
            ProviderArg::Openai => ProviderSelection::OpenAiCompatible,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "beeno", version, about = "Beeno runtime (M1)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a file in Beeno.
    Run {
        file: PathBuf,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        print_js: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_llm: bool,
        #[arg(long, value_enum, default_value_t = ProviderArg::Auto)]
        provider: ProviderArg,
        #[arg(long)]
        ollama_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        verbose: bool,
    },
    /// Evaluate inline JavaScript.
    Eval { code: String },
    /// Start a JavaScript REPL.
    Repl,
}

struct OllamaProbe {
    client: OllamaClient,
}

impl ReachabilityProbe for OllamaProbe {
    fn ollama_reachable(&self) -> bool {
        self.client.is_reachable()
    }
}

struct MaybeOpenAiClient {
    inner: Option<OpenAiCompatibleClient>,
}

impl LlmClient for MaybeOpenAiClient {
    fn translate_to_js(&self, req: &LlmTranslateRequest, model: &str) -> Result<String> {
        let client = self.inner.as_ref().ok_or_else(|| {
            anyhow!("OPENAI_API_KEY is required for OpenAI-compatible translation")
        })?;
        client.translate_to_js(req, model)
    }
}

fn parse_kind_hint(lang: Option<&str>) -> Option<SourceKind> {
    lang.map(SourceKind::from_hint)
}

fn run_command(
    file: PathBuf,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    force_llm: bool,
    provider: ProviderArg,
    ollama_url: Option<String>,
    model: Option<String>,
    verbose: bool,
) -> Result<()> {
    let ollama_url = ollama_url
        .or_else(|| std::env::var("BEENO_OLLAMA_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let ollama_model = std::env::var("BEENO_OLLAMA_MODEL")
        .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());
    let openai_model = std::env::var("BEENO_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());

    let ollama_client = OllamaClient::new(ollama_url)?;
    let openai_client = MaybeOpenAiClient {
        inner: OpenAiCompatibleClient::from_env().ok(),
    };

    let router = ProviderRouter {
        ollama: ollama_client.clone(),
        openai: openai_client,
        reachability: OllamaProbe {
            client: ollama_client,
        },
        ollama_model,
        openai_model,
    };

    let compiler = CompilerRouter {
        translator: router,
        cache: FileCompileCache::default(),
    };

    let options = RunOptions {
        kind_hint: parse_kind_hint(lang.as_deref()),
        language_hint: lang,
        force_llm,
        no_cache,
        print_js,
        provider_selection: provider.as_selection(),
        model_override: model,
        verbose,
    };

    let mut engine = BoaEngine::new();
    if verbose {
        eprintln!(
            "[beeno] run provider={:?} force_llm={} lang={:?}",
            provider, force_llm, options.language_hint
        );
    }
    let outcome = run_file(&mut engine, &compiler, &file, &options)
        .with_context(|| format!("failed running {}", file.display()))?;

    if let Some(value) = outcome.eval.value {
        println!("{value}");
    }

    Ok(())
}

fn eval_command(code: String) -> Result<()> {
    let mut engine = BoaEngine::new();
    let out = eval_inline(&mut engine, &code)?;
    if let Some(value) = out.value {
        println!("{value}");
    }
    Ok(())
}

fn repl_command() -> Result<()> {
    let mut engine = BoaEngine::new();
    let mut line = String::new();

    println!("Beeno REPL (M1). Type .exit to quit.");
    loop {
        line.clear();
        print!("beeno> ");
        io::stdout().flush().context("failed flushing stdout")?;

        let bytes = io::stdin()
            .read_line(&mut line)
            .context("failed reading REPL input")?;
        if bytes == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == ".exit" {
            break;
        }

        match engine.eval_script(trimmed, "<repl>") {
            Ok(output) => {
                if let Some(value) = output.value {
                    println!("{value}");
                }
            }
            Err(err) => eprintln!("error: {err:#}"),
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            file,
            lang,
            print_js,
            no_cache,
            force_llm,
            provider,
            ollama_url,
            model,
            verbose,
        } => run_command(
            file,
            lang,
            print_js,
            no_cache,
            force_llm,
            provider,
            ollama_url,
            model,
            verbose,
        ),
        Commands::Eval { code } => eval_command(code),
        Commands::Repl => repl_command(),
    }
}
