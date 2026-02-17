mod cli_defaults;
mod dispatch;
mod project_commands;
mod repl_helpers;
mod repl_web;
mod runtime_context;
mod self_heal;

use anyhow::{Context, Result, anyhow};
use klumo_compiler::{CompileRequest, Compiler, SourceKind};
use klumo_config::{CliRunOverrides, ProviderSetting};
use klumo_core::{ProgressMode, compile_file, eval_inline, run_file};
use klumo_engine::JsEngine;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

const REPL_HISTORY_LIMIT: usize = 20;
const DEFAULT_WEB_HOST: &str = "127.0.0.1";
const DEFAULT_WEB_PORT: u16 = 4173;

#[derive(Debug, Clone)]
struct WebServerConfig {
    host: String,
    port: u16,
    root_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct ApiRoute {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

type SharedApiRoutes = Arc<Mutex<HashMap<String, ApiRoute>>>;

#[derive(Debug)]
struct WebServerHandle {
    config: WebServerConfig,
    url: String,
    stop_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl WebServerHandle {
    fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug)]
struct WebServerState {
    active: Option<WebServerHandle>,
    last_config: Option<WebServerConfig>,
    api_routes: SharedApiRoutes,
}

impl Default for WebServerState {
    fn default() -> Self {
        Self {
            active: None,
            last_config: None,
            api_routes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Drop for WebServerState {
    fn drop(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.stop();
        }
    }
}

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
#[command(name = "klumo", version, about = "Klumo runtime (M2 UX)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a file in Klumo.
    Run {
        file: Option<PathBuf>,
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
        self_heal: bool,
        #[arg(long, default_value_t = 1)]
        max_heal_attempts: usize,
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
    /// Compile a source file into JavaScript.
    Bundle {
        file: PathBuf,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
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
    /// Install project dependencies from klumo.json.
    #[command(visible_alias = "i")]
    Install {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Lint source files (Deno-compatible defaults).
    Lint {
        #[arg(long)]
        fix: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Format source files (Deno-compatible defaults).
    Fmt {
        #[arg(long)]
        check: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Run tests (Deno-compatible defaults).
    Test {
        #[arg(value_name = "ARGS", allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<OsString>,
    },
    /// Evaluate inline JavaScript.
    Eval { code: String },
    /// Start a JavaScript REPL.
    Repl {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        print_js: bool,
        #[arg(long)]
        no_cache: bool,
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
}

fn normalize_cli_args<I>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = OsString>,
{
    cli_defaults::normalize_cli_args(args)
}

fn warn_predefined_script_collisions() -> Result<()> {
    cli_defaults::warn_predefined_script_collisions()
}

fn web_server_scope_text(state: &WebServerState) -> String {
    repl_web::web_server_scope_text(state)
}

fn install_repl_web_javascript_api(engine: &mut dyn JsEngine) -> Result<()> {
    repl_web::install_repl_web_javascript_api(engine)
}

fn drain_repl_web_commands(engine: &mut dyn JsEngine) -> Result<Vec<JsonValue>> {
    repl_web::drain_repl_web_commands(engine)
}

fn write_repl_web_status(engine: &mut dyn JsEngine, state: &WebServerState) -> Result<()> {
    repl_web::write_repl_web_status(engine, state)
}

fn apply_repl_web_commands(commands: Vec<JsonValue>, state: &mut WebServerState) -> Result<()> {
    repl_web::apply_repl_web_commands(commands, state)
}

fn handle_web_command(input: &str, state: &mut WebServerState) -> Result<()> {
    repl_web::handle_web_command(input, state)
}

fn print_web_usage() {
    repl_web::print_web_usage();
}

fn resolve_run_script_target(config: Option<&Path>, target: &Path) -> Result<Option<String>> {
    project_commands::resolve_run_script_target(config, target)
}

fn install_dependencies(config: Option<PathBuf>, dry_run: bool) -> Result<()> {
    project_commands::install_dependencies(config, dry_run)
}

fn lint_command(paths: Vec<PathBuf>, fix: bool) -> Result<()> {
    project_commands::lint_command(paths, fix)
}

fn fmt_command(paths: Vec<PathBuf>, check: bool) -> Result<()> {
    project_commands::fmt_command(paths, check)
}

fn test_command(args: Vec<OsString>) -> Result<()> {
    project_commands::test_command(args)
}

fn run_script_command(script_name: &str, command_line: &str) -> Result<()> {
    project_commands::run_script_command(script_name, command_line)
}

#[allow(clippy::too_many_arguments)]
fn run_command(
    file: PathBuf,
    config: Option<PathBuf>,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    force_llm: bool,
    self_heal: bool,
    max_heal_attempts: usize,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    if let Some(script) = resolve_run_script_target(config.as_deref(), &file)? {
        let script_name = file.to_string_lossy().to_string();
        return run_script_command(&script_name, &script);
    }

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

    let resolved = runtime_context::resolve_config(config, &cli_overrides)?;
    let compiler = runtime_context::build_compiler(&resolved)?;
    let options = runtime_context::build_run_options(&resolved, cli_overrides.model.clone());

    let mut engine = runtime_context::build_engine()?;
    let mut outcome = None;
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=max_heal_attempts {
        match run_file(engine.as_mut(), &compiler, &file, &options) {
            Ok(ok) => {
                outcome = Some(ok);
                break;
            }
            Err(err) => {
                if !self_heal {
                    return Err(err).with_context(|| format!("failed running {}", file.display()));
                }
                if !self_heal::is_self_heal_supported_source(&file) {
                    return Err(err).with_context(|| {
                        format!(
                            "failed running {} (self-heal currently supports .js/.mjs/.cjs/.jsx)",
                            file.display()
                        )
                    });
                }
                if attempt >= max_heal_attempts {
                    last_err = Some(err);
                    break;
                }

                let error_text = format!("{err:#}");
                if !matches!(options.progress_mode, ProgressMode::Silent) {
                    eprintln!(
                        "[klumo] runtime failed, attempting self-heal ({}/{})",
                        attempt + 1,
                        max_heal_attempts
                    );
                }

                if let Err(heal_err) =
                    self_heal::try_self_heal(&compiler, &file, &options, &error_text, attempt)
                {
                    return Err(heal_err)
                        .with_context(|| format!("self-heal failed for {}", file.display()));
                }
            }
        }
    }

    let outcome = match outcome {
        Some(value) => value,
        None => {
            let err = last_err
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "unknown error".to_string());
            return Err(anyhow!(
                "failed running {} after {} self-heal attempts: {}",
                file.display(),
                max_heal_attempts,
                err
            ));
        }
    };

    if let Some(value) = outcome.eval.value {
        println!("{value}");
    }

    Ok(())
}

fn default_bundle_output(file: &std::path::Path) -> PathBuf {
    let mut out = file.to_path_buf();
    out.set_extension("bundle.js");
    out
}

#[allow(clippy::too_many_arguments)]
fn bundle_command(
    file: PathBuf,
    output: Option<PathBuf>,
    config: Option<PathBuf>,
    lang: Option<String>,
    no_cache: bool,
    force_llm: bool,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: force_llm.then_some(true),
        print_js: None,
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };

    let resolved = runtime_context::resolve_config(config, &cli_overrides)?;
    let compiler = runtime_context::build_compiler(&resolved)?;
    let options = runtime_context::build_run_options(&resolved, cli_overrides.model.clone());

    let compiled = compile_file(&compiler, &file, &options)
        .with_context(|| format!("failed bundling {}", file.display()))?;

    let target = output.unwrap_or_else(|| default_bundle_output(&file));
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating output dir {}", parent.display()))?;
        }
    }

    fs::write(&target, &compiled.javascript)
        .with_context(|| format!("failed writing bundle {}", target.display()))?;

    match options.progress_mode {
        ProgressMode::Silent => {}
        ProgressMode::Minimal => {
            if let Some(provider) = compiled.metadata.provider {
                let model = compiled.metadata.model.unwrap_or_default();
                eprintln!(
                    "[klumo] bundled via {}:{} (cache_hit={})",
                    format!("{provider:?}").to_ascii_lowercase(),
                    model,
                    compiled.metadata.cache_hit
                );
            }
            eprintln!("[klumo] wrote bundle {}", target.display());
        }
        ProgressMode::Verbose => {
            eprintln!(
                "[klumo] bundle compile complete provider={:?} model={:?} cache_hit={}",
                compiled.metadata.provider, compiled.metadata.model, compiled.metadata.cache_hit
            );
            eprintln!("[klumo] wrote bundle {}", target.display());
        }
    }

    println!("{}", target.display());
    Ok(())
}

fn eval_command(code: String) -> Result<()> {
    let mut engine = runtime_context::build_engine()?;
    let out = eval_inline(engine.as_mut(), &code)?;
    if let Some(value) = out.value {
        println!("{value}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn repl_command(
    config: Option<PathBuf>,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: None,
        print_js: print_js.then_some(true),
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };
    let resolved = runtime_context::resolve_config(config, &cli_overrides)?;
    let compiler = runtime_context::build_compiler(&resolved)?;

    let mut engine = runtime_context::build_engine()?;
    install_repl_web_javascript_api(engine.as_mut())?;
    let baseline_globals = repl_helpers::read_global_names(engine.as_mut())?;
    let mut known_bindings: HashSet<String> = HashSet::new();
    let mut statement_history: VecDeque<String> = VecDeque::new();
    let mut js_history: VecDeque<String> = VecDeque::new();
    let mut web_server = WebServerState::default();
    let mut line = String::new();
    let repl_lang = resolved
        .lang
        .clone()
        .unwrap_or_else(|| "pseudocode".to_string());
    let provider_selection = runtime_context::provider_to_selection(resolved.provider);
    let self_heal_limit = repl_helpers::repl_self_heal_limit();

    println!("Klumo REPL (M2). Type .help for commands, .exit to quit.");
    write_repl_web_status(engine.as_mut(), &web_server)?;
    loop {
        line.clear();
        print!("klumo> ");
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
        if trimmed == ".help" {
            println!("REPL commands:");
            println!("  .help - show this help");
            println!("  .exit - quit");
            print_web_usage();
            println!("JavaScript web APIs:");
            println!("  klumo.web.start({{ dir, port, host, open, noOpenPrompt }})");
            println!("  klumo.web.stop()");
            println!("  klumo.web.restart({{ dir, port, host, open }})");
            println!("  klumo.web.open()");
            println!("  klumo.web.status()");
            println!("  klumo.web.routeJson(path, payload, {{ status }})");
            println!("  klumo.web.routeText(path, text, {{ status, contentType }})");
            println!("  klumo.web.unroute(path)");
            continue;
        }
        if trimmed.starts_with(".web") {
            if let Err(err) = handle_web_command(trimmed, &mut web_server) {
                eprintln!("error: {err:#}");
            }
            if let Err(err) = write_repl_web_status(engine.as_mut(), &web_server) {
                eprintln!("error: failed refreshing JS web status: {err:#}");
            }
            continue;
        }
        if trimmed == ".exit" {
            break;
        }

        let compiled = compiler.compile(&CompileRequest {
            source_text: trimmed.to_string(),
            source_id: "<repl>".to_string(),
            kind_hint: Some(SourceKind::Unknown(repl_lang.clone())),
            language_hint: Some(repl_lang.clone()),
            scope_context: repl_helpers::build_repl_scope_context(
                &known_bindings,
                &statement_history,
                &js_history,
                Some(&web_server_scope_text(&web_server)),
            ),
            force_llm: true,
            provider_selection,
            model_override: cli_overrides.model.clone(),
            no_cache: resolved.no_cache,
        });

        let mut candidate_js = match compiled {
            Ok(compiled) => {
                let sanitized_js = repl_helpers::sanitize_repl_javascript(&compiled.javascript);
                if sanitized_js.trim().is_empty() {
                    eprintln!("error: translated REPL code was empty after removing module syntax");
                    continue;
                }
                sanitized_js
            }
            Err(err) => {
                let mut healed: Option<String> = None;
                let initial_error = format!("{err:#}");
                let mut attempt = 0usize;
                while repl_helpers::can_continue_self_heal(attempt, self_heal_limit) {
                    eprintln!(
                        "[klumo] repl translation failed, attempting self-heal ({})",
                        attempt + 1,
                    );
                    let heal_prompt =
                        repl_helpers::build_repl_self_heal_request(
                            trimmed,
                            None,
                            &initial_error,
                            attempt,
                        );
                    let heal_scope = repl_helpers::build_repl_scope_context(
                        &known_bindings,
                        &statement_history,
                        &js_history,
                        Some(&web_server_scope_text(&web_server)),
                    );
                    match self_heal::compile_repl_heal_candidate(
                        &compiler,
                        &repl_lang,
                        provider_selection,
                        cli_overrides.model.clone(),
                        resolved.no_cache,
                        heal_scope,
                        heal_prompt,
                        attempt,
                    ) {
                        Ok(js) => {
                            healed = Some(js);
                            break;
                        }
                        Err(heal_err) => {
                            let heal_err_text = format!("{heal_err:#}");
                            eprintln!("error: self-heal compile failed: {heal_err_text}");
                            if repl_helpers::is_non_recoverable_self_heal_error(&heal_err_text) {
                                break;
                            }
                        }
                    }
                    attempt += 1;
                }
                match healed {
                    Some(js) => js,
                    None => {
                        eprintln!("error: {initial_error}");
                        continue;
                    }
                }
            }
        };

        if resolved.verbose || resolved.print_js {
            println!("/* ===== generated JavaScript ===== */");
            println!("{}", candidate_js);
            println!("/* ===== end generated JavaScript ===== */");
        }

        let mut eval_output = None;
        let mut final_runtime_error: Option<String> = None;
        let mut attempt = 0usize;
        while repl_helpers::can_continue_self_heal(attempt, self_heal_limit) {
            match engine.as_mut().eval_script(&candidate_js, "<repl>") {
                Ok(output) => {
                    eval_output = Some(output);
                    break;
                }
                Err(err) => {
                    let err_text = format!("{err:#}");
                    eprintln!(
                        "[klumo] repl runtime failed, attempting self-heal ({})",
                        attempt + 1,
                    );
                    let heal_prompt = repl_helpers::build_repl_self_heal_request(
                        trimmed,
                        Some(&candidate_js),
                        &err_text,
                        attempt,
                    );
                    let heal_scope = repl_helpers::build_repl_scope_context(
                        &known_bindings,
                        &statement_history,
                        &js_history,
                        Some(&web_server_scope_text(&web_server)),
                    );
                    match self_heal::compile_repl_heal_candidate(
                        &compiler,
                        &repl_lang,
                        provider_selection,
                        cli_overrides.model.clone(),
                        resolved.no_cache,
                        heal_scope,
                        heal_prompt,
                        attempt,
                    ) {
                        Ok(healed_js) => {
                            candidate_js = healed_js;
                            if resolved.verbose || resolved.print_js {
                                println!("/* ===== healed JavaScript ===== */");
                                println!("{}", candidate_js);
                                println!("/* ===== end healed JavaScript ===== */");
                            }
                        }
                        Err(heal_err) => {
                            let heal_err_text = format!("{heal_err:#}");
                            eprintln!("error: self-heal compile failed: {heal_err_text}");
                            if repl_helpers::is_non_recoverable_self_heal_error(&heal_err_text) {
                                final_runtime_error = Some(err_text);
                                break;
                            }
                        }
                    }
                }
            }
            attempt += 1;
        }
        if eval_output.is_none() && final_runtime_error.is_none() {
            final_runtime_error = Some(
                "repl self-heal limit reached before successful execution (set KLUMO_REPL_SELF_HEAL_MAX_ATTEMPTS=0 for unlimited retries)"
                    .to_string(),
            );
        }

        if let Some(output) = eval_output {
            repl_helpers::push_bounded(
                &mut statement_history,
                trimmed.to_string(),
                REPL_HISTORY_LIMIT,
            );
            repl_helpers::push_bounded(&mut js_history, candidate_js.clone(), REPL_HISTORY_LIMIT);

            if let Some(value) = output.value {
                println!("{value}");
            }
            if let Ok(current) = repl_helpers::read_global_names(engine.as_mut()) {
                known_bindings = current
                    .difference(&baseline_globals)
                    .filter(|name| !name.starts_with("__klumo_"))
                    .cloned()
                    .collect();
            }
        } else if let Some(err) = final_runtime_error {
            eprintln!("error: {err}");
        }

        match drain_repl_web_commands(engine.as_mut()) {
            Ok(commands) => {
                if let Err(err) = apply_repl_web_commands(commands, &mut web_server) {
                    eprintln!("error: {err:#}");
                }
            }
            Err(err) => eprintln!("error: failed reading JS web command queue: {err:#}"),
        }
        if let Err(err) = write_repl_web_status(engine.as_mut(), &web_server) {
            eprintln!("error: failed refreshing JS web status: {err:#}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
use super::{
        DEFAULT_WEB_HOST, DEFAULT_WEB_PORT, REPL_HISTORY_LIMIT, normalize_cli_args,
    };
    use super::{cli_defaults, project_commands, repl_helpers, repl_web, self_heal};
    use klumo_config::FileConfig;
    use std::collections::{HashSet, VecDeque};
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn repl_scope_context_includes_bindings_and_history() {
        let mut bindings = HashSet::new();
        bindings.insert("hello".to_string());
        bindings.insert("count".to_string());

        let statements = VecDeque::from(vec![
            "store 2 in hello variable".to_string(),
            "print hello variable".to_string(),
        ]);
        let js = VecDeque::from(vec![
            "const hello = 2;".to_string(),
            "console.log(hello);".to_string(),
        ]);

        let context = repl_helpers::build_repl_scope_context(&bindings, &statements, &js, None)
            .expect("context");
        assert!(context.contains("Bindings currently defined"));
        assert!(context.contains("Previously run REPL statements"));
        assert!(context.contains("Previously generated JavaScript snippets"));
    }

    #[test]
    fn web_start_parser_applies_defaults() {
        let (config, open_override, ask_open) = repl_web::parse_web_start(&[]).expect("parse");
        assert_eq!(config.host, DEFAULT_WEB_HOST);
        assert_eq!(config.port, DEFAULT_WEB_PORT);
        assert!(open_override.is_none());
        assert!(ask_open);
    }

    #[test]
    fn web_start_parser_supports_flags() {
        let (config, open_override, ask_open) = repl_web::parse_web_start(&[
            "--host", "0.0.0.0", "--port", "8080", "--dir", "web", "--open",
        ])
        .expect("parse");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.root_dir.to_string_lossy(), "web");
        assert_eq!(open_override, Some(true));
        assert!(!ask_open);
    }

    #[test]
    fn route_path_normalizes_missing_leading_slash() {
        let normalized = repl_web::route_path("api/health").expect("path");
        assert_eq!(normalized, "/api/health");
    }

    #[test]
    fn route_path_rejects_parent_segments() {
        let err = repl_web::route_path("../escape").expect_err("should reject");
        assert!(err.to_string().contains("invalid route path"));
    }

    #[test]
    fn push_bounded_trims_old_entries() {
        let mut history = VecDeque::new();
        for i in 0..=(REPL_HISTORY_LIMIT + 2) {
            repl_helpers::push_bounded(&mut history, format!("entry-{i}"), REPL_HISTORY_LIMIT);
        }
        assert_eq!(history.len(), REPL_HISTORY_LIMIT);
        assert_eq!(history.front().expect("front"), "entry-3");
    }

    #[test]
    fn self_heal_supported_extensions_are_limited() {
        assert!(self_heal::is_self_heal_supported_source(Path::new("a.js")));
        assert!(self_heal::is_self_heal_supported_source(Path::new("a.mjs")));
        assert!(self_heal::is_self_heal_supported_source(Path::new("a.cjs")));
        assert!(self_heal::is_self_heal_supported_source(Path::new("a.jsx")));
        assert!(!self_heal::is_self_heal_supported_source(Path::new("a.ts")));
        assert!(!self_heal::is_self_heal_supported_source(Path::new("a.pseudo")));
    }

    #[test]
    fn backup_path_is_derived_from_file_name() {
        let backup = self_heal::backup_path_for(Path::new("/tmp/demo.js"));
        assert_eq!(backup.to_string_lossy(), "/tmp/demo.js.klumo.bak");
    }

    #[test]
    fn self_heal_prompt_contains_error_and_source() {
        let prompt = self_heal::build_self_heal_request(
            Path::new("demo.js"),
            "console.log(1)",
            "ReferenceError",
        );
        assert!(prompt.contains("demo.js"));
        assert!(prompt.contains("ReferenceError"));
        assert!(prompt.contains("console.log(1)"));
        assert!(prompt.contains("Return ONLY complete JavaScript source"));
    }

    #[test]
    fn repl_self_heal_prompt_contains_structured_error_report() {
        let prompt = repl_helpers::build_repl_self_heal_request(
            "print user profile",
            Some("const x = y;"),
            "failed evaluating <repl>: ReferenceError: y is not defined",
            0,
        );
        assert!(prompt.contains("FAILURE REPORT"));
        assert!(prompt.contains("Failure stage: runtime execution after JS generation"));
        assert!(prompt.contains("Probable cause: Undefined variable or symbol usage."));
        assert!(prompt.contains("FULL ERROR OUTPUT"));
        assert!(prompt.contains("const x = y;"));
    }

    #[test]
    fn self_heal_limit_predicate_handles_unlimited_and_bounded() {
        assert!(repl_helpers::can_continue_self_heal(0, None));
        assert!(repl_helpers::can_continue_self_heal(10, None));
        assert!(repl_helpers::can_continue_self_heal(0, Some(1)));
        assert!(!repl_helpers::can_continue_self_heal(1, Some(1)));
    }

    #[test]
    fn non_recoverable_error_detection_matches_provider_failures() {
        assert!(repl_helpers::is_non_recoverable_self_heal_error(
            "OPENAI_API_KEY is required for OpenAI-compatible translation"
        ));
        assert!(repl_helpers::is_non_recoverable_self_heal_error(
            "llm unavailable"
        ));
        assert!(!repl_helpers::is_non_recoverable_self_heal_error(
            "ReferenceError: value is not defined"
        ));
    }

    #[test]
    fn dependency_to_jsr_spec_supports_scoped_packages() {
        assert_eq!(
            project_commands::dependency_to_jsr_spec("@arvid/is-char", "latest"),
            "jsr:@arvid/is-char@latest"
        );
        assert_eq!(
            project_commands::dependency_to_jsr_spec("jsr:@scope/pkg", "1.2.3"),
            "jsr:@scope/pkg@1.2.3"
        );
        assert_eq!(
            project_commands::dependency_to_jsr_spec("@scope/pkg", ""),
            "jsr:@scope/pkg"
        );
    }

    #[test]
    fn normalize_cli_args_infers_run_for_bare_file() {
        let args = vec![OsString::from("klumo"), OsString::from("app.js")];
        let normalized = normalize_cli_args(args);
        assert_eq!(normalized[1], OsString::from("run"));
        assert_eq!(normalized[2], OsString::from("app.js"));
    }

    #[test]
    fn normalize_cli_args_keeps_install_alias() {
        let args = vec![OsString::from("klumo"), OsString::from("i")];
        let normalized = normalize_cli_args(args);
        assert_eq!(normalized[1], OsString::from("i"));
        assert_eq!(normalized.len(), 2);
    }

    #[test]
    fn normalize_cli_args_keeps_lint_command() {
        let args = vec![OsString::from("klumo"), OsString::from("lint")];
        let normalized = normalize_cli_args(args);
        assert_eq!(normalized[1], OsString::from("lint"));
        assert_eq!(normalized.len(), 2);
    }

    #[test]
    fn predefined_script_collisions_detects_reserved_names() {
        let cfg = FileConfig {
            scripts: Some(
                [
                    ("lint".to_string(), "echo lint".to_string()),
                    ("start".to_string(), "echo start".to_string()),
                    ("i".to_string(), "echo install".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            ..FileConfig::default()
        };

        let collisions = cli_defaults::predefined_script_collisions(&cfg);
        assert_eq!(collisions, vec!["i".to_string(), "lint".to_string()]);
    }

    #[test]
    fn deno_tooling_preferred_for_deno_config_projects() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("deno.json"), "{}").expect("write deno.json");
        assert!(project_commands::should_prefer_deno_tooling(dir.path()));
    }

    #[test]
    fn deno_tooling_not_preferred_for_rust_workspace_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\nversion='0.1.0'")
            .expect("write Cargo.toml");
        std::fs::write(dir.path().join("package.json"), "{}").expect("write package.json");
        assert!(!project_commands::should_prefer_deno_tooling(dir.path()));
    }
}

fn main() -> Result<()> {
    warn_predefined_script_collisions()?;
    let cli = Cli::parse_from(normalize_cli_args(std::env::args_os()));
    dispatch::execute(cli)
}
