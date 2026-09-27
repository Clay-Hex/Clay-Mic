use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_llm_enabled")]
    pub enabled: bool,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub format_prompt: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    #[serde(default)]
    pub models: HashMap<String, String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_llm_enabled() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    120
}

fn default_max_tokens() -> u32 {
    8192
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "openai".to_string(),
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            format_prompt: include_str!("../../prompts/format-prompt.md").trim().to_string(),
            reasoning: String::new(),
            api_keys: HashMap::new(),
            models: HashMap::new(),
            timeout_secs: default_timeout_secs(),
            max_tokens: default_max_tokens(),
        }
    }
}

/// Process-wide HTTP client. Cloning a `reqwest::Client` shares the same
/// connection pool, so reusing one client keeps TLS sessions alive instead of
/// paying a fresh handshake on every formatting request.
fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// Stable opaque id, sent to OpenCode Go so it can route/cache consistently.
fn session_id() -> String {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION
        .get_or_init(|| uuid::Uuid::new_v4().to_string())
        .clone()
}

fn request_headers(provider: &str, api_key: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {api_key}")) {
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    if provider == "opencode-go" {
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static("clay-mic/0.1.0"),
        );
        headers.insert(
            HeaderName::from_static("x-opencode-client"),
            HeaderValue::from_static("clay-mic"),
        );
        if let Ok(value) = HeaderValue::from_str(&session_id()) {
            headers.insert(HeaderName::from_static("x-opencode-session"), value);
        }
    }
    headers
}

enum ThinkingFamily {
    /// Toggle via `{"thinking": {"type": "..."}}` (DeepSeek / GLM / Kimi).
    Toggle,
    /// Toggle via `enable_thinking` (Qwen).
    Qwen,
    /// Only `reasoning_effort` is available (OpenAI o-series / gpt-5).
    OpenAi,
}

/// Effort levels in ascending strength; used to snap the chosen level onto the
/// set a specific model accepts.
const EFFORT_ORDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

fn clamp_effort<'a>(level: &'a str, accepted: Option<&'a [String]>) -> &'a str {
    let Some(accepted) = accepted else {
        return level;
    };
    if accepted.iter().any(|value| value == level) {
        return level;
    }
    let Some(position) = EFFORT_ORDER.iter().position(|value| *value == level) else {
        return level;
    };
    let mut best: Option<(usize, usize, &str)> = None;
    for value in accepted {
        let Some(index) = EFFORT_ORDER
            .iter()
            .position(|candidate| *candidate == value.as_str())
        else {
            continue;
        };
        let distance = index.abs_diff(position);
        // On a tie prefer the stronger level: the user asked for thinking, so
        // more of it beats silently sending none.
        let better = match best {
            None => true,
            Some((best_distance, best_index, _)) => {
                distance < best_distance || (distance == best_distance && index > best_index)
            }
        };
        if better {
            best = Some((distance, index, value.as_str()));
        }
    }
    best.map(|(_, _, value)| value).unwrap_or(level)
}

fn thinking_family(model: &str) -> Option<ThinkingFamily> {
    let name = model.to_lowercase();
    if name.contains("qwen") {
        return Some(ThinkingFamily::Qwen);
    }
    if name.contains("deepseek")
        || name.contains("glm")
        || name.contains("kimi")
        || name.contains("moonshot")
    {
        return Some(ThinkingFamily::Toggle);
    }
    if name.contains("gpt-5")
        || name.contains("gpt-oss")
        || name.starts_with("o1")
        || name.starts_with("o3")
        || name.starts_with("o4")
    {
        return Some(ThinkingFamily::OpenAi);
    }
    None
}

