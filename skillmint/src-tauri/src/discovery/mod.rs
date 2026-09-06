//! SPEC-I2: discovery pipeline — four detectors + optional LLM enhancement.

use std::collections::HashSet;

use chrono::Datelike;
use crate::db::Db;
use crate::models::{
    Discovery, DiscoveryDecisionResult, DiscoveryKind, DiscoveryRunResult, DiscoveryStatus,
    SyncSummary, WeeklyReport,
};
use crate::settings::AiConfig;

pub mod config;
pub mod keywords;

pub(crate) mod capability_gap;
mod high_value_prompt;
pub(crate) mod llm_enhance;
pub(crate) mod repeat_pattern;
mod skill_feedback;

/// Input context shared by all detectors.
pub struct DetectContext<'a> {
    pub db: &'a Db,
    pub device_id: &'a str,
    pub window_start: i64,
    pub window_end: i64,
}

/// A candidate before persistence. Detectors only produce candidates; the
/// pipeline handles dedup, ranking, persistence, and optional LLM enhancement.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveryCandidate {
    pub kind: DiscoveryKind,
    pub title: String,
    pub payload: serde_json::Value,
    pub confidence: f64,
    pub dedup_key: String,
}

/// Every detector implements this trait. Failures are isolated inside the
/// pipeline so one broken detector never blocks the others.
pub trait Detector: Send + Sync {
    fn kind(&self) -> DiscoveryKind;
    fn detect(&self, ctx: &DetectContext) -> anyhow::Result<Vec<DiscoveryCandidate>>;
}

