//! Talking to whichever language model the user configured.
//!
//! This is the only part of Summo that sends anything off the machine, and it does so only for
//! summarisation, translation and question answering — never audio. The endpoint is the user's
//! choice, and the presets exist because "paste your base URL" is a bad first-run experience.
//!
//! Everything speaks the OpenAI chat-completions shape, including Ollama and LM Studio, so one
//! client covers local and hosted models alike. Anthropic's native API differs enough to need its
//! own request shape, which is why [`Provider::wire`] exists rather than a single hardcoded format.

use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

/// Which request/response shape an endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Wire {
    /// `POST /chat/completions` with `{model, messages, stream}`. Ollama, LM Studio, vLLM,
    /// llama.cpp, OpenAI, Groq, OpenRouter and most others.
    OpenAi,
    /// `POST /v1/messages`, with the system prompt as a top-level field.
    Anthropic,
}

/// A configured endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub wire: Wire,
    /// Read from the OS keychain at runtime; never persisted alongside this struct.
    #[serde(skip)]
    pub api_key: Option<String>,
    /// Ceiling on generated tokens, so a runaway model cannot bill without limit.
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Provider {
    /// A local Ollama server. The default suggestion: nothing leaves the machine.
    #[must_use]
    pub fn ollama(model: &str) -> Self {
        Self {
            name: "Ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: model.into(),
            wire: Wire::OpenAi,
            api_key: None,
            max_tokens: 2048,
            temperature: 0.2,
        }
    }

    #[must_use]
    pub fn lm_studio(model: &str) -> Self {
        Self {
            name: "LM Studio".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            ..Self::ollama(model)
        }
    }

    #[must_use]
    pub fn openai(model: &str, api_key: &str) -> Self {
        Self {
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some(api_key.into()),
            ..Self::ollama(model)
        }
    }

    #[must_use]
    pub fn anthropic(model: &str, api_key: &str) -> Self {
        Self {
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            model: model.into(),
            wire: Wire::Anthropic,
            api_key: Some(api_key.into()),
            max_tokens: 2048,
            temperature: 0.2,
        }
    }

    /// Point at any OpenAI-compatible server.
    #[must_use]
    pub fn custom(name: &str, base_url: &str, model: &str) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.trim_end_matches('/').into(),
            model: model.into(),
            wire: Wire::OpenAi,
            api_key: None,
            max_tokens: 2048,
            temperature: 0.2,
        }
    }

    /// Whether this endpoint keeps data on the user's machine.
    ///
    /// Surfaced in the UI next to the endpoint: the whole product promise is that recordings stay
    /// local, and a user pointing summaries at a hosted model should see that they have opted into
    /// sending transcript text away.
    #[must_use]
    pub fn is_local(&self) -> bool {
        let host = self
            .base_url
            .split("//")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("");
        host.starts_with("127.0.0.1")
            || host.starts_with("localhost")
            || host.starts_with("[::1]")
            || host.starts_with("0.0.0.0")
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.wire {
            Wire::OpenAi => format!("{base}/chat/completions"),
            Wire::Anthropic => format!("{base}/messages"),
        }
    }
}

/// One turn in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Message {
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

/// Sends requests to a configured provider.
pub struct LlmClient {
    provider: Provider,
    http: reqwest::Client,
}

