//! PRD-08: a minimal LLM HTTP client shared by the prompt classifier
//! (`classifier.rs`) and the daily-summary analyzer (`analyzer.rs`).
//!
//! Supports two request envelopes — OpenAI-compatible (default) and Anthropic —
//! selected by `AiConfig.provider`. Uses the already-vendored `reqwest` (blocking
//! + rustls, no system OpenSSL) so no new dependency is introduced.
//!
//! Graceful degradation: every public entry point returns `Ok(None)` (or an
//! empty result) when `api_key` is empty or the network call fails. The analysis
//! pipeline must never hard-fail on LLM unavailability.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use crate::settings::AiConfig;

/// Shared blocking HTTP client for all LLM calls. `reqwest::blocking::Client`
/// is cheap to clone but expensive to build; caching it avoids repeating TLS
/// setup on every classification/summary request.
fn http_client() -> anyhow::Result<&'static reqwest::blocking::Client> {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| anyhow::anyhow!(e))?;
    // If another thread raced and initialized first, ignore the error.
    let _ = CLIENT.set(client);
    Ok(CLIENT.get().expect("client was just set"))
}

/// One chat message: role ("system" / "user") + content.
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system", content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user", content: content.into() }
    }
}

fn default_chat_model(cfg: &AiConfig) -> Option<&crate::settings::AiModelConfig> {
    let id = cfg.default_chat_model_id.as_deref()?;
    cfg.models.iter().find(|m| m.id == id && m.capabilities.contains(&"chat".to_string()))
}

/// Whether a chat-capable model with a non-empty API key is configured.
/// （仅云端口径——cloud_chat 的内部闸门用这个，避免 ACP 启用时拿空 Key 打云端。）
pub fn has_cloud_key(cfg: &AiConfig) -> bool {
    default_chat_model(cfg).map_or(false, |m| !m.api_key.trim().is_empty())
}

/// P5: 任一启用的 ACP 连接（命令非空）即视为本地分析引擎可用。
pub fn has_enabled_acp(cfg: &AiConfig) -> bool {
    cfg.acp_connections.iter().any(|c| {
        c.enabled
            && match &c.transport {
                crate::acp::AcpTransport::Stdio { command, .. } => !command.trim().is_empty(),
                crate::acp::AcpTransport::Sse { .. } => false,
            }
    })
}

/// 「AI 已配置」= 云端 Key 或 ACP 连接任一可用（Q3 决议：无 Key 纯本地模式）。
pub fn is_configured(cfg: &AiConfig) -> bool {
    has_cloud_key(cfg) || has_enabled_acp(cfg)
}

/// Outcome of a single LLM/ACP chat attempt, including routing metadata
/// so callers can log whether data left the local machine.
#[derive(Debug, Clone)]
pub struct ChatOutcome {
    pub content: Option<String>,
    /// Provider identifier, e.g. "acp:claude", "cloud:default:gpt-4o-mini", "none".
    pub provider: String,
    /// True when the first attempted provider failed and a fallback was used.
    pub fallback: bool,
    /// Human-readable error when content is None.
    pub error: Option<String>,
}

impl ChatOutcome {
    fn ok(provider: impl Into<String>, content: String) -> Self {
        Self {
            content: Some(content),
            provider: provider.into(),
            fallback: false,
            error: None,
        }
    }
    fn fallback(provider: impl Into<String>, content: String) -> Self {
        Self {
            content: Some(content),
            provider: provider.into(),
            fallback: true,
            error: None,
        }
    }
    fn err(provider: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            content: None,
            provider: provider.into(),
            fallback: false,
            error: Some(error.into()),
        }
    }
}

/// Call the configured LLM with a system+user message pair and return the text
/// content of its reply. Returns `Ok(None)` on any failure (no key, network
/// error, unexpected response shape) so callers degrade gracefully.
pub fn chat(cfg: &AiConfig, system: &str, user: &str) -> Option<String> {
    chat_with_outcome(cfg, system, user).content
}