/// Run the full discovery pipeline: expire stale → detect → dedup → top-5 →
/// optional LLM enhance → insert. Idempotent via dedup_key.
pub fn run_pipeline(
    db: &mut Db,
    device_id: &str,
    cfg: &AiConfig,
) -> anyhow::Result<DiscoveryRunResult> {
    let now = crate::db::now_secs();
    let window_start = (now as i64) - config::DETECTION_WINDOW_DAYS * 86400;
    let window_end = now as i64;
    let ctx = DetectContext {
        db,
        device_id,
        window_start,
        window_end,
    };

    // 1. Expire stale pending discoveries.
    let expired = db.expire_stale_discoveries(config::STALE_DISCOVERY_DAYS)?;

    // 2. Run all detectors, isolating individual failures.
    let detectors: Vec<Box<dyn Detector>> = vec![
        Box::new(repeat_pattern::RepeatPatternDetector),
        Box::new(high_value_prompt::HighValuePromptDetector),
        Box::new(skill_feedback::SkillFeedbackDetector),
        Box::new(capability_gap::CapabilityGapDetector),
    ];

    let mut all_candidates: Vec<DiscoveryCandidate> = Vec::new();
    for det in &detectors {
        match det.detect(&ctx) {
            Ok(mut cands) => all_candidates.append(&mut cands),
            Err(e) => eprintln!("[discovery] {} detector failed: {}", det.kind(), e),
        }
    }

    // 3. Dedup: skip candidates whose dedup_key is already pending or recently dismissed.
    //    SPEC-C1 T3: cooling is now tiered by the last reject_reason (duplicate/
    //    trivial/wrong), falling back to the uniform window for legacy rows.
    //    SPEC-C1 T4: each suppressed candidate is recorded in gate_rejections
    //    with reason='cooling' so the inbox can explain *why* a signal vanished.
    let mut seen_dedup: HashSet<String> = HashSet::new();
    for d in db.list_discoveries(Some("pending"))? {
        seen_dedup.insert(d.dedup_key);
    }

    let mut filtered: Vec<DiscoveryCandidate> = Vec::new();
    for c in all_candidates {
        if seen_dedup.contains(&c.dedup_key) {
            continue;
        }
        let reason = db.latest_reject_reason_for_dedup(&c.dedup_key)?;
        let cooling_days = config::cooling_days_for_reason(reason.as_deref());
        if db.is_dedup_cooling(&c.dedup_key, cooling_days)? {
            let _ = db.insert_gate_rejection(
                "cooling",
                c.kind.to_string().as_str(),
                c.confidence,
                &serde_json::json!({
                    "title": c.title,
                    "dedup_key": c.dedup_key,
                    "reject_reason": reason,
                    "cooling_days": cooling_days,
                }),
            );
            continue;
        }
        seen_dedup.insert(c.dedup_key.clone());
        filtered.push(c);
    }

    // 4. Keep top N by confidence. Candidates dropped here are recorded as
    //    `daily_limit` rejections; candidates that never made it past the
    //    detector threshold are recorded by the detectors themselves, but we
    //    also note any candidate below the low-confidence band here as
    //    `below_threshold`.
    filtered.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

    // SPEC-C1 T4: record sub-threshold candidates before truncation so the
    // low_confidence inbox band has them. We treat anything below
    // LOW_CONFIDENCE_MIN as silently rejected by the gate.
    let (below, mut kept): (Vec<_>, Vec<_>) = filtered
        .iter()
        .cloned()
        .partition(|c| c.confidence < config::LOW_CONFIDENCE_MIN);
    for c in &below {
        let _ = db.insert_gate_rejection(
            "below_threshold",
            c.kind.to_string().as_str(),
            c.confidence,
            &serde_json::json!({ "title": c.title, "dedup_key": c.dedup_key }),
        );
    }
    // Keep only candidates at or above the low-confidence floor; anything in
    // [LOW_CONFIDENCE_MIN, LOW_CONFIDENCE_MAX) is still surfaced to the inbox
    // (the low_confidence band) rather than silently dropped.
    let _ = &mut kept;
    filtered = kept;

    // Truncate to the daily cap and record the overflow as daily_limit.
    let limit = config::DAILY_DISCOVERY_LIMIT;
    if filtered.len() > limit {
        for c in filtered.iter().skip(limit) {
            let _ = db.insert_gate_rejection(
                "daily_limit",
                c.kind.to_string().as_str(),
                c.confidence,
                &serde_json::json!({ "title": c.title, "dedup_key": c.dedup_key }),
            );
        }
        filtered.truncate(limit);
    }

    // 5. Optional LLM enhancement (never blocks).
    let enhanced = if !filtered.is_empty() && crate::llm::is_configured(cfg) {
        match llm_enhance::enhance(filtered.clone(), cfg) {
            Ok(cands) => cands,
            Err(e) => {
                eprintln!("[discovery] LLM enhance failed, using rule candidates: {}", e);
                filtered
            }
        }
    } else {
        filtered
    };

    // 6. Persist.
    let to_insert: Vec<Discovery> = enhanced
        .into_iter()
        .map(|c| Discovery {
            id: crate::db::new_id(),
            kind: c.kind,
            title: c.title,
            payload: c.payload,
            confidence: c.confidence.clamp(0.0, 1.0),
            dedup_key: c.dedup_key,
            status: DiscoveryStatus::Pending,
            created_at: now,
            decided_at: None,
            resulting_skill_id: None,
        })
        .collect();

    let inserted = db.insert_discoveries(&to_insert)?;

    // SPEC-C1 T4: opportunistically prune the gate-rejection ledger so it
    // never grows unbounded. Runs inline (cheap DELETE) on every pipeline run.
    let _ = db.prune_gate_rejections(config::GATE_REJECTION_RETENTION_DAYS);

    Ok(DiscoveryRunResult { inserted, expired })
}

/// SPEC-C1: decide a discovery with the three-way (really four-value) contract.
///
/// Actions:
/// - `"accept"`: create a Skill **and immediately sync it** to every enabled
///   agent. Records `adopted_as_is`. Partial sync failures do NOT roll back the
///   skill (PRD-12 red line A-R1); the failures are returned in `sync_summary`.
/// - `"accept_edited"`: create a Skill draft only; the frontend navigates to
///   the editor. Records `adopted_edited`.
/// - `"dismiss"`: requires `reason` in {wrong, trivial, duplicate}. Records
///   `rejected` with the reason, which drives tiered cooling (T3). For
///   backward-compat with older frontends a missing reason defaults to
///   `trivial` (deprecation noted inline).
/// - any other value: business error, no state mutation.
pub fn decide_discovery(
    db: &Db,
    device_id: &str,
    hub: &std::path::Path,
    id: &str,
    action: &str,
    reason: Option<&str>,
) -> anyhow::Result<DiscoveryDecisionResult> {
    let discovery = db
        .get_discovery_by_id(id)?
        .ok_or_else(|| anyhow::anyhow!("Discovery not found"))?;

    match action {
        "accept" => decide_accept(db, device_id, hub, discovery, /*sync=*/ true),
        "accept_edited" => decide_accept(db, device_id, hub, discovery, /*sync=*/ false),
        "dismiss" => decide_dismiss(db, discovery, reason),
        other => anyhow::bail!("unknown action: {}", other),
    }
}