impl LlmClient {
    pub fn new(provider: Provider) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("summo/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // Generous: a local model on a laptop can take a while to produce a long summary, and
            // cutting it off mid-answer is worse than waiting.
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| Error::Other(format!("cannot build http client: {e}")))?;
        Ok(Self { provider, http })
    }

    #[must_use]
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    fn body(&self, messages: &[Message], stream: bool) -> serde_json::Value {
        match self.provider.wire {
            Wire::OpenAi => serde_json::json!({
                "model": self.provider.model,
                "messages": messages,
                "max_tokens": self.provider.max_tokens,
                "temperature": self.provider.temperature,
                "stream": stream,
            }),
            Wire::Anthropic => {
                // Anthropic takes the system prompt as a top-level field rather than a message.
                let system: String = messages
                    .iter()
                    .filter(|m| m.role == Role::System)
                    .map(|m| m.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let turns: Vec<&Message> =
                    messages.iter().filter(|m| m.role != Role::System).collect();
                serde_json::json!({
                    "model": self.provider.model,
                    "system": system,
                    "messages": turns,
                    "max_tokens": self.provider.max_tokens,
                    "temperature": self.provider.temperature,
                    "stream": stream,
                })
            }
        }
    }

    fn authorize(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let Some(key) = &self.provider.api_key else {
            return req;
        };
        match self.provider.wire {
            Wire::OpenAi => req.bearer_auth(key),
            Wire::Anthropic => req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
        }
    }

    /// Send a request and wait for the whole answer.
    pub async fn complete(&self, messages: &[Message]) -> Result<String> {
        let req = self
            .http
            .post(self.provider.endpoint())
            .json(&self.body(messages, false));

        let resp = self
            .authorize(req)
            .send()
            .await
            .map_err(|e| Error::Other(format!("{}: {e}", self.provider.name)))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Other(format!("{}: {e}", self.provider.name)))?;

        if !status.is_success() {
            // The provider's own error message is far more useful than the status code alone —
            // "model not found" and "insufficient quota" both arrive as a 4xx.
            return Err(Error::Other(format!(
                "{} returned {status}: {}",
                self.provider.name,
                body.chars().take(400).collect::<String>()
            )));
        }

        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            Error::Other(format!("{} sent malformed json: {e}", self.provider.name))
        })?;
        extract_text(&json, self.provider.wire).ok_or_else(|| {
            Error::Other(format!(
                "{} sent no completion: {}",
                self.provider.name,
                body.chars().take(200).collect::<String>()
            ))
        })
    }

    /// Send a request and deliver text as it arrives.
    ///
    /// Streaming matters for translation, where a sentence appearing progressively feels live,
    /// and for long summaries, where waiting for the whole thing feels broken.
    pub async fn stream<F>(&self, messages: &[Message], mut on_chunk: F) -> Result<String>
    where
        F: FnMut(&str) + Send,
    {
        let req = self
            .http
            .post(self.provider.endpoint())
            .json(&self.body(messages, true));

        let resp = self
            .authorize(req)
            .send()
            .await
            .map_err(|e| Error::Other(format!("{}: {e}", self.provider.name)))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Other(format!(
                "{} returned {status}: {}",
                self.provider.name,
                body.chars().take(400).collect::<String>()
            )));
        }

        let mut full = String::new();
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Other(format!("stream failed: {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Server-sent events are newline-delimited; a chunk can split an event in half, so
            // only complete lines are consumed and the remainder stays buffered.
            while let Some(newline) = buffer.find('\n') {
                let line = buffer[..newline].trim().to_string();
                buffer.drain(..=newline);

                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(full);
                }
                let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };
                if let Some(delta) = extract_delta(&json, self.provider.wire) {
                    on_chunk(&delta);
                    full.push_str(&delta);
                }
            }
        }
        Ok(full)
    }

    /// Check that the endpoint answers, for the settings screen's "test connection" button.
    pub async fn health_check(&self) -> Result<String> {
        self.complete(&[Message::user("Reply with the single word: ok")])
            .await
    }
}

