use anyhow::{Context, Result};
use beeno_compiler::{CompileRequest, CompileResult, Compiler, SourceKind};
use beeno_engine::{EvalOutput, JsEngine};
use beeno_llm::ProviderSelection;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub kind_hint: Option<SourceKind>,
    pub language_hint: Option<String>,
    pub force_llm: bool,
    pub no_cache: bool,
    pub print_js: bool,
    pub provider_selection: ProviderSelection,
    pub model_override: Option<String>,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub compile: CompileResult,
    pub eval: EvalOutput,
}

pub fn run_file<E, C>(
    engine: &mut E,
    compiler: &C,
    path: &Path,
    options: &RunOptions,
) -> Result<RunOutcome>
where
    E: JsEngine,
    C: Compiler,
{
    if options.verbose {
        eprintln!("[beeno] loading source {}", path.display());
    }
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed reading script file {}", path.display()))?;

    if options.verbose {
        eprintln!("[beeno] compiling source (force_llm={})", options.force_llm);
    }
    let compile = compiler.compile(&CompileRequest {
        source_text: source,
        source_id: path.display().to_string(),
        kind_hint: options.kind_hint.clone(),
        language_hint: options.language_hint.clone(),
        force_llm: options.force_llm,
        provider_selection: options.provider_selection,
        model_override: options.model_override.clone(),
        no_cache: options.no_cache,
    })?;

    if options.print_js {
        println!("/* ===== generated JavaScript ===== */");
        println!("{}", compile.javascript);
        println!("/* ===== end generated JavaScript ===== */");
    }

    if options.verbose {
        eprintln!(
            "[beeno] compile complete provider={:?} model={:?} cache_hit={}",
            compile.metadata.provider, compile.metadata.model, compile.metadata.cache_hit
        );
        eprintln!("[beeno] executing JavaScript");
    }

    let eval = engine.eval_script(&compile.javascript, &path.display().to_string())?;
    if options.verbose {
        eprintln!("[beeno] execution complete");
    }
    Ok(RunOutcome { compile, eval })
}

pub fn eval_inline<E: JsEngine>(engine: &mut E, code: &str) -> Result<EvalOutput> {
    engine.eval_script(code, "<eval>")
}