/// Shared body for accept / accept_edited. P3 review (Q3a): the draft is
/// authored into the global private hub (git auto-commit) instead of the
/// retired center repo; `sync_summary` stays `None` because distribution is
/// now an explicit `npx skills add`, never an automatic push.
fn decide_accept(
    db: &Db,
    device_id: &str,
    hub: &std::path::Path,
    mut discovery: Discovery,
    _sync: bool,
) -> anyhow::Result<DiscoveryDecisionResult> {
    let draft_name = discovery
        .payload
        .get("draft_skill")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now = crate::db::now_secs();

    let skill_id = match draft_name {
        Some(ref name) => create_skill_draft(db, device_id, hub, name, &discovery)?,
        None => None,
    };

    let sync_summary: Option<SyncSummary> = None;

    // Persist the decision. Skill creation has succeeded (PRD-12 A-R1: never
    // roll back a created skill).
    db.update_discovery_status(
        &discovery.id,
        DiscoveryStatus::Accepted,
        Some(now),
        skill_id.as_deref(),
    )?;
    let decision = if _sync { "adopted_as_is" } else { "adopted_edited" };
    db.insert_adoption_event(
        &discovery.id,
        decision,
        None,
        skill_id.as_deref(),
        now,
    )?;

    discovery.status = DiscoveryStatus::Accepted;
    discovery.decided_at = Some(now);
    discovery.resulting_skill_id = skill_id.clone();

    Ok(DiscoveryDecisionResult {
        discovery,
        created_skill_id: skill_id,
        sync_summary,
    })
}