/// Pull the assistant text out of a non-streaming response.
fn extract_text(json: &serde_json::Value, wire: Wire) -> Option<String> {
    match wire {
        Wire::OpenAi => json["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string),
        Wire::Anthropic => {
            let blocks = json["content"].as_array()?;
            let text: String = blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        }
    }
}

/// Pull the incremental text out of one streamed event.
fn extract_delta(json: &serde_json::Value, wire: Wire) -> Option<String> {
    match wire {
        Wire::OpenAi => json["choices"][0]["delta"]["content"]
            .as_str()
            .map(str::to_string),
        Wire::Anthropic => json["delta"]["text"].as_str().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_endpoints_are_recognised() {
        assert!(Provider::ollama("qwen3").is_local());
        assert!(Provider::lm_studio("qwen3").is_local());
        assert!(Provider::custom("x", "http://localhost:8080/v1", "m").is_local());
        assert!(!Provider::openai("gpt-5", "sk-x").is_local());
        assert!(!Provider::anthropic("claude-opus-5", "sk-x").is_local());
        assert!(
            !Provider::custom("x", "https://api.example.com/v1", "m").is_local(),
            "a remote host must not be reported as local"
        );
    }

    #[test]
    fn endpoints_are_built_per_wire_format() {
        assert_eq!(
            LlmClient::new(Provider::ollama("q"))
                .unwrap()
                .provider
                .endpoint(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        assert_eq!(
            LlmClient::new(Provider::anthropic("c", "k"))
                .unwrap()
                .provider
                .endpoint(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn trailing_slashes_do_not_double_up() {
        let p = Provider::custom("x", "http://127.0.0.1:8080/v1/", "m");
        assert_eq!(p.endpoint(), "http://127.0.0.1:8080/v1/chat/completions");
    }

    #[test]
    fn anthropic_lifts_the_system_prompt_out_of_the_messages() {
        let client = LlmClient::new(Provider::anthropic("claude", "k")).unwrap();
        let body = client.body(
            &[Message::system("be terse"), Message::user("hello")],
            false,
        );

        assert_eq!(body["system"], "be terse");
        let turns = body["messages"].as_array().unwrap();
        assert_eq!(
            turns.len(),
            1,
            "the system turn must not also appear in messages"
        );
        assert_eq!(turns[0]["role"], "user");
    }

    #[test]
    fn openai_keeps_the_system_prompt_as_a_message() {
        let client = LlmClient::new(Provider::ollama("q")).unwrap();
        let body = client.body(
            &[Message::system("be terse"), Message::user("hello")],
            false,
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert!(body["system"].is_null());
    }

    #[test]
    fn api_keys_are_not_serialized_with_settings() {
        // Keys live in the OS keychain. A settings file that quietly contained one would be a
        // credential leak into backups, sync and support bundles.
        let json = serde_json::to_string(&Provider::openai("gpt-5", "sk-secret")).unwrap();
        assert!(
            !json.contains("sk-secret"),
            "api key leaked into settings: {json}"
        );
    }

    #[test]
    fn responses_are_parsed_for_both_wire_formats() {
        let openai = serde_json::json!({
            "choices": [{"message": {"content": "xin chào"}}]
        });
        assert_eq!(extract_text(&openai, Wire::OpenAi).unwrap(), "xin chào");

        let anthropic = serde_json::json!({
            "content": [{"type": "text", "text": "xin "}, {"type": "text", "text": "chào"}]
        });
        assert_eq!(
            extract_text(&anthropic, Wire::Anthropic).unwrap(),
            "xin chào"
        );
    }

    #[test]
    fn stream_deltas_are_parsed_for_both_wire_formats() {
        let openai = serde_json::json!({"choices": [{"delta": {"content": "một"}}]});
        assert_eq!(extract_delta(&openai, Wire::OpenAi).unwrap(), "một");

        let anthropic = serde_json::json!({"delta": {"text": "hai"}});
        assert_eq!(extract_delta(&anthropic, Wire::Anthropic).unwrap(), "hai");
    }

    #[test]
    fn an_empty_response_is_not_silently_treated_as_success() {
        let empty = serde_json::json!({"choices": []});
        assert!(extract_text(&empty, Wire::OpenAi).is_none());
        assert!(extract_text(&serde_json::json!({"content": []}), Wire::Anthropic).is_none());
    }
}
