use anyhow::{Context, Result, anyhow};
use beeno_llm::{LlmClient, LlmTranslateRequest};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct OllamaClient {
    pub base_url: String,
    pub timeout: Duration,
}

impl OllamaClient {
    pub fn new(base_url: String) -> Result<Self> {
        Ok(Self {
            base_url,
            timeout: Duration::from_secs(2),
        })
    }

    pub fn is_reachable(&self) -> bool {
        let client = match Client::builder().timeout(self.timeout).build() {
            Ok(c) => c,
            Err(_) => return false,
        };

        let url = format!("{}/api/tags", self.base_url.trim_end_matches('/'));
        client
            .get(url)
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[derive(Debug, Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    response: String,
}

impl LlmClient for OllamaClient {
    fn translate_to_js(&self, req: &LlmTranslateRequest, model: &str) -> Result<String> {
        let prompt = build_prompt(req);
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        let url = format!("{}/api/generate", self.base_url.trim_end_matches('/'));
        let response = client
            .post(url)
            .json(&GenerateRequest {
                model,
                prompt: &prompt,
                stream: false,
            })
            .send()
            .context("failed calling Ollama")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_else(|_| "<unavailable>".to_string());
            return Err(anyhow!("Ollama request failed ({status}): {body}"));
        }

        let parsed: GenerateResponse = response
            .json()
            .context("failed to decode Ollama response")?;

        Ok(parsed.response)
    }
}

fn build_prompt(req: &LlmTranslateRequest) -> String {
    let hint = req
        .language_hint
        .as_ref()
        .map_or("unknown".to_string(), ToString::to_string);

    format!(
        "You are a strict transpiler. Return only runnable modern JavaScript (Node-style ESM), no prose.\\nSource id: {}\\nLanguage hint: {}\\nINPUT START\\n{}\\nINPUT END",
        req.source_id, hint, req.source_text
    )
}

#[cfg(test)]
mod tests {
    use super::OllamaClient;
    use beeno_llm::{LlmClient, LlmTranslateRequest};

    #[test]
    #[ignore]
    fn live_ollama_translate_if_enabled() {
        if std::env::var("BEENO_RUN_LIVE_TESTS").ok().as_deref() != Some("1") {
            return;
        }

        let base = std::env::var("BEENO_OLLAMA_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = std::env::var("BEENO_OLLAMA_MODEL")
            .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());

        let client = OllamaClient::new(base).expect("client should build");
        let req = LlmTranslateRequest {
            source_text: "write hello".to_string(),
            source_id: "live.pseudo".to_string(),
            language_hint: Some("pseudocode".to_string()),
        };

        let out = client
            .translate_to_js(&req, &model)
            .expect("ollama live request should succeed");
        assert!(!out.trim().is_empty());
    }
}
