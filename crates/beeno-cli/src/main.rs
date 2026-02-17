use anyhow::{Context, Result, anyhow};
use beeno_compiler::{CompilerRouter, FileCompileCache, SourceKind};
use beeno_config::{
    CliRunOverrides, EnvConfig, ProgressSetting, ProviderSetting, load_file_config,
    resolve_run_defaults,
};
use beeno_core::{ProgressMode, RunOptions, eval_inline, run_file};
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
    fn as_setting(self) -> ProviderSetting {
        match self {
            ProviderArg::Auto => ProviderSetting::Auto,
            ProviderArg::Ollama => ProviderSetting::Ollama,
            ProviderArg::Openai => ProviderSetting::Openai,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "beeno", version, about = "Beeno runtime (M2 UX)")]
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
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        print_js: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_llm: bool,
        #[arg(long)]
        no_progress: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long)]
        ollama_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
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

fn provider_to_selection(provider: ProviderSetting) -> ProviderSelection {
    match provider {
        ProviderSetting::Auto => ProviderSelection::Auto,
        ProviderSetting::Ollama => ProviderSelection::Ollama,
        ProviderSetting::Openai => ProviderSelection::OpenAiCompatible,
    }
}

fn resolved_progress_mode(progress: ProgressSetting, verbose: bool) -> ProgressMode {
    match progress {
        ProgressSetting::Silent => ProgressMode::Silent,
        ProgressSetting::Verbose => ProgressMode::Verbose,
        ProgressSetting::Auto => {
            if verbose {
                ProgressMode::Verbose
            } else {
                ProgressMode::Minimal
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_command(
    file: PathBuf,
    config: Option<PathBuf>,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    force_llm: bool,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cwd = std::env::current_dir().context("failed getting current directory")?;
    let file_cfg = load_file_config(config.as_deref(), &cwd)?;
    let env_cfg = EnvConfig::from_current_env();

    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: force_llm.then_some(true),
        print_js: print_js.then_some(true),
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };

    let resolved = resolve_run_defaults(&cli_overrides, &env_cfg, file_cfg.as_ref());

    let ollama_client = OllamaClient::new(resolved.ollama_url.clone())?;
    let openai_client = MaybeOpenAiClient {
        inner: std::env::var("OPENAI_API_KEY")
            .ok()
            .map(|api_key| OpenAiCompatibleClient::from_parts(resolved.openai_base_url.clone(), api_key)),
    };

    let router = ProviderRouter {
        ollama: ollama_client.clone(),
        openai: openai_client,
        reachability: OllamaProbe {
            client: ollama_client,
        },
        ollama_model: resolved.ollama_model,
        openai_model: resolved.openai_model,
    };

    let compiler = CompilerRouter {
        translator: router,
        cache: FileCompileCache::default(),
    };

    let options = RunOptions {
        kind_hint: parse_kind_hint(resolved.lang.as_deref()),
        language_hint: resolved.lang,
        force_llm: resolved.force_llm,
        no_cache: resolved.no_cache,
        print_js: resolved.print_js,
        provider_selection: provider_to_selection(resolved.provider),
        model_override: cli_overrides.model,
        progress_mode: resolved_progress_mode(resolved.progress, resolved.verbose),
    };

    let mut engine = BoaEngine::new();
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

    println!("Beeno REPL (M2). Type .exit to quit.");
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
            config,
            lang,
            print_js,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        } => run_command(
            file,
            config,
            lang,
            print_js,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        Commands::Eval { code } => eval_command(code),
        Commands::Repl => repl_command(),
    }
}