/// 仅走云端（连接测试等语义上专测云模型的场景，避免被 prefer_acp 串路由）。
pub fn chat_cloud(cfg: &AiConfig, system: &str, user: &str) -> Option<String> {
    cloud_chat(cfg, system, user)
}

/// Same as `chat` but returns routing metadata for audit logging.
///
/// P5 路由语义：prefer_acp 时按连接顺序尝试（冷却中的跳过），首个成功者胜出；
/// 全部失败时 strict_local_mode 报错不回退，否则回退云端——并把 ACP 失败摘要
/// 写进 outcome.error，审计日志能看到「为什么没走本地」。
pub fn chat_with_outcome(cfg: &AiConfig, system: &str, user: &str) -> ChatOutcome {
    let mut acp_attempts: Option<String> = None;
    if cfg.prefer_acp {
        match crate::acp::route_chat(&cfg.acp_connections, system, user) {
            crate::acp::AcpRouteResult::Succeeded { connection, reply, attempts } => {
                if !attempts.is_empty() {
                    // 有连接失败后降级成功的，也留痕。
                    acp_attempts = Some(attempts.join("；"));
                }
                return ChatOutcome {
                    content: Some(reply),
                    provider: format!("acp:{connection}"),
                    fallback: !attempts.is_empty(),
                    error: acp_attempts,
                };
            }
            crate::acp::AcpRouteResult::Failed { attempts } => {
                let summary = if attempts.is_empty() {
                    "没有已启用的 ACP 连接".to_string()
                } else {
                    attempts.join("；")
                };
                if cfg.strict_local_mode {
                    return ChatOutcome::err(
                        "acp",
                        format!("本地 Agent 调用失败且已开启严格本地模式，禁止回退到云端 LLM（{summary}）"),
                    );
                }
                acp_attempts = Some(format!("本地 Agent 全部失败（{summary}），已回退云端"));
            }
        }
    }
    // Fall back to cloud HTTP provider.
    match cloud_chat(cfg, system, user) {
        Some(reply) => {
            let provider = default_chat_model(cfg)
                .map(|m| format!("cloud:{}", m.id))
                .unwrap_or_else(|| "cloud:unknown".to_string());
            ChatOutcome { content: Some(reply), provider, fallback: true, error: acp_attempts }
        }
        None => {
            let mut error = String::from("云端 LLM 请求失败或未配置");
            if let Some(note) = acp_attempts {
                error = format!("{note}；{error}");
            }
            ChatOutcome::err("cloud", error)
        }
    }
}

fn cloud_chat(cfg: &AiConfig, system: &str, user: &str) -> Option<String> {
    let model_cfg = default_chat_model(cfg)?;
    if !has_cloud_key(cfg) {
        return None;
    }
    let model = if model_cfg.model.trim().is_empty() {
        "gpt-4o-mini".to_string()
    } else {
        model_cfg.model.clone()
    };

    let (endpoint, body, auth_header) = match model_cfg.provider {
        crate::settings::AiProvider::Anthropic => anthropic_request(model_cfg, &model, system, user),
        crate::settings::AiProvider::OpenAI => openai_request(model_cfg, &model, system, user),
    };

    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[llm] failed to build http client: {}", e);
            return None;
        }
    };

    let mut req = client
        .post(&endpoint)
        .header("Content-Type", "application/json");
    for (k, v) in auth_header {
        req = req.header(k, v);
    }
    let resp = req.body(body).send().ok()?;
    if !resp.status().is_success() {
        eprintln!("[llm] request failed: HTTP {}", resp.status());
        return None;
    }
    let text = resp.text().ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    match model_cfg.provider {
        crate::settings::AiProvider::Anthropic => json
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string()),
        crate::settings::AiProvider::OpenAI => json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string()),
    }
}