/// Write explicit thinking parameters instead of relying on provider defaults.
fn apply_thinking(body: &mut serde_json::Value, provider: &str, model: &str, reasoning: &str) {
    let mut level = reasoning.trim();
    let mut disabled = level.is_empty();
    // The curated table beats the name heuristic: it knows whether the model
    // reasons and which effort values it accepts. Ids it never heard of keep
    // the behaviour below exactly as it was.
    let mut snapped = false;
    let caps = crate::model_caps::lookup(provider, model);
    if let Some(entry) = &caps {
        if !entry.reasoning {
            disabled = true;
        } else if !disabled {
            level = clamp_effort(level, entry.effort.as_deref());
            snapped = entry.effort.is_some();
        }
    }
    match thinking_family(model) {
        Some(ThinkingFamily::Qwen) => {
            let enabled = !disabled;
            body["enable_thinking"] = serde_json::json!(enabled);
            body["chat_template_kwargs"] = serde_json::json!({ "enable_thinking": enabled });
        }
        Some(ThinkingFamily::Toggle) => {
            body["thinking"] = serde_json::json!({
                "type": if disabled { "disabled" } else { "enabled" }
            });
            if !disabled {
                let effort = if snapped {
                    level
                } else if model.to_lowercase().contains("deepseek") {
                    match level {
                        "low" => "low",
                        "high" => "max",
                        _ => "high",
                    }
                } else {
                    level
                };
                body["reasoning_effort"] = serde_json::json!(effort);
            }
        }
        Some(ThinkingFamily::OpenAi) => {
            body["reasoning_effort"] = serde_json::json!(if disabled {
                "minimal"
            } else {
                level
            });
        }
        None => {
            if disabled {
                // The name reveals no family, but the table knows the model
                // reasons. "Turn off" was verified live against zen:
                // `reasoning_effort: "none"` where the provider lists that
                // value, otherwise the `thinking` toggle form. Sending
                // nothing would leave the provider default — on — in force.
                let knows_reasoning = caps.as_ref().is_some_and(|entry| entry.reasoning);
                let allows_none = caps
                    .as_ref()
                    .and_then(|entry| entry.effort.as_deref())
                    .is_some_and(|values| values.iter().any(|value| value == "none"));
                if knows_reasoning {
                    if allows_none {
                        body["reasoning_effort"] = serde_json::json!("none");
                    } else {
                        body["thinking"] = serde_json::json!({ "type": "disabled" });
                    }
                }
            } else {
                body["reasoning_effort"] = serde_json::json!(level);
            }
        }
    }
}

/// A streamed piece of the model output.
pub enum LlmDelta {
    Reasoning(String),
    Content(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn format(
        &self,
        text: &str,
        prompt: &str,
        on_event: Box<dyn FnMut(LlmDelta) + Send>,
    ) -> Result<(String, LlmUsage), String>;
}

pub struct OpenAiProvider {
    pub client: reqwest::Client,
    pub config: LlmConfig,
}

impl OpenAiProvider {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            client: shared_client(),
            config,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn format(
        &self,
        text: &str,
        prompt: &str,
        mut on_event: Box<dyn FnMut(LlmDelta) + Send>,
    ) -> Result<(String, LlmUsage), String> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let started = std::time::Instant::now();
        log::info!(
            "llm: request {} model={} chars={}",
            self.config.base_url,
            self.config.model,
            text.chars().count()
        );

        let mut body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": text}
            ],
            "temperature": 0.3,
            "max_tokens": self.config.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        apply_thinking(&mut body, &self.config.provider, &self.config.model, &self.config.reasoning);

