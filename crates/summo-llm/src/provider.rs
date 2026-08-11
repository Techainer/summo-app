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

/// A known endpoint, and everything the UI needs to describe it without a second list.
///
/// This catalogue used to exist twice — as a `match` here and as an array in the settings screen —
/// and the two had already drifted: the daemon accepted four names and the picker offered the same
/// four, but only this side knew which needed a key, and neither knew the environment variable a
/// user would already have set. Serving the catalogue means adding a provider is one edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    /// Stable identifier. What lands in `settings.json`.
    pub id: &'static str,
    /// What a person calls it.
    pub name: &'static str,
    pub base_url: &'static str,
    /// A reasonable starting model. Always editable — provider model names change constantly and a
    /// stale default here should be a nuisance, not a wall.
    pub model: &'static str,
    pub wire: Wire,
    /// The variable this provider's own tooling uses, so a machine that already talks to it needs
    /// no new configuration. `None` means the endpoint takes no key.
    pub key_env: Option<&'static str>,
    /// The variable that redirects this provider at a gateway, where one is conventional.
    ///
    /// Corporate installs almost never talk to `api.openai.com` directly — they run a proxy and
    /// export `OPENAI_BASE_URL` beside the key. Ignoring it meant the key resolved, the request
    /// went to the wrong host, and the 401 that came back looked like a bad key.
    pub url_env: Option<&'static str>,
    /// Whether text stays on this machine. The one fact the product promise turns on.
    pub local: bool,
}

