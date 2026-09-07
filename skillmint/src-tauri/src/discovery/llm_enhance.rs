//! SPEC-I2: optional LLM enhancement layer for discovery candidates and weekly reports.

use super::DiscoveryCandidate;
use crate::settings::AiConfig;

/// Enhance a batch of candidates: polish titles and generate a Skill draft markdown.
pub fn enhance(candidates: Vec<DiscoveryCandidate>, cfg: &AiConfig) -> anyhow::Result<Vec<DiscoveryCandidate>> {
    if candidates.is_empty() {
        return Ok(candidates);
    }

    let system = "你是一位工程经验沉淀助手。请根据下面的候选发现，为每个发现润色一个一句话中文标题，并生成一个 Skill 草稿（name + body）。输出必须是合法 JSON 数组，每个元素包含：title, draft_skill.name, draft_skill.body。不要加 markdown 代码块。";
    let user = format!("候选发现：\n{}", serde_json::to_string_pretty(&candidates)?);

    // P6：分析类 system 统一带哨兵（origin.rs 防线 2），走本地 Agent 时副产品可识别。
    let outcome = crate::llm::chat_with_outcome(cfg, &crate::origin::analysis_system_prompt(system), &user);
    if let Some(content) = outcome.content {
        let clean = crate::llm::strip_codefence(&content);
        let enhanced: Vec<EnhancedItem> = serde_json::from_str(&clean).unwrap_or_default();
        let mut out = Vec::with_capacity(candidates.len());
        for (mut c, e) in candidates.into_iter().zip(enhanced.into_iter()) {
            if !e.title.is_empty() {
                c.title = e.title;
            }
            if let Some(draft) = e.draft_skill {
                let mut payload = c.payload.as_object().cloned().unwrap_or_default();
                payload.insert("draft_skill".to_string(), serde_json::to_value(draft)?);
                c.payload = serde_json::Value::Object(payload);
            }
            out.push(c);
        }
        Ok(out)
    } else {
        Err(anyhow::anyhow!("LLM returned no content"))
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct EnhancedItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    draft_skill: Option<SkillDraft>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SkillDraft {
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
}

/// Generate weekly report content via LLM.
pub fn generate_weekly_report_content(
    db: &crate::db::Db,
    _device_id: &str,
    week_start: &str,
    cfg: &AiConfig,
) -> anyhow::Result<crate::models::WeeklyReportContent> {
    let start = super::parse_week_start(week_start);
    let end = start + super::config::WEEKLY_REPORT_WINDOW_DAYS * 86400;

    let sessions = db.query_sessions_for_day_range(start, end)?;
    if sessions.is_empty() {
        return Ok(crate::models::WeeklyReportContent::default());
    }

    let context = build_weekly_context(&sessions);
    let system = "你是一位工程效率分析师。请根据本周会话日志生成一份中文周报，包含：projects（每个项目一段进展摘要，数组，元素含 project_id/name/summary）、pitfalls（最多 3 次踩坑/浪费会话，元素含 session_id/title/lesson）、growth（本周新增 Skill 列表、消除的重复模式、accepted/dismissed 数量）。输出合法 JSON，不要 markdown 代码块。";
    let outcome = crate::llm::chat_with_outcome(cfg, &crate::origin::analysis_system_prompt(system), &context);
    if let Some(content) = outcome.content {
        let clean = crate::llm::strip_codefence(&content);
        let parsed: crate::models::WeeklyReportContent = serde_json::from_str(&clean)?;
        Ok(parsed)
    } else {
        Err(anyhow::anyhow!("LLM returned no content"))
    }
}

fn build_weekly_context(sessions: &[crate::models::SessionStub]) -> String {
    sessions
        .iter()
        .map(|s| {
            format!(
                "session_id={} project={} title={} messages={}",
                s.session_id,
                s.project_path.as_deref().unwrap_or("-"),
                s.title.as_deref().unwrap_or("-"),
                s.message_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