fn create_skill_draft(
    db: &Db,
    _device_id: &str,
    hub: &std::path::Path,
    name: &str,
    discovery: &Discovery,
) -> anyhow::Result<Option<String>> {
    crate::hub::validate_skill_name(name)?;

    let draft_body = discovery
        .payload
        .get("draft_skill")
        .and_then(|v| v.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let description = format!("从发现「{}」自动生成的 Skill 草稿。", discovery.title);
    let dir = crate::hub::create_skill(hub, "global", name, &description)?;

    let body = if draft_body.is_empty() {
        format!(
            "# {}\n\n## Description\n\n{}\n\n## Usage\n\n请补充使用方式。\n",
            name, description
        )
    } else {
        draft_body.to_string()
    };
    // Overwrite the template body but keep the frontmatter hub::create_skill wrote.
    let skill_md = dir.join("SKILL.md");
    let existing = std::fs::read_to_string(&skill_md).unwrap_or_default();
    let merged = if existing.starts_with("---") {
        if let Some(idx) = existing[3..].find("\n---") {
            let frontmatter = &existing[..3 + idx + 4];
            format!("{}\n{}", frontmatter, body)
        } else {
            body
        }
    } else {
        body
    };
    std::fs::write(&skill_md, merged)?;

    // The hub skill's stable identifier is its name (adoption ledger uses it).
    let _ = db;
    Ok(Some(name.to_string()))
}

fn decide_dismiss(
    db: &Db,
    mut discovery: Discovery,
    reason: Option<&str>,
) -> anyhow::Result<DiscoveryDecisionResult> {
    // Normalize the reason. Old frontends may omit it; default to `trivial`
    // (the legacy uniform cooling) with a deprecation note.
    let reason = match reason {
        Some(r) if matches!(r, "wrong" | "trivial" | "duplicate") => r.to_string(),
        Some(other) => anyhow::bail!(
            "invalid dismiss reason '{}': expected one of wrong/trivial/duplicate",
            other
        ),
        None => {
            "trivial".to_string()
        }
    };

    let now = crate::db::now_secs();
    db.update_discovery_status(&discovery.id, DiscoveryStatus::Dismissed, Some(now), None)?;
    db.insert_adoption_event(&discovery.id, "rejected", Some(&reason), None, now)?;

    discovery.status = DiscoveryStatus::Dismissed;
    discovery.decided_at = Some(now);
    Ok(DiscoveryDecisionResult {
        discovery,
        created_skill_id: None,
        sync_summary: None,
    })
}

/// Generate (or regenerate) the weekly report for `week_start` (YYYY-MM-DD Monday).
pub fn generate_weekly_report(
    db: &Db,
    device_id: &str,
    cfg: &AiConfig,
    week_start: &str,
) -> anyhow::Result<WeeklyReport> {
    let content = if crate::llm::is_configured(cfg) {
        llm_enhance::generate_weekly_report_content(db, device_id, week_start, cfg).unwrap_or_else(|e| {
            eprintln!("[discovery] LLM weekly report failed, using template: {}", e);
            template_weekly_report(db, device_id, week_start)
        })
    } else {
        template_weekly_report(db, device_id, week_start)
    };

    let now = crate::db::now_secs();
    let model = cfg
        .default_chat_model_id
        .as_ref()
        .and_then(|id| cfg.models.iter().find(|m| &m.id == id))
        .map(|m| m.model.clone());
    let report = WeeklyReport {
        week_start: week_start.to_string(),
        content,
        generated_at: now,
        model,
        provider: Some("skillmint".to_string()),
    };
    db.upsert_weekly_report(&report)?;
    Ok(report)
}

fn template_weekly_report(
    db: &Db,
    device_id: &str,
    week_start: &str,
) -> crate::models::WeeklyReportContent {
    use crate::models::{WeeklyGrowthSummary, WeeklyPitfall, WeeklyProjectSummary};

    let start = parse_week_start(week_start);
    let end = start + config::WEEKLY_REPORT_WINDOW_DAYS * 86400;

    // Pass `end` as the "now" anchor so the 7-day cutoff lands exactly on the
    // week start (get_project_usage_summary computes cutoff = now - days*86400).
    let projects = match db.get_project_usage_summary(device_id, 7, end as u64) {
        Ok(list) => list
            .into_iter()
            .take(5)
            .map(|p| WeeklyProjectSummary {
                project_id: p.project_id,
                name: p.name,
                summary: format!("{} 个会话 / {} tokens", p.session_count, p.total_tokens),
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let pitfalls = {
        let rows = db.conn_ref()
            .prepare(
                "SELECT id, project_id, title_or_prompt, message_count
                 FROM collected_sessions
                 WHERE device_id = ?1 AND start_time >= ?2 AND start_time < ?3
                 ORDER BY message_count DESC LIMIT 3",
            )
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![device_id, start, end], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map(|iter| {
                    iter.filter_map(|r| r.ok())
                        .map(|(id, project_id, title, _msgs)| WeeklyPitfall {
                            session_id: id,
                            project_id,
                            title: title.unwrap_or_else(|| "未命名会话".to_string()),
                            lesson: "（LLM 未配置，降级为模板句）".to_string(),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        rows
    };

    let growth = WeeklyGrowthSummary {
        new_skills: Vec::new(),
        eliminated_patterns: Vec::new(),
        accepted_count: 0,
        dismissed_count: 0,
    };

    crate::models::WeeklyReportContent { projects, pitfalls, growth }
}

fn parse_week_start(week_start: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(week_start, "%Y-%m-%d")
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc().timestamp())
        .unwrap_or_else(|_| crate::db::now_secs() as i64)
}

/// Compute this week's Monday in UTC seconds and format as YYYY-MM-DD.
pub fn this_monday_utc() -> String {
    let now = chrono::Utc::now();
    let days_since_monday = now.weekday().num_days_from_monday();
    let monday = now - chrono::Duration::days(days_since_monday as i64);
    monday.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests;
