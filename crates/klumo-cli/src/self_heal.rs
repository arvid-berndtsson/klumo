use crate::repl_helpers;
use crate::runtime_context::KlumoCompiler;
use anyhow::{Context, Result, anyhow};
use klumo_compiler::{CompileRequest, Compiler, SourceKind};
use klumo_core::{ProgressMode, RunOptions};
use klumo_llm::ProviderSelection;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn compile_repl_heal_candidate(
    compiler: &KlumoCompiler,
    repl_lang: &str,
    provider_selection: ProviderSelection,
    model_override: Option<String>,
    no_cache: bool,
    scope_context: Option<String>,
    heal_prompt: String,
    attempt: usize,
) -> Result<String> {
    let healed = compiler.compile(&CompileRequest {
        source_text: heal_prompt,
        source_id: format!("<repl-self-heal-{attempt}>"),
        kind_hint: Some(SourceKind::Unknown(repl_lang.to_string())),
        language_hint: Some(repl_lang.to_string()),
        scope_context,
        force_llm: true,
        provider_selection,
        model_override,
        no_cache,
    })?;
    let sanitized_js = repl_helpers::sanitize_repl_javascript(&healed.javascript);
    if sanitized_js.trim().is_empty() {
        return Err(anyhow!(
            "self-heal generated empty JavaScript after module-syntax cleanup"
        ));
    }
    Ok(sanitized_js)
}

pub(crate) fn is_self_heal_supported_source(file: &Path) -> bool {
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

pub(crate) fn backup_path_for(file: &Path) -> PathBuf {
    let mut backup = file.as_os_str().to_os_string();
    backup.push(".klumo.bak");
    PathBuf::from(backup)
}

pub(crate) fn build_self_heal_request(path: &Path, source: &str, error_text: &str) -> String {
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

pub(crate) fn try_self_heal(
    compiler: &KlumoCompiler,
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
            "[klumo] self-heal attempt {}: requesting file patch via LLM",
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
        eprintln!("[klumo] self-heal wrote patch to {}", file.display());
    }
    Ok(())
}
