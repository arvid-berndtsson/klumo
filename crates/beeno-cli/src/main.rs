use anyhow::{Context, Result, anyhow};
use beeno_compiler::{CompileRequest, Compiler, CompilerRouter, FileCompileCache, SourceKind};
use beeno_config::{
    CliRunOverrides, EnvConfig, ProgressSetting, ProviderSetting, RunDefaults, load_file_config,
    resolve_run_defaults,
};
use beeno_core::{ProgressMode, RunOptions, compile_file, eval_inline, run_file};
use beeno_engine::{BoaEngine, JsEngine};
use beeno_engine_v8::V8Engine;
use beeno_llm::{
    LlmClient, LlmTranslateRequest, ProviderRouter, ProviderSelection, ReachabilityProbe,
};
use beeno_llm_ollama::OllamaClient;
use beeno_llm_openai::OpenAiCompatibleClient;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const REPL_HISTORY_LIMIT: usize = 20;

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
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a file in Beeno.
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

struct OllamaProbe {
    client: OllamaClient,
}

type BeenoProviderRouter = ProviderRouter<OllamaClient, MaybeOpenAiClient, OllamaProbe>;
type BeenoCompiler = CompilerRouter<BeenoProviderRouter, FileCompileCache>;

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

fn sanitize_repl_javascript(input: &str) -> String {
    let mut output = String::new();
    for line in input.lines() {
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("export default ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("export ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if trimmed.starts_with("import ") {
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output.trim_end().to_string()
}

fn read_global_names(engine: &mut dyn JsEngine) -> Result<HashSet<String>> {
    let out = engine
        .eval_script(
            "JSON.stringify(Object.getOwnPropertyNames(globalThis))",
            "<repl-scope>",
        )
        .context("failed reading REPL global scope")?;
    let raw = out
        .value
        .ok_or_else(|| anyhow!("scope probe returned empty result"))?;
    let names: Vec<String> =
        serde_json::from_str(&raw).context("failed parsing REPL global scope JSON")?;
    Ok(names.into_iter().collect())
}

fn scope_context_text(bindings: &HashSet<String>) -> Option<String> {
    if bindings.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = bindings.iter().map(String::as_str).collect();
    names.sort_unstable();
    Some(format!(
        "Bindings currently defined in this REPL session: {}. Avoid redeclaring them with const/let/class.",
        names.join(", ")
    ))
}

fn joined_recent_entries(history: &VecDeque<String>) -> Option<String> {
    if history.is_empty() {
        return None;
    }

    let joined = history
        .iter()
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n");
    Some(joined)
}

fn build_repl_scope_context(
    bindings: &HashSet<String>,
    statement_history: &VecDeque<String>,
    js_history: &VecDeque<String>,
) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(bindings_text) = scope_context_text(bindings) {
        sections.push(bindings_text);
    }
    if let Some(statements) = joined_recent_entries(statement_history) {
        sections.push(format!(
            "Previously run REPL statements (oldest to newest):\n{}",
            statements
        ));
    }
    if let Some(js_snippets) = joined_recent_entries(js_history) {
        sections.push(format!(
            "Previously generated JavaScript snippets (oldest to newest):\n{}",
            js_snippets
        ));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn push_bounded(history: &mut VecDeque<String>, item: String, cap: usize) {
    history.push_back(item);
    while history.len() > cap {
        history.pop_front();
    }
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

fn build_run_options(resolved: &RunDefaults, model_override: Option<String>) -> RunOptions {
    RunOptions {
        kind_hint: parse_kind_hint(resolved.lang.as_deref()),
        language_hint: resolved.lang.clone(),
        force_llm: resolved.force_llm,
        no_cache: resolved.no_cache,
        print_js: resolved.print_js,
        provider_selection: provider_to_selection(resolved.provider),
        model_override,
        progress_mode: resolved_progress_mode(resolved.progress, resolved.verbose),
    }
}

fn resolve_config(config: Option<PathBuf>, cli_overrides: &CliRunOverrides) -> Result<RunDefaults> {
    let cwd = std::env::current_dir().context("failed getting current directory")?;
    let file_cfg = load_file_config(config.as_deref(), &cwd)?;
    let env_cfg = EnvConfig::from_current_env();
    Ok(resolve_run_defaults(
        cli_overrides,
        &env_cfg,
        file_cfg.as_ref(),
    ))
}

fn build_compiler(resolved: &RunDefaults) -> Result<BeenoCompiler> {
    let ollama_client = OllamaClient::new(resolved.ollama_url.clone())?;
    let openai_client = MaybeOpenAiClient {
        inner: resolved.openai_api_key.clone().map(|api_key| {
            OpenAiCompatibleClient::from_parts(resolved.openai_base_url.clone(), api_key)
        }),
    };

    let router = ProviderRouter {
        ollama: ollama_client.clone(),
        openai: openai_client,
        reachability: OllamaProbe {
            client: ollama_client,
        },
        ollama_model: resolved.ollama_model.clone(),
        openai_model: resolved.openai_model.clone(),
    };

    Ok(CompilerRouter {
        translator: router,
        cache: FileCompileCache::default(),
    })
}

fn build_engine() -> Result<Box<dyn JsEngine>> {
    let selected = std::env::var("BEENO_ENGINE").unwrap_or_else(|_| "boa".to_string());
    match selected.trim().to_ascii_lowercase().as_str() {
        "boa" => Ok(Box::new(BoaEngine::new())),
        "v8" => Ok(Box::new(V8Engine::new()?)),
        other => Err(anyhow!("unknown engine '{other}'. Supported: 'boa', 'v8'")),
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
    self_heal: bool,
    max_heal_attempts: usize,
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
        print_js: print_js.then_some(true),
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };

    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;
    let options = build_run_options(&resolved, cli_overrides.model.clone());

    let mut engine = build_engine()?;
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
                if !is_self_heal_supported_source(&file) {
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
                        "[beeno] runtime failed, attempting self-heal ({}/{})",
                        attempt + 1,
                        max_heal_attempts
                    );
                }

                if let Err(heal_err) =
                    try_self_heal(&compiler, &file, &options, &error_text, attempt)
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

fn is_self_heal_supported_source(file: &Path) -> bool {
    file.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "js" | "mjs" | "cjs" | "jsx"
            )
        })
        .unwrap_or(false)
}

fn backup_path_for(file: &Path) -> PathBuf {
    let mut backup = file.as_os_str().to_os_string();
    backup.push(".beeno.bak");
    PathBuf::from(backup)
}

fn build_self_heal_request(path: &Path, source: &str, error_text: &str) -> String {
    format!(
        "Repair this JavaScript file so it runs successfully.\n\
Return ONLY complete JavaScript source for the full file, no markdown, no prose.\n\
Preserve behavior and structure as much as possible.\n\
File: {}\n\
Runtime error:\n{}\n\
SOURCE START\n{}\n\
SOURCE END",
        path.display(),
        error_text,
        source
    )
}

fn try_self_heal(
    compiler: &BeenoCompiler,
    file: &Path,
    options: &RunOptions,
    error_text: &str,
    attempt: usize,
) -> Result<()> {
    let current_source = fs::read_to_string(file)
        .with_context(|| format!("failed reading source for self-heal {}", file.display()))?;

    let backup = backup_path_for(file);
    if !backup.exists() {
        fs::copy(file, &backup).with_context(|| {
            format!(
                "failed creating self-heal backup {} -> {}",
                file.display(),
                backup.display()
            )
        })?;
    }

    if !matches!(options.progress_mode, ProgressMode::Silent) {
        eprintln!(
            "[beeno] self-heal attempt {}: requesting file patch via LLM",
            attempt + 1
        );
    }

    let repaired = compiler.compile(&CompileRequest {
        source_text: build_self_heal_request(file, &current_source, error_text),
        source_id: format!("{}#self-heal-{}", file.display(), attempt + 1),
        kind_hint: Some(SourceKind::Unknown("self-heal".to_string())),
        language_hint: Some("self-heal-javascript".to_string()),
        scope_context: None,
        force_llm: true,
        provider_selection: options.provider_selection,
        model_override: options.model_override.clone(),
        no_cache: true,
    })?;

    if repaired.javascript.trim().is_empty() {
        return Err(anyhow!("self-heal generated empty output"));
    }

    fs::write(file, repaired.javascript)
        .with_context(|| format!("failed writing healed file {}", file.display()))?;

    if !matches!(options.progress_mode, ProgressMode::Silent) {
        eprintln!("[beeno] self-heal wrote patch to {}", file.display());
    }
    Ok(())
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

    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;
    let options = build_run_options(&resolved, cli_overrides.model.clone());

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
                    "[beeno] bundled via {}:{} (cache_hit={})",
                    format!("{provider:?}").to_ascii_lowercase(),
                    model,
                    compiled.metadata.cache_hit
                );
            }
            eprintln!("[beeno] wrote bundle {}", target.display());
        }
        ProgressMode::Verbose => {
            eprintln!(
                "[beeno] bundle compile complete provider={:?} model={:?} cache_hit={}",
                compiled.metadata.provider, compiled.metadata.model, compiled.metadata.cache_hit
            );
            eprintln!("[beeno] wrote bundle {}", target.display());
        }
    }

    println!("{}", target.display());
    Ok(())
}

