use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderSetting {
    Auto,
    Ollama,
    Openai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressSetting {
    Auto,
    Silent,
    Verbose,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub provider: Option<ProviderSetting>,
    pub ollama_url: Option<String>,
    pub ollama_model: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
    pub lang: Option<String>,
    pub force_llm: Option<bool>,
    pub print_js: Option<bool>,
    pub no_cache: Option<bool>,
    pub verbose: Option<bool>,
    pub progress: Option<ProgressSetting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvConfig {
    pub provider: Option<ProviderSetting>,
    pub ollama_url: Option<String>,
    pub ollama_model: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
    pub lang: Option<String>,
    pub force_llm: Option<bool>,
    pub print_js: Option<bool>,
    pub no_cache: Option<bool>,
    pub verbose: Option<bool>,
    pub progress: Option<ProgressSetting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliRunOverrides {
    pub provider: Option<ProviderSetting>,
    pub ollama_url: Option<String>,
    pub model: Option<String>,
    pub lang: Option<String>,
    pub force_llm: Option<bool>,
    pub print_js: Option<bool>,
    pub no_cache: Option<bool>,
    pub verbose: Option<bool>,
    pub no_progress: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDefaults {
    pub provider: ProviderSetting,
    pub ollama_url: String,
    pub ollama_model: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub lang: Option<String>,
    pub force_llm: bool,
    pub print_js: bool,
    pub no_cache: bool,
    pub verbose: bool,
    pub progress: ProgressSetting,
}

impl Default for RunDefaults {
    fn default() -> Self {
        Self {
            provider: ProviderSetting::Auto,
            ollama_url: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5-coder:7b".to_string(),
            openai_base_url: "https://api.openai.com/v1".to_string(),
            openai_model: "gpt-4.1-mini".to_string(),
            lang: None,
            force_llm: false,
            print_js: false,
            no_cache: false,
            verbose: false,
            progress: ProgressSetting::Auto,
        }
    }
}

pub fn load_file_config(explicit_path: Option<&Path>, cwd: &Path) -> Result<Option<FileConfig>> {
    let path = match explicit_path {
        Some(p) => p.to_path_buf(),
        None => {
            let candidate = cwd.join("beeno.json");
            if !candidate.exists() {
                return Ok(None);
            }
            candidate
        }
    };

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed reading config file {}", path.display()))?;
    let parsed: FileConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed parsing config file {}", path.display()))?;
    Ok(Some(parsed))
}

impl EnvConfig {
    pub fn from_current_env() -> Self {
        Self {
            provider: env::var("BEENO_PROVIDER")
                .ok()
                .and_then(|v| parse_provider(&v)),
            ollama_url: env::var("BEENO_OLLAMA_URL").ok(),
            ollama_model: env::var("BEENO_OLLAMA_MODEL").ok(),
            openai_base_url: env::var("OPENAI_BASE_URL").ok(),
            openai_model: env::var("BEENO_MODEL").ok(),
            lang: env::var("BEENO_LANG").ok(),
            force_llm: env::var("BEENO_FORCE_LLM").ok().and_then(|v| parse_bool(&v)),
            print_js: env::var("BEENO_PRINT_JS").ok().and_then(|v| parse_bool(&v)),
            no_cache: env::var("BEENO_NO_CACHE").ok().and_then(|v| parse_bool(&v)),
            verbose: env::var("BEENO_VERBOSE").ok().and_then(|v| parse_bool(&v)),
            progress: env::var("BEENO_PROGRESS")
                .ok()
                .and_then(|v| parse_progress(&v)),
        }
    }
}

pub fn resolve_run_defaults(
    cli: &CliRunOverrides,
    env_cfg: &EnvConfig,
    file_cfg: Option<&FileConfig>,
) -> RunDefaults {
    let base = RunDefaults::default();

    let provider = cli
        .provider
        .or(env_cfg.provider)
        .or(file_cfg.and_then(|c| c.provider))
        .unwrap_or(base.provider);

    let ollama_url = cli
        .ollama_url
        .clone()
        .or_else(|| env_cfg.ollama_url.clone())
        .or_else(|| file_cfg.and_then(|c| c.ollama_url.clone()))
        .unwrap_or(base.ollama_url);

    let ollama_model = cli
        .model
        .clone()
        .or_else(|| env_cfg.ollama_model.clone())
        .or_else(|| file_cfg.and_then(|c| c.ollama_model.clone()))
        .unwrap_or(base.ollama_model);

    let openai_base_url = env_cfg
        .openai_base_url
        .clone()
        .or_else(|| file_cfg.and_then(|c| c.openai_base_url.clone()))
        .unwrap_or(base.openai_base_url);

    let openai_model = cli
        .model
        .clone()
        .or_else(|| env_cfg.openai_model.clone())
        .or_else(|| file_cfg.and_then(|c| c.openai_model.clone()))
        .unwrap_or(base.openai_model);

    let lang = cli
        .lang
        .clone()
        .or_else(|| env_cfg.lang.clone())
        .or_else(|| file_cfg.and_then(|c| c.lang.clone()))
        .or(base.lang);

    let force_llm = cli
        .force_llm
        .or(env_cfg.force_llm)
        .or(file_cfg.and_then(|c| c.force_llm))
        .unwrap_or(base.force_llm);

    let print_js = cli
        .print_js
        .or(env_cfg.print_js)
        .or(file_cfg.and_then(|c| c.print_js))
        .unwrap_or(base.print_js);

    let no_cache = cli
        .no_cache
        .or(env_cfg.no_cache)
        .or(file_cfg.and_then(|c| c.no_cache))
        .unwrap_or(base.no_cache);

    let verbose = cli
        .verbose
        .or(env_cfg.verbose)
        .or(file_cfg.and_then(|c| c.verbose))
        .unwrap_or(base.verbose);

    let mut progress = env_cfg
        .progress
        .or(file_cfg.and_then(|c| c.progress))
        .unwrap_or(base.progress);

    if cli.no_progress == Some(true) {
        progress = ProgressSetting::Silent;
    }

    RunDefaults {
        provider,
        ollama_url,
        ollama_model,
        openai_base_url,
        openai_model,
        lang,
        force_llm,
        print_js,
        no_cache,
        verbose,
        progress,
    }
}

fn parse_bool(input: &str) -> Option<bool> {
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_provider(input: &str) -> Option<ProviderSetting> {
    match input.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ProviderSetting::Auto),
        "ollama" => Some(ProviderSetting::Ollama),
        "openai" | "openai-compatible" => Some(ProviderSetting::Openai),
        _ => None,
    }
}

fn parse_progress(input: &str) -> Option<ProgressSetting> {
    match input.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ProgressSetting::Auto),
        "silent" => Some(ProgressSetting::Silent),
        "verbose" => Some(ProgressSetting::Verbose),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliRunOverrides, EnvConfig, FileConfig, ProgressSetting, ProviderSetting,
        load_file_config, resolve_run_defaults,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn valid_config_parses() {
        let dir = tempdir().expect("tempdir should work");
        let path = dir.path().join("beeno.json");
        fs::write(&path, r#"{"provider":"ollama","force_llm":true}"#)
            .expect("write should work");

        let parsed = load_file_config(None, dir.path())
            .expect("parse should work")
            .expect("file should exist");
        assert_eq!(parsed.provider, Some(ProviderSetting::Ollama));
        assert_eq!(parsed.force_llm, Some(true));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let dir = tempdir().expect("tempdir should work");
        let path = dir.path().join("beeno.json");
        fs::write(&path, r#"{"unknown":1}"#).expect("write should work");

        let err = load_file_config(None, dir.path()).expect_err("parse should fail");
        assert!(format!("{err:#}").contains("unknown field"));
    }

    #[test]
    fn malformed_json_has_location() {
        let dir = tempdir().expect("tempdir should work");
        let path = dir.path().join("beeno.json");
        fs::write(&path, "{\n  \"provider\":\n").expect("write should work");

        let err = load_file_config(None, dir.path()).expect_err("parse should fail");
        assert!(
            format!("{err:#}").contains("line") || format!("{err:#}").contains("column"),
            "expected location details, got: {err}"
        );
    }

    #[test]
    fn precedence_cli_env_file_defaults() {
        let file = FileConfig {
            provider: Some(ProviderSetting::Openai),
            progress: Some(ProgressSetting::Verbose),
            force_llm: Some(false),
            ..FileConfig::default()
        };

        let env_cfg = EnvConfig {
            provider: Some(ProviderSetting::Ollama),
            force_llm: Some(false),
            ..EnvConfig::default()
        };

        let cli = CliRunOverrides {
            provider: Some(ProviderSetting::Auto),
            force_llm: Some(true),
            no_progress: Some(true),
            ..CliRunOverrides::default()
        };

        let resolved = resolve_run_defaults(&cli, &env_cfg, Some(&file));
        assert_eq!(resolved.provider, ProviderSetting::Auto);
        assert!(resolved.force_llm);
        assert_eq!(resolved.progress, ProgressSetting::Silent);
    }
}