/// Every endpoint Summo knows by name.
///
/// Local first, because that is the recommendation and the default. Everything except Anthropic
/// speaks the OpenAI chat-completions shape, including Google's compatibility endpoint, so adding
/// most of these costs a row rather than a code path.
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "ollama",
        name: "Ollama",
        base_url: "http://127.0.0.1:11434/v1",
        model: "qwen3:8b",
        wire: Wire::OpenAi,
        key_env: None,
        // Deliberately not `OLLAMA_HOST`: it would let an environment variable point the "runs on
        // your machine" preset at someone else's server while the interface still said local.
        // Somebody using a remote Ollama can say so with a base URL, and see it labelled honestly.
        url_env: None,
        local: true,
    },
    Preset {
        id: "lm-studio",
        name: "LM Studio",
        base_url: "http://127.0.0.1:1234/v1",
        model: "local-model",
        wire: Wire::OpenAi,
        key_env: None,
        url_env: None,
        local: true,
    },
    Preset {
        id: "llama-cpp",
        name: "llama.cpp",
        base_url: "http://127.0.0.1:8080/v1",
        model: "local-model",
        wire: Wire::OpenAi,
        key_env: None,
        url_env: None,
        local: true,
    },
    Preset {
        id: "vllm",
        name: "vLLM",
        base_url: "http://127.0.0.1:8000/v1",
        model: "local-model",
        wire: Wire::OpenAi,
        key_env: None,
        url_env: None,
        local: true,
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        model: "gpt-5",
        wire: Wire::OpenAi,
        key_env: Some("OPENAI_API_KEY"),
        url_env: Some("OPENAI_BASE_URL"),
        local: false,
    },
    Preset {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        model: "claude-opus-5",
        wire: Wire::Anthropic,
        key_env: Some("ANTHROPIC_API_KEY"),
        url_env: Some("ANTHROPIC_BASE_URL"),
        local: false,
    },
    Preset {
        // Google's OpenAI-compatibility endpoint rather than `generateContent`: same answers, and
        // no third request shape to maintain.
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        model: "gemini-2.5-pro",
        wire: Wire::OpenAi,
        key_env: Some("GEMINI_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
        wire: Wire::OpenAi,
        key_env: Some("DEEPSEEK_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        model: "llama-3.3-70b-versatile",
        wire: Wire::OpenAi,
        key_env: Some("GROQ_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        model: "anthropic/claude-sonnet-4.5",
        wire: Wire::OpenAi,
        key_env: Some("OPENROUTER_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        model: "mistral-large-latest",
        wire: Wire::OpenAi,
        key_env: Some("MISTRAL_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "together",
        name: "Together",
        base_url: "https://api.together.xyz/v1",
        model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        wire: Wire::OpenAi,
        key_env: Some("TOGETHER_API_KEY"),
        url_env: None,
        local: false,
    },
    Preset {
        id: "xai",
        name: "xAI",
        base_url: "https://api.x.ai/v1",
        model: "grok-4",
        wire: Wire::OpenAi,
        key_env: Some("XAI_API_KEY"),
        url_env: None,
        local: false,
    },
];

/// Look up a preset by id.
#[must_use]
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The key for a provider, from the environment.
///
/// `SUMMO_API_KEY` first so one variable can override everything, then the provider's own — a
/// machine that already runs `ollama` and `claude` has `ANTHROPIC_API_KEY` set, and asking the user
/// to copy it into a Summo-specific name is make-work. Returns `None` for an endpoint that takes no
/// key, so a local server is never handed a stray credential.
#[must_use]
pub fn key_from_env(id: &str) -> Option<String> {
    // A bare base URL is not a preset, but it is almost always an OpenAI-compatible gateway put
    // there by an organisation that also exported `OPENAI_API_KEY`. Sending nothing meant that
    // setup — the most common corporate one there is — failed with "no api key passed in".
    let env = match preset(id) {
        Some(preset) => preset.key_env?,
        None if id.starts_with("http") => "OPENAI_API_KEY",
        None => return None,
    };
    ["SUMMO_API_KEY", env]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// A gateway to use instead of a preset's own address, if the environment names one.
///
/// `SUMMO_BASE_URL` overrides everything; otherwise the provider's own convention. Only presets
/// that genuinely have one carry a `url_env` — inventing variables would mean a name nobody sets
/// and a redirect nobody expects.
#[must_use]
pub fn base_url_from_env(id: &str) -> Option<String> {
    let candidates = [Some("SUMMO_BASE_URL"), preset(id).and_then(|p| p.url_env)];
    candidates
        .into_iter()
        .flatten()
        .find_map(|name| std::env::var(name).ok())
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| url.starts_with("http"))
}

impl Provider {
    /// Build from a preset.
    #[must_use]
    pub fn from_preset(preset: &Preset, model: Option<&str>, api_key: Option<&str>) -> Self {
        Self {
            name: preset.name.into(),
            base_url: preset.base_url.into(),
            model: model
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .unwrap_or(preset.model)
                .into(),
            wire: preset.wire,
            api_key: api_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string),
            max_tokens: 2048,
            temperature: 0.2,
        }
    }

    /// A local Ollama server. The default suggestion: nothing leaves the machine.
    #[must_use]
    pub fn ollama(model: &str) -> Self {
        Self::from_preset(preset("ollama").expect("ollama is a preset"), Some(model), None)
    }

    #[must_use]
    pub fn lm_studio(model: &str) -> Self {
        Self::from_preset(
            preset("lm-studio").expect("lm-studio is a preset"),
            Some(model),
            None,
        )
    }

    #[must_use]
    pub fn openai(model: &str, api_key: &str) -> Self {
        Self::from_preset(
            preset("openai").expect("openai is a preset"),
            Some(model),
            Some(api_key),
        )
    }

    #[must_use]
    pub fn anthropic(model: &str, api_key: &str) -> Self {
        Self::from_preset(
            preset("anthropic").expect("anthropic is a preset"),
            Some(model),
            Some(api_key),
        )
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
    /// Build a provider from what a user configured: a well-known name, or a base URL.
    ///
    /// One copy of this, shared by the CLI, the daemon and the settings screen. The alternative was
    /// three lists of default model names that would agree until one of them was updated.
    ///
    /// The key is never stored — it is read from the environment or the keychain at the moment it
    /// is needed, so it cannot end up in a settings file, a backup or a support bundle.
    pub fn resolve(name: &str, model: Option<&str>, api_key: Option<&str>) -> Result<Self> {
        let name = match name.trim() {
            "" => "ollama",
            other => other,
        };

        if let Some(preset) = preset(name) {
            // The caller's key wins; otherwise the provider's own environment variable, which a
            // machine that already talks to it will have set.
            let key = api_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .or_else(|| key_from_env(name));

            if preset.key_env.is_some() && key.is_none() {
                let env = preset.key_env.unwrap_or("SUMMO_API_KEY");
                return Err(Error::Config(format!(
                    "{} needs an API key: set {env} or SUMMO_API_KEY",
                    preset.name
                )));
            }
            let mut provider = Self::from_preset(preset, model, key.as_deref());
            if let Some(url) = base_url_from_env(name) {
                provider.base_url = url;
            }
            return Ok(provider);
        }

        if name.starts_with("http://") || name.starts_with("https://") {
            let mut provider = Self::custom("custom", name, model.unwrap_or("default"));
            // A self-hosted gateway can still want a key; an empty one stays `None` so a local
            // server is never sent an `Authorization` header it did not ask for.
            provider.api_key = api_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .or_else(|| key_from_env(name))
                .filter(|k| !k.trim().is_empty());
            return Ok(provider);
        }

        let known: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        Err(Error::Config(format!(
            "unknown provider `{name}`. Use one of {}, or a base URL.",
            known.join(", ")
        )))
    }

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

    // ---- the catalogue -------------------------------------------------------------------

    /// Ids reach `settings.json` and the query string of a settings save; a duplicate would make
    /// one of them unreachable in a way nothing else would notice.
    #[test]
    fn preset_ids_are_unique() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate preset id");
    }

    /// A gateway variable on a local preset would let the environment point "runs on your machine"
    /// at someone else's server while the interface kept saying local.
    #[test]
    fn a_local_preset_cannot_be_redirected_by_the_environment() {
        for preset in PRESETS.iter().filter(|p| p.local) {
            assert!(preset.url_env.is_none(), "{} can be redirected", preset.id);
        }
    }

    /// `local` is what the interface shows next to the endpoint, and the product promise turns on
    /// it. Deriving it from the URL rather than trusting the flag stops the two disagreeing.
    #[test]
    fn every_preset_agrees_with_its_own_url_about_being_local() {
        for preset in PRESETS {
            let built = Provider::from_preset(preset, None, None);
            assert_eq!(
                built.is_local(),
                preset.local,
                "{} claims local={} but its URL says otherwise",
                preset.id,
                preset.local
            );
        }
    }

    /// A local server must never be handed a credential, whatever happens to be in the
    /// environment: it is somebody else's process listening on a loopback port.
    #[test]
    fn a_local_preset_declares_no_key_variable() {
        for preset in PRESETS.iter().filter(|p| p.local) {
            assert!(preset.key_env.is_none(), "{} wants a key", preset.id);
        }
    }

    #[test]
    fn every_hosted_preset_names_the_variable_its_own_tooling_uses() {
        for preset in PRESETS.iter().filter(|p| !p.local) {
            assert!(preset.key_env.is_some(), "{} names no key variable", preset.id);
        }
    }

    #[test]
    fn resolving_a_preset_uses_its_url_and_default_model() {
        let groq = Provider::resolve("groq", None, Some("k")).unwrap();
        assert_eq!(groq.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(groq.model, "llama-3.3-70b-versatile");
        assert_eq!(groq.wire, Wire::OpenAi);
    }

    #[test]
    fn a_chosen_model_overrides_the_default() {
        let p = Provider::resolve("deepseek", Some("deepseek-reasoner"), Some("k")).unwrap();
        assert_eq!(p.model, "deepseek-reasoner");
    }

    /// An empty string is not a model. Sending one asks the provider for a model named "", and the
    /// error that comes back names neither Summo nor the setting that caused it.
    #[test]
    fn a_blank_model_falls_back_to_the_default() {
        let p = Provider::resolve("openai", Some("   "), Some("k")).unwrap();
        assert_eq!(p.model, "gpt-5");
    }

    #[test]
    fn an_unknown_name_lists_what_is_known() {
        let err = Provider::resolve("kimi", None, None).unwrap_err().to_string();
        assert!(err.contains("ollama"), "{err}");
        assert!(err.contains("openrouter"), "{err}");
    }

    #[test]
    fn a_base_url_is_still_accepted() {
        let p = Provider::resolve("https://gateway.internal/v1", Some("m"), None).unwrap();
        assert_eq!(p.base_url, "https://gateway.internal/v1");
    }

    /// The message has to name the variable the user should set. "needs an API key" without it
    /// sends people to the documentation for a one-word answer.
    #[test]
    fn a_missing_key_names_the_variable_to_set() {
        let err = Provider::resolve("gemini", None, None).unwrap_err().to_string();
        assert!(err.contains("GEMINI_API_KEY"), "{err}");
    }

    #[test]
    fn a_local_endpoint_needs_no_key() {
        assert!(Provider::resolve("ollama", None, None).is_ok());
        assert!(Provider::resolve("llama-cpp", None, None).is_ok());
    }

    #[test]
    fn an_empty_name_still_means_ollama() {
        assert_eq!(Provider::resolve("", None, None).unwrap().name, "Ollama");
    }
}