fn eval_command(code: String) -> Result<()> {
    let mut engine = build_engine()?;
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
    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;

    let mut engine = build_engine()?;
    let baseline_globals = read_global_names(engine.as_mut())?;
    let mut known_bindings: HashSet<String> = HashSet::new();
    let mut statement_history: VecDeque<String> = VecDeque::new();
    let mut js_history: VecDeque<String> = VecDeque::new();
    let mut line = String::new();
    let repl_lang = resolved
        .lang
        .clone()
        .unwrap_or_else(|| "pseudocode".to_string());
    let provider_selection = provider_to_selection(resolved.provider);

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

        let compiled = compiler.compile(&CompileRequest {
            source_text: trimmed.to_string(),
            source_id: "<repl>".to_string(),
            kind_hint: Some(SourceKind::Unknown(repl_lang.clone())),
            language_hint: Some(repl_lang.clone()),
            scope_context: build_repl_scope_context(
                &known_bindings,
                &statement_history,
                &js_history,
            ),
            force_llm: true,
            provider_selection,
            model_override: cli_overrides.model.clone(),
            no_cache: resolved.no_cache,
        });

        match compiled {
            Ok(compiled) => {
                let sanitized_js = sanitize_repl_javascript(&compiled.javascript);
                if sanitized_js.trim().is_empty() {
                    eprintln!("error: translated REPL code was empty after removing module syntax");
                    continue;
                }

                if resolved.verbose || resolved.print_js {
                    println!("/* ===== generated JavaScript ===== */");
                    println!("{}", sanitized_js);
                    println!("/* ===== end generated JavaScript ===== */");
                }

                match engine.as_mut().eval_script(&sanitized_js, "<repl>") {
                    Ok(output) => {
                        push_bounded(
                            &mut statement_history,
                            trimmed.to_string(),
                            REPL_HISTORY_LIMIT,
                        );
                        push_bounded(&mut js_history, sanitized_js.clone(), REPL_HISTORY_LIMIT);

                        if let Some(value) = output.value {
                            println!("{value}");
                        }
                        if let Ok(current) = read_global_names(engine.as_mut()) {
                            known_bindings = current
                                .difference(&baseline_globals)
                                .filter(|name| !name.starts_with("__beeno_"))
                                .cloned()
                                .collect();
                        }
                    }
                    Err(err) => eprintln!("error: {err:#}"),
                }
            }
            Err(err) => eprintln!("error: {err:#}"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        REPL_HISTORY_LIMIT, backup_path_for, build_repl_scope_context, build_self_heal_request,
        is_self_heal_supported_source, push_bounded,
    };
    use std::collections::{HashSet, VecDeque};
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

        let context = build_repl_scope_context(&bindings, &statements, &js).expect("context");
        assert!(context.contains("Bindings currently defined"));
        assert!(context.contains("Previously run REPL statements"));
        assert!(context.contains("Previously generated JavaScript snippets"));
    }

    #[test]
    fn push_bounded_trims_old_entries() {
        let mut history = VecDeque::new();
        for i in 0..=(REPL_HISTORY_LIMIT + 2) {
            push_bounded(&mut history, format!("entry-{i}"), REPL_HISTORY_LIMIT);
        }
        assert_eq!(history.len(), REPL_HISTORY_LIMIT);
        assert_eq!(history.front().expect("front"), "entry-3");
    }

    #[test]
    fn self_heal_supported_extensions_are_limited() {
        assert!(is_self_heal_supported_source(Path::new("a.js")));
        assert!(is_self_heal_supported_source(Path::new("a.mjs")));
        assert!(is_self_heal_supported_source(Path::new("a.cjs")));
        assert!(is_self_heal_supported_source(Path::new("a.jsx")));
        assert!(!is_self_heal_supported_source(Path::new("a.ts")));
        assert!(!is_self_heal_supported_source(Path::new("a.pseudo")));
    }

    #[test]
    fn backup_path_is_derived_from_file_name() {
        let backup = backup_path_for(Path::new("/tmp/demo.js"));
        assert_eq!(backup.to_string_lossy(), "/tmp/demo.js.beeno.bak");
    }

    #[test]
    fn self_heal_prompt_contains_error_and_source() {
        let prompt =
            build_self_heal_request(Path::new("demo.js"), "console.log(1)", "ReferenceError");
        assert!(prompt.contains("demo.js"));
        assert!(prompt.contains("ReferenceError"));
        assert!(prompt.contains("console.log(1)"));
        assert!(prompt.contains("Return ONLY complete JavaScript source"));
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run {
            file,
            config,
            lang,
            print_js,
            no_cache,
            force_llm,
            self_heal,
            max_heal_attempts,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => {
            if let Some(path) = file {
                run_command(
                    path,
                    config,
                    lang,
                    print_js,
                    no_cache,
                    force_llm,
                    self_heal,
                    max_heal_attempts,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            } else {
                repl_command(
                    config,
                    lang,
                    print_js,
                    no_cache,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            }
        }
        Some(Commands::Eval { code }) => eval_command(code),
        Some(Commands::Bundle {
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => bundle_command(
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        Some(Commands::Repl {
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => repl_command(
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        None => repl_command(None, None, false, false, false, false, None, None, None),
    }
}