/// Build the OpenAI-compatible chat/completions request (endpoint, body, auth).
fn openai_request(
    cfg: &crate::settings::AiModelConfig,
    model: &str,
    system: &str,
    user: &str,
) -> (String, String, Vec<(&'static str, String)>) {
    let mut endpoint = if cfg.base_url.trim().is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        cfg.base_url.trim().trim_end_matches('/').to_string()
    };
    if !endpoint.ends_with("/chat/completions") && !endpoint.ends_with("/completions") {
        endpoint.push_str("/chat/completions");
    }
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": 8192,
    })
    .to_string();
    let auth = vec![("Authorization", format!("Bearer {}", cfg.api_key))];
    (endpoint, body, auth)
}

/// Build the Anthropic /v1/messages request.
fn anthropic_request(
    cfg: &crate::settings::AiModelConfig,
    model: &str,
    system: &str,
    user: &str,
) -> (String, String, Vec<(&'static str, String)>) {
    let mut endpoint = if cfg.base_url.trim().is_empty() {
        "https://api.anthropic.com".to_string()
    } else {
        cfg.base_url.trim().trim_end_matches('/').to_string()
    };
    if !endpoint.ends_with("/v1/messages") && !endpoint.ends_with("/messages") {
        endpoint.push_str("/v1/messages");
    }
    let body = serde_json::json!({
        "model": model,
        "system": system,
        "messages": [{"role": "user", "content": user}],
        "max_tokens": 4096,
    })
    .to_string();
    let auth = vec![
        ("x-api-key", cfg.api_key.clone()),
        ("anthropic-version", "2023-06-01".to_string()),
    ];
    (endpoint, body, auth)
}

/// Strip ```json ... ``` wrapping if present, and unwrap a top-level object that
/// wraps the array under a known key. Mirrors `_strip_codefence`.
pub fn strip_codefence(text: &str) -> String {
    let mut t = text.trim().to_string();
    if let Some(rest) = t.strip_prefix("```json") {
        t = rest.to_string();
    } else if let Some(rest) = t.strip_prefix("```") {
        t = rest.to_string();
    }
    if let Some(rest) = t.strip_suffix("```") {
        t = rest.to_string();
    }
    let t = t.trim();
    // Some models wrap the array in {"prompts": [...]} etc.
    if t.starts_with('{') {
        if let Ok(obj) = serde_json::from_str::<Value>(t) {
            if let Some(obj) = obj.as_object() {
                for k in ["prompts", "results", "data", "items"] {
                    if let Some(arr) = obj.get(k).and_then(|v| v.as_array()) {
                        return serde_json::to_string(arr).unwrap_or_else(|_| t.to_string());
                    }
                }
            }
        }
    }
    t.to_string()
}

/// Recover complete JSON objects from a possibly-truncated array string.
/// When the LLM output hits the token limit mid-array, `serde_json::from_str`
/// fails on the whole; this extracts every fully-formed `{"id":..., ...}` object
/// instead. Mirrors `_salvage_json_array`.
pub fn salvage_json_array(text: &str) -> Vec<Value> {
    let mut result = Vec::new();
    // Walk the string collecting balanced {...} substrings containing an "id".
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // find the matching closing brace at depth 0
            let mut depth = 0i32;
            let mut j = i;
            let mut in_str = false;
            let mut esc = false;
            while j < bytes.len() {
                let c = bytes[j];
                if in_str {
                    if esc {
                        esc = false;
                    } else if c == b'\\' {
                        esc = true;
                    } else if c == b'"' {
                        in_str = false;
                    }
                } else if c == b'"' {
                    in_str = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        let chunk = &text[i..=j];
                        if chunk.contains("\"id\"") {
                            if let Ok(v) = serde_json::from_str::<Value>(chunk) {
                                result.push(v);
                            }
                        }
                        break;
                    }
                }
                j += 1;
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    result
}
