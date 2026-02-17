use anyhow::{Result, anyhow};
use beeno_compiler::{CompileCache, CompileResult, CompilerRouter, SourceKind};
use beeno_core::{ProgressMode, RunOptions, run_file};
use beeno_engine::BoaEngine;
use beeno_llm::{
    LlmTranslateRequest, LlmTranslateResponse, Provider, ProviderDescriptor, ProviderSelection,
    TranslationService,
};
use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

struct MemoryCache {
    data: Mutex<HashMap<String, CompileResult>>,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl CompileCache for MemoryCache {
    fn get(&self, key: &str) -> Option<CompileResult> {
        self.data.lock().expect("lock should work").get(key).cloned()
    }

    fn put(&self, key: &str, result: &CompileResult) -> Result<()> {
        self.data
            .lock()
            .expect("lock should work")
            .insert(key.to_string(), result.clone());
        Ok(())
    }
}

struct MockService {
    fail: bool,
    js: String,
    provider: Provider,
    model: String,
    chain: Vec<ProviderDescriptor>,
}

impl TranslationService for MockService {
    fn candidate_chain(&self, _selection: ProviderSelection) -> Vec<ProviderDescriptor> {
        self.chain.clone()
    }

    fn translate(
        &self,
        _selection: ProviderSelection,
        _req: &LlmTranslateRequest,
        _model_override: Option<&str>,
    ) -> Result<LlmTranslateResponse> {
        if self.fail {
            return Err(anyhow!("compile failed"));
        }

        Ok(LlmTranslateResponse {
            javascript: self.js.clone(),
            provider: self.provider,
            model: self.model.clone(),
        })
    }
}

fn options() -> RunOptions {
    RunOptions {
        kind_hint: None,
        language_hint: None,
        force_llm: false,
        no_cache: true,
        print_js: false,
        provider_selection: ProviderSelection::Auto,
        model_override: None,
        progress_mode: ProgressMode::Silent,
    }
}

#[test]
fn runs_js_file_without_llm() {
    let dir = tempdir().expect("tempdir should work");
    let file = dir.path().join("hello.js");
    fs::write(&file, "21 * 2").expect("write should work");

    let compiler = CompilerRouter {
        translator: MockService {
            fail: true,
            js: String::new(),
            provider: Provider::Ollama,
            model: "qwen".to_string(),
            chain: vec![],
        },
        cache: MemoryCache::default(),
    };

    let mut engine = BoaEngine::new();
    let outcome = run_file(&mut engine, &compiler, &file, &options()).expect("run should pass");
    assert_eq!(outcome.eval.value.as_deref(), Some("42"));
}

#[test]
fn runs_non_js_with_llm_compile() {
    let dir = tempdir().expect("tempdir should work");
    let file = dir.path().join("hello.pseudo");
    fs::write(&file, "write hello").expect("write should work");

    let compiler = CompilerRouter {
        translator: MockService {
            fail: false,
            js: "'compiled-' + 'ok'".to_string(),
            provider: Provider::Ollama,
            model: "qwen".to_string(),
            chain: vec![ProviderDescriptor {
                provider: Provider::Ollama,
                model: "qwen".to_string(),
            }],
        },
        cache: MemoryCache::default(),
    };

    let mut engine = BoaEngine::new();
    let outcome = run_file(
        &mut engine,
        &compiler,
        &file,
        &RunOptions {
            kind_hint: Some(SourceKind::Unknown("pseudo".to_string())),
            language_hint: Some("pseudocode".to_string()),
            ..options()
        },
    )
    .expect("run should pass");

    assert_eq!(outcome.eval.value.as_deref(), Some("compiled-ok"));
}

#[test]
fn compile_failure_returns_error() {
    let dir = tempdir().expect("tempdir should work");
    let file = dir.path().join("bad.pseudo");
    fs::write(&file, "broken").expect("write should work");

    let compiler = CompilerRouter {
        translator: MockService {
            fail: true,
            js: String::new(),
            provider: Provider::Ollama,
            model: "qwen".to_string(),
            chain: vec![ProviderDescriptor {
                provider: Provider::Ollama,
                model: "qwen".to_string(),
            }],
        },
        cache: MemoryCache::default(),
    };

    let mut engine = BoaEngine::new();
    let err = run_file(
        &mut engine,
        &compiler,
        &file,
        &RunOptions {
            kind_hint: Some(SourceKind::Unknown("pseudo".to_string())),
            language_hint: Some("pseudocode".to_string()),
            ..options()
        },
    )
    .expect_err("run should fail");

    assert!(err.to_string().contains("compile failed"));
}

#[test]
fn runtime_failure_returns_error() {
    let dir = tempdir().expect("tempdir should work");
    let file = dir.path().join("boom.js");
    fs::write(&file, "throw new Error('boom')").expect("write should work");

    let compiler = CompilerRouter {
        translator: MockService {
            fail: true,
            js: String::new(),
            provider: Provider::Ollama,
            model: "qwen".to_string(),
            chain: vec![],
        },
        cache: MemoryCache::default(),
    };

    let mut engine = BoaEngine::new();
    let err = run_file(&mut engine, &compiler, &file, &options()).expect_err("run should fail");
    assert!(err.to_string().contains("failed evaluating"));
}
