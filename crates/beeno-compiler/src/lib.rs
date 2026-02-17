use anyhow::{Context, Result};
use beeno_llm::{LlmTranslateRequest, Provider, ProviderSelection, TranslationService};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

pub const PROMPT_VERSION: &str = "m1-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    JavaScript,
    TypeScript,
    Unknown(String),
    Auto,
}

impl SourceKind {
    pub fn from_hint(hint: &str) -> Self {
        match hint.to_ascii_lowercase().as_str() {
            "js" | "mjs" | "cjs" | "javascript" => Self::JavaScript,
            "ts" | "typescript" => Self::TypeScript,
            "auto" => Self::Auto,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn infer_from_source_id(source_id: &str) -> Self {
        if let Some((_, ext)) = source_id.rsplit_once('.') {
            return Self::from_hint(ext);
        }
        Self::Unknown("unknown".to_string())
    }

    pub fn as_hint(&self) -> String {
        match self {
            Self::JavaScript => "javascript".to_string(),
            Self::TypeScript => "typescript".to_string(),
            Self::Unknown(v) => v.clone(),
            Self::Auto => "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompileRequest {
    pub source_text: String,
    pub source_id: String,
    pub kind_hint: Option<SourceKind>,
    pub language_hint: Option<String>,
    pub force_llm: bool,
    pub provider_selection: ProviderSelection,
    pub model_override: Option<String>,
    pub no_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileMetadata {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub prompt_version: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResult {
    pub javascript: String,
    pub metadata: CompileMetadata,
}

pub trait Compiler {
    fn compile(&self, req: &CompileRequest) -> Result<CompileResult>;
}

pub trait CompileCache {
    fn get(&self, key: &str) -> Option<CompileResult>;
    fn put(&self, key: &str, result: &CompileResult) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct FileCompileCache {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedResult {
    javascript: String,
    provider: Option<String>,
    model: Option<String>,
    prompt_version: String,
}

impl FileCompileCache {
    pub fn default_root() -> Result<PathBuf> {
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        Ok(home.join(".beeno").join("cache").join("compile"))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Default for FileCompileCache {
    fn default() -> Self {
        let root = Self::default_root().unwrap_or_else(|_| PathBuf::from(".beeno-cache"));
        Self { root }
    }
}

impl CompileCache for FileCompileCache {
    fn get(&self, key: &str) -> Option<CompileResult> {
        let path = self.root.join(format!("{key}.json"));
        let raw = fs::read_to_string(path).ok()?;
        let parsed: CachedResult = serde_json::from_str(&raw).ok()?;

        Some(CompileResult {
            javascript: parsed.javascript,
            metadata: CompileMetadata {
                provider: parsed.provider.as_deref().map(parse_provider),
                model: parsed.model,
                prompt_version: parsed.prompt_version,
                cache_hit: true,
            },
        })
    }

    fn put(&self, key: &str, result: &CompileResult) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed creating cache dir {}", self.root.display()))?;
        let path = self.root.join(format!("{key}.json"));

        let payload = CachedResult {
            javascript: result.javascript.clone(),
            provider: result.metadata.provider.map(format_provider),
            model: result.metadata.model.clone(),
            prompt_version: result.metadata.prompt_version.clone(),
        };

        let raw = serde_json::to_string_pretty(&payload).context("failed serializing cache payload")?;
        fs::write(path, raw).context("failed writing cache file")?;
        Ok(())
    }
}

pub struct CompilerRouter<T, C>
where
    T: TranslationService,
    C: CompileCache,
{
    pub translator: T,
    pub cache: C,
}

impl<T, C> CompilerRouter<T, C>
where
    T: TranslationService,
    C: CompileCache,
{
    fn resolved_kind(&self, req: &CompileRequest) -> SourceKind {
        match req.kind_hint.clone().unwrap_or(SourceKind::Auto) {
            SourceKind::Auto => SourceKind::infer_from_source_id(&req.source_id),
            explicit => explicit,
        }
    }

    fn cache_key(
        source_text: &str,
        source_id: &str,
        kind_hint: &str,
        provider: Provider,
        model: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(source_text.as_bytes());
        hasher.update(b"\n--source-id--\n");
        hasher.update(source_id.as_bytes());
        hasher.update(b"\n--kind--\n");
        hasher.update(kind_hint.as_bytes());
        hasher.update(b"\n--provider--\n");
        hasher.update(format_provider(provider).as_bytes());
        hasher.update(b"\n--model--\n");
        hasher.update(model.as_bytes());
        hasher.update(b"\n--prompt-version--\n");
        hasher.update(PROMPT_VERSION.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

impl<T, C> Compiler for CompilerRouter<T, C>
where
    T: TranslationService,
    C: CompileCache,
{
    fn compile(&self, req: &CompileRequest) -> Result<CompileResult> {
        let kind = self.resolved_kind(req);
        let kind_hint = req.language_hint.clone().unwrap_or_else(|| kind.as_hint());
        let needs_llm = req.force_llm || !matches!(kind, SourceKind::JavaScript);

        if !needs_llm {
            return Ok(CompileResult {
                javascript: req.source_text.clone(),
                metadata: CompileMetadata {
                    provider: None,
                    model: None,
                    prompt_version: PROMPT_VERSION.to_string(),
                    cache_hit: false,
                },
            });
        }

        if !req.no_cache {
            for candidate in self.translator.candidate_chain(req.provider_selection) {
                let model_for_key = req
                    .model_override
                    .as_deref()
                    .unwrap_or(&candidate.model)
                    .to_string();
                let key = Self::cache_key(
                    &req.source_text,
                    &req.source_id,
                    &kind_hint,
                    candidate.provider,
                    &model_for_key,
                );
                if let Some(cached) = self.cache.get(&key) {
                    return Ok(cached);
                }
            }
        }

        let translated = self.translator.translate(
            req.provider_selection,
            &LlmTranslateRequest {
                source_text: req.source_text.clone(),
                source_id: req.source_id.clone(),
                language_hint: Some(kind_hint.clone()),
            },
            req.model_override.as_deref(),
        )?;

        let result = CompileResult {
            javascript: translated.javascript,
            metadata: CompileMetadata {
                provider: Some(translated.provider),
                model: Some(translated.model.clone()),
                prompt_version: PROMPT_VERSION.to_string(),
                cache_hit: false,
            },
        };

        if !req.no_cache {
            let key = Self::cache_key(
                &req.source_text,
                &req.source_id,
                &kind_hint,
                translated.provider,
                &translated.model,
            );
            self.cache.put(&key, &result)?;
        }

        Ok(result)
    }
}

fn parse_provider(value: &str) -> Provider {
    if value == "ollama" {
        Provider::Ollama
    } else {
        Provider::OpenAiCompatible
    }
}

fn format_provider(provider: Provider) -> String {
    match provider {
        Provider::Ollama => "ollama".to_string(),
        Provider::OpenAiCompatible => "openai-compatible".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{CompileCache, CompileRequest, CompileResult, Compiler, CompilerRouter, FileCompileCache, PROMPT_VERSION, SourceKind};
    use anyhow::{Result, anyhow};
    use beeno_llm::{
        LlmTranslateRequest, LlmTranslateResponse, Provider, ProviderDescriptor, ProviderSelection,
        TranslationService,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    struct MockTranslator {
        fail: bool,
        response_js: String,
        provider: Provider,
        model: String,
        chain: Vec<ProviderDescriptor>,
    }

    impl TranslationService for MockTranslator {
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
                return Err(anyhow!("llm unavailable"));
            }

            Ok(LlmTranslateResponse {
                javascript: self.response_js.clone(),
                provider: self.provider,
                model: self.model.clone(),
            })
        }
    }

    #[derive(Default)]
    struct MemoryCache {
        map: Mutex<HashMap<String, CompileResult>>,
    }

    impl CompileCache for MemoryCache {
        fn get(&self, key: &str) -> Option<CompileResult> {
            self.map.lock().expect("lock must work").get(key).cloned()
        }

        fn put(&self, key: &str, result: &CompileResult) -> Result<()> {
            self.map
                .lock()
                .expect("lock must work")
                .insert(key.to_string(), result.clone());
            Ok(())
        }
    }

    fn pseudo_request() -> CompileRequest {
        CompileRequest {
            source_text: "write hello".to_string(),
            source_id: "sample.pseudo".to_string(),
            kind_hint: Some(SourceKind::Unknown("pseudo".to_string())),
            language_hint: Some("pseudocode".to_string()),
            force_llm: false,
            provider_selection: ProviderSelection::Auto,
            model_override: None,
            no_cache: false,
        }
    }

    #[test]
    fn passthrough_for_js_without_force_llm() {
        let router = CompilerRouter {
            translator: MockTranslator {
                fail: true,
                response_js: String::new(),
                provider: Provider::Ollama,
                model: "m".to_string(),
                chain: vec![],
            },
            cache: MemoryCache::default(),
        };

        let req = CompileRequest {
            source_text: "1+1".to_string(),
            source_id: "sample.js".to_string(),
            kind_hint: Some(SourceKind::JavaScript),
            language_hint: None,
            force_llm: false,
            provider_selection: ProviderSelection::Auto,
            model_override: None,
            no_cache: false,
        };

        let result = router.compile(&req).expect("compile should pass");
        assert_eq!(result.javascript, "1+1");
        assert_eq!(result.metadata.provider, None);
    }

    #[test]
    fn non_js_uses_llm() {
        let router = CompilerRouter {
            translator: MockTranslator {
                fail: false,
                response_js: "console.log('hi')".to_string(),
                provider: Provider::Ollama,
                model: "model".to_string(),
                chain: vec![ProviderDescriptor {
                    provider: Provider::Ollama,
                    model: "model".to_string(),
                }],
            },
            cache: MemoryCache::default(),
        };

        let result = router
            .compile(&pseudo_request())
            .expect("compile should use llm");
        assert_eq!(result.javascript, "console.log('hi')");
        assert_eq!(result.metadata.provider, Some(Provider::Ollama));
    }

    #[test]
    fn force_llm_on_js() {
        let router = CompilerRouter {
            translator: MockTranslator {
                fail: false,
                response_js: "2+2".to_string(),
                provider: Provider::OpenAiCompatible,
                model: "gpt".to_string(),
                chain: vec![ProviderDescriptor {
                    provider: Provider::OpenAiCompatible,
                    model: "gpt".to_string(),
                }],
            },
            cache: MemoryCache::default(),
        };

        let req = CompileRequest {
            source_text: "1+1".to_string(),
            source_id: "sample.js".to_string(),
            kind_hint: Some(SourceKind::JavaScript),
            language_hint: None,
            force_llm: true,
            provider_selection: ProviderSelection::OpenAiCompatible,
            model_override: None,
            no_cache: false,
        };

        let result = router.compile(&req).expect("compile should pass");
        assert_eq!(result.metadata.provider, Some(Provider::OpenAiCompatible));
    }

    #[test]
    fn cache_includes_provider_and_model_and_hits() {
        let temp = tempdir().expect("tempdir should work");
        let cache = FileCompileCache::new(PathBuf::from(temp.path()));

        let router = CompilerRouter {
            translator: MockTranslator {
                fail: false,
                response_js: "console.log('cached')".to_string(),
                provider: Provider::Ollama,
                model: "qwen".to_string(),
                chain: vec![ProviderDescriptor {
                    provider: Provider::Ollama,
                    model: "qwen".to_string(),
                }],
            },
            cache,
        };

        let req = pseudo_request();
        let first = router.compile(&req).expect("first compile should pass");
        assert!(!first.metadata.cache_hit);

        let second = router.compile(&req).expect("second compile should pass");
        assert!(second.metadata.cache_hit);
        assert_eq!(second.metadata.prompt_version, PROMPT_VERSION);
    }

    #[test]
    fn snapshot_error_for_llm_failure() {
        let router = CompilerRouter {
            translator: MockTranslator {
                fail: true,
                response_js: String::new(),
                provider: Provider::Ollama,
                model: "qwen".to_string(),
                chain: vec![ProviderDescriptor {
                    provider: Provider::Ollama,
                    model: "qwen".to_string(),
                }],
            },
            cache: MemoryCache::default(),
        };

        let err = router
            .compile(&pseudo_request())
            .expect_err("compile should fail");
        insta::assert_snapshot!(err.to_string(), @r"llm unavailable");
    }
}
