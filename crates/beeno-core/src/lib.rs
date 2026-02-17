use anyhow::{Context, Result};
use beeno_compiler::{CompileRequest, CompileResult, Compiler, SourceKind};
use beeno_engine::{EvalOutput, JsEngine};
use beeno_llm::ProviderSelection;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Silent,
    Minimal,
    Verbose,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub kind_hint: Option<SourceKind>,
    pub language_hint: Option<String>,
    pub force_llm: bool,
    pub no_cache: bool,
    pub print_js: bool,
    pub provider_selection: ProviderSelection,
    pub model_override: Option<String>,
    pub progress_mode: ProgressMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub compile: CompileResult,
    pub eval: EvalOutput,
}

pub fn compile_file<C>(compiler: &C, path: &Path, options: &RunOptions) -> Result<CompileResult>
where
    C: Compiler,
{
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed reading script file {}", path.display()))?;

    compiler.compile(&CompileRequest {
        source_text: source,
        source_id: path.display().to_string(),
        kind_hint: options.kind_hint.clone(),
        language_hint: options.language_hint.clone(),
        scope_context: None,
        force_llm: options.force_llm,
        provider_selection: options.provider_selection,
        model_override: options.model_override.clone(),
        no_cache: options.no_cache,
    })
}

pub fn run_file<E, C>(
    engine: &mut E,
    compiler: &C,
    path: &Path,
    options: &RunOptions,
) -> Result<RunOutcome>
where
    E: JsEngine + ?Sized,
    C: Compiler,
{
    if matches!(options.progress_mode, ProgressMode::Verbose) {
        eprintln!("[beeno] loading source {}", path.display());
    }
    if matches!(options.progress_mode, ProgressMode::Verbose) {
        eprintln!("[beeno] compiling source (force_llm={})", options.force_llm);
    }
    let compile = compile_file(compiler, path, options)?;

    let llm_path = compile.metadata.provider.is_some();
    if options.print_js || (matches!(options.progress_mode, ProgressMode::Verbose) && llm_path) {
        println!("/* ===== generated JavaScript ===== */");
        println!("{}", compile.javascript);
        println!("/* ===== end generated JavaScript ===== */");
    }

    match options.progress_mode {
        ProgressMode::Silent => {}
        ProgressMode::Minimal => {
            if llm_path {
                let provider = compile
                    .metadata
                    .provider
                    .map(|p| format!("{p:?}").to_ascii_lowercase())
                    .unwrap_or_else(|| "unknown".to_string());
                let model = compile.metadata.model.clone().unwrap_or_default();
                eprintln!(
                    "[beeno] compiling via {}:{} (cache_hit={})",
                    provider, model, compile.metadata.cache_hit
                );
                eprintln!("[beeno] executing");
            }
        }
        ProgressMode::Verbose => {
            eprintln!(
                "[beeno] compile complete provider={:?} model={:?} cache_hit={}",
                compile.metadata.provider, compile.metadata.model, compile.metadata.cache_hit
            );
            eprintln!("[beeno] executing JavaScript");
        }
    }

    let eval = engine.eval_script(&compile.javascript, &path.display().to_string())?;
    if matches!(options.progress_mode, ProgressMode::Verbose) {
        eprintln!("[beeno] execution complete");
    }
    Ok(RunOutcome { compile, eval })
}

pub fn eval_inline<E: JsEngine + ?Sized>(engine: &mut E, code: &str) -> Result<EvalOutput> {
    engine.eval_script(code, "<eval>")
}