        let mut response = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(self.config.timeout_secs.max(1)))
            .headers(request_headers(&self.config.provider, &self.config.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        log::info!(
            "llm: response headers after {}ms (status {})",
            started.elapsed().as_millis(),
            response.status()
        );

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("LLM API error {}: {}", status, body));
        }

        let mut buffer: Vec<u8> = Vec::new();
        let mut raw_body: Vec<u8> = Vec::new();
        let mut full = String::new();
        let mut done = false;
        let mut first_delta = true;
        let mut first_reasoning = true;
        let mut finish_reason: Option<String> = None;
        let mut usage = LlmUsage::default();

        while !done {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => return Err(format!("Failed to read LLM stream: {}", error)),
            };
            raw_body.extend_from_slice(&chunk);
            buffer.extend_from_slice(&chunk);

            while let Some(newline) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line);
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    done = true;
                    break;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                    let choice = &value["choices"][0];
                    if let Some(reason) = choice["finish_reason"].as_str() {
                        finish_reason = Some(reason.to_string());
                    }
                    if let Some(u) = value.get("usage") {
                        if let Some(pt) = u["prompt_tokens"].as_u64() {
                            usage.prompt_tokens = pt;
                        }
                        if let Some(ct) = u["completion_tokens"].as_u64() {
                            usage.completion_tokens = ct;
                        }
                    }
                    let delta = &choice["delta"];
                    if let Some(reasoning) = delta["reasoning_content"].as_str() {
                        if !reasoning.is_empty() {
                            if first_reasoning {
                                first_reasoning = false;
                                log::info!(
                                    "llm: first reasoning token after {}ms",
                                    started.elapsed().as_millis()
                                );
                            }
                            on_event(LlmDelta::Reasoning(reasoning.to_string()));
                        }
                    }
                    if let Some(content) = delta["content"].as_str() {
                        if !content.is_empty() {
                            if first_delta {
                                first_delta = false;
                                log::info!(
                                    "llm: first token after {}ms",
                                    started.elapsed().as_millis()
                                );
                            }
                            let text = content.to_string();
                            full.push_str(&text);
                            on_event(LlmDelta::Content(text));
                        }
                    }
                }
            }
        }

        // Some gateways ignore `stream: true` and answer with a normal JSON
        // body; fall back to parsing the whole response in that case.
        if full.is_empty() {
            log::info!("llm: no content tokens; parsing buffered response");
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw_body) {
                let choice = &value["choices"][0];
                if let Some(reason) = choice["finish_reason"].as_str() {
                    finish_reason = Some(reason.to_string());
                }
                if let Some(content) = choice["message"]["content"].as_str() {
                    if !content.is_empty() {
                        let text = content.to_string();
                        full.push_str(&text);
                        on_event(LlmDelta::Content(text));
                    }
                }
                if let Some(u) = value.get("usage") {
                    if let Some(pt) = u["prompt_tokens"].as_u64() {
                        usage.prompt_tokens = pt;
                    }
                    if let Some(ct) = u["completion_tokens"].as_u64() {
                        usage.completion_tokens = ct;
                    }
                }
            }
        }

        log::info!(
            "llm: done after {}ms ({} chars, finish_reason={:?})",
            started.elapsed().as_millis(),
            full.chars().count(),
            finish_reason
        );

        if full.is_empty() {
            let reason = finish_reason.as_deref().unwrap_or("unknown");
            if reason == "length" {
                return Err(
                    "输出被 max_tokens 截断：思考占满了预算、没有正文；请调大 max_tokens 或关闭思考"
                        .into(),
                );
            }
            return Err(format!("模型没有返回正文（finish_reason: {reason}）"));
        }

        Ok((full, usage))
    }
}

/// Fetch the OpenAI-compatible model list from `{base_url}/models`.
pub async fn list_models(
    base_url: &str,
    api_key: &str,
    provider: &str,
) -> Result<Vec<String>, String> {
    if provider.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Two-state source: the curated table answers for providers it covers
    // (clean id list, no network); the live endpoint stays the source of
    // truth for local and custom endpoints, which no catalog can know.
    if let Some(ids) = crate::model_caps::list_ids(provider) {
        return Ok(ids);
    }
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let response = shared_client()
        .get(&url)
        .timeout(Duration::from_secs(30))
        .headers(request_headers(provider, api_key))
        .send()
        .await
        .map_err(|e| format!("获取模型列表失败：{e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("模型列表接口返回 {status}：{}", body.trim()));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败：{e}"))?;
    let mut ids: Vec<String> = value["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    if ids.is_empty() {
        return Err("模型列表为空".into());
    }
    Ok(ids)
}

pub fn create_provider(config: &LlmConfig) -> Box<dyn LlmProvider> {
    match config.provider.as_str() {
        "openai" | "claude" | "deepseek" | "groq" | "ollama" => {
            Box::new(OpenAiProvider::new(config.clone()))
        }
        _ => Box::new(OpenAiProvider::new(config.clone())),
    }
}
