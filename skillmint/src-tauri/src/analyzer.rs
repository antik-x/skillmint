//! PRD-08 §3.6 (P1): the LLM daily-summary analyzer — a 1:1 Rust port of
//! AI-Digest's `src/digest/analyzer.py`.
//!
//! Given a date, it gathers the day's sessions, builds a bounded context text,
//! asks the LLM for a Chinese daily work digest (highlights + clustered
//! activities), parses the JSON, and persists it into `digest_summary`.
//!
//! Graceful degradation: with no `api_key`, no sessions, or any failure, it
//! returns a structured `Outcome` describing why — the pipeline never hard-fails.

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::llm;
use crate::settings::AiConfig;

/// Cap on the total context text length sent to the LLM. Matches
/// `Analyzer.MAX_CONTEXT_CHARS`.
const MAX_CONTEXT_CHARS: usize = 60000;

/// The system prompt (fixed). Identical to the Python original for caliber
/// parity; `{date}` is substituted at call time.
fn system_prompt(date: &str) -> String {
    format!(
"你是一位专业的工程效率分析师。请分析用户当天与各 AI Agent 的交互日志，生成一份详细的中文每日工作摘要。
日期：{date}

规则：
1. 忽略无意义的交互（如只说了\"hi\"或空对话）。
2. 将相关会话按时间线或项目聚合为\"活动（Activity）\"。
3. 每个活动的 summary 要简洁概括做了什么事。
4. **details 必须详细列出该活动中具体做的每一个要点**，不能笼统带过。例如：修改了哪些文件、实现了什么功能、讨论了什么技术概念、产出了什么文档等。至少列出 3-5 个要点。
5. 如果日志中项目名缺失，请根据上下文推断项目或仓库名（如 \"skill-hub\"、\"ai-digest\"）。实在推断不出就填 \"-\"。
6. highlights 提供 1-2 句当天整体亮点概述。
7. 所有文本内容（summary、details、highlights 等）必须使用中文。技术术语保留英文。
8. 必须输出合法 JSON，匹配以下 schema，不要包含 markdown 代码块。

JSON 输出格式示例：
{{
  \"date\": \"{date}\",
  \"highlights\": [\"全天主要围绕 SkillMint 使用洞察页升级和 AI-Digest 分析方法移植\"],
  \"activities\": [
    {{
      \"time_range\": \"09:00 - 11:30\",
      \"project\": \"skill-hub\",
      \"category\": \"coding\",
      \"summary\": \"实现了 AI-Digest 分析引擎的 Rust 移植\",
      \"details\": [
        \"编写了 pricing.rs 定价引擎，支持精确与模糊匹配\",
        \"移植了 token_metrics / prompt_metrics 指标计算\",
        \"设计了环环比同比的 window_metrics 编排\",
        \"为使用洞察页接入了四维语义分布展示\"
      ]
    }}
  ]
}}"
    )
}

/// The structured daily summary (matches the LLM JSON schema).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailySummary {
    pub date: String,
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub activities: Vec<ActivityItem>,
}

/// One clustered activity within a day.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityItem {
    #[serde(default)]
    pub time_range: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: Vec<String>,
}

/// Why a summary could not be generated (for the UI to show a helpful message).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    /// Successfully generated (and persisted) a summary.
    Generated { summary: DailySummary },
    /// No LLM key configured.
    NoKey,
    /// The day had no sessions.
    NoSessions,
    /// The LLM call failed or returned unparseable output.
    Failed { reason: String },
}

/// Generate (and persist) the daily summary for `date` (YYYY-MM-DD).
pub fn generate_daily_summary(db: &Db, date: &str, cfg: &AiConfig) -> anyhow::Result<Outcome> {
    if !llm::is_configured(cfg) {
        return Ok(Outcome::NoKey);
    }

    let sessions = db.query_sessions_for_day(date)?;
    if sessions.is_empty() {
        return Ok(Outcome::NoSessions);
    }

    let context = build_context_text(&sessions);
    if context.is_empty() {
        return Ok(Outcome::NoSessions);
    }

    let system = system_prompt(date);
    let outcome = llm::chat_with_outcome(cfg, &system, &format!("Logs:\n{context}"));
    db.log_llm_request(&outcome, "daily_summary");
    let content = match outcome.content {
        Some(c) => c,
        None => return Ok(Outcome::Failed { reason: outcome.error.unwrap_or_else(|| "LLM 请求失败".into()) }),
    };
    let clean = llm::strip_codefence(&content);
    let summary: DailySummary = match serde_json::from_str(&clean) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Outcome::Failed {
                reason: format!("LLM 返回 JSON 解析失败: {e}"),
            });
        }
    };

    // Persist into digest_summary (replace any existing row for this date).
    let model_name = cfg
        .default_chat_model_id
        .as_ref()
        .and_then(|id| cfg.models.iter().find(|m| &m.id == id))
        .map(|m| m.model.as_str())
        .unwrap_or("");
    db.save_daily_summary(date, &summary, model_name)?;
    Ok(Outcome::Generated { summary })
}

/// One day's session, in the shape needed to build the LLM context. Mirrors
/// AI-Digest `NormalizedSession` (the fields the analyzer actually reads).
#[derive(Debug, Clone)]
pub struct DaySession {
    pub start_time: Option<u64>, // epoch seconds (local)
    pub source: String,
    pub project_path: Option<String>,
    pub title_or_prompt: Option<String>,
    pub message_count: i64,
}

/// Build the bounded context text from the day's sessions. Mirrors
/// `_build_context_text`: total length capped at `MAX_CONTEXT_CHARS`.
fn build_context_text(sessions: &[DaySession]) -> String {
    let mut out = String::new();
    for s in sessions {
        let time_str = s
            .start_time
            .and_then(local_hhmm)
            .unwrap_or_else(|| "--:--".to_string());
        let project = s
            .project_path
            .as_deref()
            .map(|p| format!(" | Project: {p}"))
            .unwrap_or_default();
        let title = s.title_or_prompt.clone().unwrap_or_default();
        let header = format!(
            "[{time_str}] Source: {source}{project}\nTitle/Summary: {title}\nMessages: {msgs}\n",
            source = s.source,
            title = title,
            msgs = s.message_count,
        );
        if out.len() + header.len() + 4 > MAX_CONTEXT_CHARS {
            break;
        }
        out.push_str(&header);
        out.push_str("---\n");
    }
    out
}

/// Format an epoch-seconds timestamp as local HH:MM.
fn local_hhmm(epoch_secs: u64) -> Option<String> {
    use chrono::Local;
    use chrono::TimeZone;
    let dt = Local.timestamp_opt(epoch_secs as i64, 0).single()?;
    Some(dt.format("%H:%M").to_string())
}
