use anyhow::{Result, anyhow};
use klumo_compiler::{CompilerRouter, FileCompileCache, SourceKind};
use klumo_config::{
    CliRunOverrides, EnvConfig, ProgressSetting, ProviderSetting, RunDefaults, load_file_config,
    resolve_run_defaults,
};
use klumo_core::{ProgressMode, RunOptions};
use klumo_engine::{BoaEngine, JsEngine};
use klumo_engine_v8::V8Engine;
use klumo_llm::{
    LlmClient, LlmTranslateRequest, ProviderRouter, ProviderSelection, ReachabilityProbe,
};
use klumo_llm_ollama::OllamaClient;
use klumo_llm_openai::OpenAiCompatibleClient;
use std::path::PathBuf;

pub(crate) struct OllamaProbe {
    client: OllamaClient,
}

type KlumoProviderRouter = ProviderRouter<OllamaClient, MaybeOpenAiClient, OllamaProbe>;
pub(crate) type KlumoCompiler = CompilerRouter<KlumoProviderRouter, FileCompileCache>;

impl ReachabilityProbe for OllamaProbe {
    fn ollama_reachable(&self) -> bool {
        self.client.is_reachable()
    }
}

pub(crate) struct MaybeOpenAiClient {
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

pub(crate) fn provider_to_selection(provider: ProviderSetting) -> ProviderSelection {
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

pub(crate) fn build_run_options(resolved: &RunDefaults, model_override: Option<String>) -> RunOptions {
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

pub(crate) fn resolve_config(
    config: Option<PathBuf>,
    cli_overrides: &CliRunOverrides,
) -> Result<RunDefaults> {
    let cwd = std::env::current_dir()?;
    let file_cfg = load_file_config(config.as_deref(), &cwd)?;
    let env_cfg = EnvConfig::from_current_env();
    Ok(resolve_run_defaults(
        cli_overrides,
        &env_cfg,
        file_cfg.as_ref(),
    ))
}

pub(crate) fn build_compiler(resolved: &RunDefaults) -> Result<KlumoCompiler> {
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

pub(crate) fn build_engine() -> Result<Box<dyn JsEngine>> {
    let selected = std::env::var("KLUMO_ENGINE").unwrap_or_else(|_| "boa".to_string());
    match selected.trim().to_ascii_lowercase().as_str() {
        "boa" => Ok(Box::new(BoaEngine::new())),
        "v8" => Ok(Box::new(V8Engine::new()?)),
        other => Err(anyhow!("unknown engine '{other}'. Supported: 'boa', 'v8'")),
    }
}
