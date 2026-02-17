use anyhow::{Context, Result, anyhow};
use beeno_llm::{LlmClient, LlmTranslateRequest};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    pub base_url: String,
    pub api_key: String,
}

impl OpenAiCompatibleClient {
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let api_key = std::env::var("OPENAI_API_KEY")
            .context("OPENAI_API_KEY is required for OpenAI-compatible provider")?;

        Ok(Self { base_url, api_key })
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    temperature: f32,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

impl LlmClient for OpenAiCompatibleClient {
    fn translate_to_js(&self, req: &LlmTranslateRequest, model: &str) -> Result<String> {
        let prompt = build_prompt(req);
        let body = ChatRequest {
            model: model.to_string(),
            temperature: 0.0,
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You convert arbitrary source text into executable JavaScript. Return code only."
                        .to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt,
                },
            ],
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .context("failed to build HTTP client")?;
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let response = client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .context("failed calling OpenAI-compatible endpoint")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_else(|_| "<unavailable>".to_string());
            return Err(anyhow!(
                "OpenAI-compatible request failed ({status}): {body}"
            ));
        }

        let parsed: ChatResponse = response
            .json()
            .context("failed to decode OpenAI-compatible response")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("OpenAI-compatible response had no choices"))?;

        Ok(content)
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
    use super::OpenAiCompatibleClient;
    use beeno_llm::{LlmClient, LlmTranslateRequest};

    #[test]
    #[ignore]
    fn live_openai_translate_if_enabled() {
        if std::env::var("BEENO_RUN_LIVE_TESTS").ok().as_deref() != Some("1") {
            return;
        }

        let client = match OpenAiCompatibleClient::from_env() {
            Ok(c) => c,
            Err(_) => return,
        };

        let model = std::env::var("BEENO_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());
        let req = LlmTranslateRequest {
            source_text: "write hello".to_string(),
            source_id: "live.pseudo".to_string(),
            language_hint: Some("pseudocode".to_string()),
        };

        let out = client
            .translate_to_js(&req, &model)
            .expect("openai live request should succeed");
        assert!(!out.trim().is_empty());
    }
}
