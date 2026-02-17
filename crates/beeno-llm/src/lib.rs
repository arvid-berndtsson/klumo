use anyhow::{Result, anyhow};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Ollama,
    OpenAiCompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSelection {
    Auto,
    Ollama,
    OpenAiCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider: Provider,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmTranslateRequest {
    pub source_text: String,
    pub source_id: String,
    pub language_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmTranslateResponse {
    pub javascript: String,
    pub provider: Provider,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAttempt {
    pub provider: Provider,
    pub stage: &'static str,
    pub error: String,
}

#[derive(Debug, Error)]
#[error("LLM routing failed after {attempts:?}")]
pub struct ProviderRoutingError {
    pub attempts: Vec<ProviderAttempt>,
}

pub trait LlmClient {
    fn translate_to_js(&self, req: &LlmTranslateRequest, model: &str) -> Result<String>;
}

pub trait ReachabilityProbe {
    fn ollama_reachable(&self) -> bool;
}

pub trait TranslationService {
    fn candidate_chain(&self, selection: ProviderSelection) -> Vec<ProviderDescriptor>;
    fn translate(
        &self,
        selection: ProviderSelection,
        req: &LlmTranslateRequest,
        model_override: Option<&str>,
    ) -> Result<LlmTranslateResponse>;
}

pub fn normalize_js_output(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("LLM returned empty output"));
    }

    if let Some(block) = extract_fenced_code(trimmed) {
        if block.trim().is_empty() {
            return Err(anyhow!("LLM returned empty fenced output"));
        }
        return Ok(block.trim().to_string());
    }

    Ok(trimmed.to_string())
}

fn extract_fenced_code(input: &str) -> Option<String> {
    let start = input.find("```")?;
    let remainder = &input[start + 3..];
    let body_start = remainder.find('\n')? + 1;
    let body = &remainder[body_start..];
    let end = body.find("```")?;
    Some(body[..end].to_string())
}

pub struct ProviderRouter<O, P, R>
where
    O: LlmClient,
    P: LlmClient,
    R: ReachabilityProbe,
{
    pub ollama: O,
    pub openai: P,
    pub reachability: R,
    pub ollama_model: String,
    pub openai_model: String,
}

impl<O, P, R> ProviderRouter<O, P, R>
where
    O: LlmClient,
    P: LlmClient,
    R: ReachabilityProbe,
{
    fn call_provider(
        &self,
        provider: Provider,
        req: &LlmTranslateRequest,
        model_override: Option<&str>,
    ) -> Result<LlmTranslateResponse> {
        match provider {
            Provider::Ollama => {
                let model = model_override.unwrap_or(&self.ollama_model);
                let output = self.ollama.translate_to_js(req, model)?;
                Ok(LlmTranslateResponse {
                    javascript: normalize_js_output(&output)?,
                    provider,
                    model: model.to_string(),
                })
            }
            Provider::OpenAiCompatible => {
                let model = model_override.unwrap_or(&self.openai_model);
                let output = self.openai.translate_to_js(req, model)?;
                Ok(LlmTranslateResponse {
                    javascript: normalize_js_output(&output)?,
                    provider,
                    model: model.to_string(),
                })
            }
        }
    }
}

impl<O, P, R> TranslationService for ProviderRouter<O, P, R>
where
    O: LlmClient,
    P: LlmClient,
    R: ReachabilityProbe,
{
    fn candidate_chain(&self, selection: ProviderSelection) -> Vec<ProviderDescriptor> {
        match selection {
            ProviderSelection::Ollama => vec![ProviderDescriptor {
                provider: Provider::Ollama,
                model: self.ollama_model.clone(),
            }],
            ProviderSelection::OpenAiCompatible => vec![ProviderDescriptor {
                provider: Provider::OpenAiCompatible,
                model: self.openai_model.clone(),
            }],
            ProviderSelection::Auto => {
                if self.reachability.ollama_reachable() {
                    vec![
                        ProviderDescriptor {
                            provider: Provider::Ollama,
                            model: self.ollama_model.clone(),
                        },
                        ProviderDescriptor {
                            provider: Provider::OpenAiCompatible,
                            model: self.openai_model.clone(),
                        },
                    ]
                } else {
                    vec![ProviderDescriptor {
                        provider: Provider::OpenAiCompatible,
                        model: self.openai_model.clone(),
                    }]
                }
            }
        }
    }

    fn translate(
        &self,
        selection: ProviderSelection,
        req: &LlmTranslateRequest,
        model_override: Option<&str>,
    ) -> Result<LlmTranslateResponse> {
        let chain = self.candidate_chain(selection);
        let mut attempts = Vec::new();

        for entry in chain {
            match self.call_provider(entry.provider, req, model_override) {
                Ok(response) => return Ok(response),
                Err(err) => attempts.push(ProviderAttempt {
                    provider: entry.provider,
                    stage: "translate",
                    error: err.to_string(),
                }),
            }
        }

        Err(ProviderRoutingError { attempts }.into())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LlmClient, LlmTranslateRequest, Provider, ProviderDescriptor, ProviderRouter,
        ProviderSelection, ReachabilityProbe, TranslationService, normalize_js_output,
    };
    use anyhow::{Result, anyhow};

    struct StubClient {
        fail: bool,
        output: String,
    }

    impl LlmClient for StubClient {
        fn translate_to_js(&self, _req: &LlmTranslateRequest, _model: &str) -> Result<String> {
            if self.fail {
                return Err(anyhow!("stub failure"));
            }
            Ok(self.output.clone())
        }
    }

    struct Probe(bool);

    impl ReachabilityProbe for Probe {
        fn ollama_reachable(&self) -> bool {
            self.0
        }
    }

    fn req() -> LlmTranslateRequest {
        LlmTranslateRequest {
            source_text: "write 1".to_string(),
            source_id: "sample.pseudo".to_string(),
            language_hint: Some("pseudo".to_string()),
        }
    }

    #[test]
    fn strips_fence() {
        let out = normalize_js_output("```js\n1+1\n```").expect("normalize should pass");
        assert_eq!(out, "1+1");
    }

    #[test]
    fn rejects_empty() {
        let err = normalize_js_output("  ").expect_err("must fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn auto_prefers_ollama_when_reachable() {
        let router = ProviderRouter {
            ollama: StubClient {
                fail: false,
                output: "1".to_string(),
            },
            openai: StubClient {
                fail: false,
                output: "2".to_string(),
            },
            reachability: Probe(true),
            ollama_model: "ollama-model".to_string(),
            openai_model: "openai-model".to_string(),
        };

        let chain = router.candidate_chain(ProviderSelection::Auto);
        assert_eq!(chain[0].provider, Provider::Ollama);
    }

    #[test]
    fn auto_falls_back_to_openai_on_ollama_failure() {
        let router = ProviderRouter {
            ollama: StubClient {
                fail: true,
                output: String::new(),
            },
            openai: StubClient {
                fail: false,
                output: "3".to_string(),
            },
            reachability: Probe(true),
            ollama_model: "ollama-model".to_string(),
            openai_model: "openai-model".to_string(),
        };

        let response = router
            .translate(ProviderSelection::Auto, &req(), None)
            .expect("fallback should work");
        assert_eq!(response.provider, Provider::OpenAiCompatible);
        assert_eq!(response.javascript, "3");
    }

    #[test]
    fn explicit_provider_bypasses_auto() {
        let router = ProviderRouter {
            ollama: StubClient {
                fail: false,
                output: "1".to_string(),
            },
            openai: StubClient {
                fail: false,
                output: "9".to_string(),
            },
            reachability: Probe(true),
            ollama_model: "ollama-model".to_string(),
            openai_model: "openai-model".to_string(),
        };

        let chain = router.candidate_chain(ProviderSelection::OpenAiCompatible);
        assert_eq!(
            chain,
            vec![ProviderDescriptor {
                provider: Provider::OpenAiCompatible,
                model: "openai-model".to_string()
            }]
        );

        let response = router
            .translate(ProviderSelection::OpenAiCompatible, &req(), None)
            .expect("openai path should work");
        assert_eq!(response.provider, Provider::OpenAiCompatible);
        assert_eq!(response.javascript, "9");
    }
}
