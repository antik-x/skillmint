//! SPEC-I2: discovery pipeline — four detectors + optional LLM enhancement.

use std::collections::HashSet;

use chrono::Datelike;
use crate::db::Db;
use crate::models::{
    Discovery, DiscoveryDecisionResult, DiscoveryKind, DiscoveryRunResult, DiscoveryStatus,
    SyncFailure, SyncSummary, WeeklyReport,
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
    center_repo: &std::path::Path,
    id: &str,
    action: &str,
    reason: Option<&str>,
) -> anyhow::Result<DiscoveryDecisionResult> {
    let discovery = db
        .get_discovery_by_id(id)?
        .ok_or_else(|| anyhow::anyhow!("Discovery not found"))?;

    match action {
        "accept" => decide_accept(db, device_id, center_repo, discovery, /*sync=*/ true),
        "accept_edited" => decide_accept(db, device_id, center_repo, discovery, /*sync=*/ false),
        "dismiss" => decide_dismiss(db, discovery, reason),
        other => anyhow::bail!("unknown action: {}", other),
    }
}

/// Shared body for accept / accept_edited. When `sync` is true the created
/// skill is pushed to every enabled agent via [`sync_single_skill`] and the
/// failures are surfaced in `sync_summary`.
fn decide_accept(
    db: &Db,
    device_id: &str,
    center_repo: &std::path::Path,
    mut discovery: Discovery,
    sync: bool,
) -> anyhow::Result<DiscoveryDecisionResult> {
    let draft_name = discovery
        .payload
        .get("draft_skill")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now = crate::db::now_secs();

    let skill_id = match draft_name {
        Some(name) => create_skill_draft(db, device_id, center_repo, &name, &discovery)?,
        None => None,
    };

    // Sync only when the caller asked for the direct path AND we actually
    // created a skill. accept_edited stops here so the user can edit first.
    let mut sync_summary: Option<SyncSummary> = None;
    if let Some(ref sid) = skill_id {
        if sync {
            let report = sync_single_skill(db, sid)?;
            sync_summary = Some(SyncSummary {
                success: report.updated.len(),
                failed: report.broken.len(),
                failures: report.broken,
            });
        }
    }

    // Persist the decision. Skill creation has succeeded; even if sync had
    // failures we mark the discovery accepted (PRD-12 A-R1: never roll back a
    // created skill because of a sync failure).
    db.update_discovery_status(
        &discovery.id,
        DiscoveryStatus::Accepted,
        Some(now),
        skill_id.as_deref(),
    )?;
    let decision = if sync { "adopted_as_is" } else { "adopted_edited" };
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
            // DEPRECATION: callers should always pass an explicit reason. The
            // default keeps old frontends working until SPEC-C2 lands.
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

fn create_skill_draft(
    db: &Db,
    device_id: &str,
    center_repo: &std::path::Path,
    name: &str,
    discovery: &Discovery,
) -> anyhow::Result<Option<String>> {
    let skill_dir = center_repo.join(name);
    if skill_dir.exists() {
        anyhow::bail!("Skill '{}' already exists", name);
    }
    std::fs::create_dir_all(&skill_dir)?;

    let draft_body = discovery
        .payload
        .get("draft_skill")
        .and_then(|v| v.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let body = if draft_body.is_empty() {
        format!("# {}\n\n## Description\n\n从发现「{}」自动生成的 Skill 草稿。\n\n## Usage\n\n请补充使用方式。\n", name, discovery.title)
    } else {
        draft_body.to_string()
    };
    std::fs::write(skill_dir.join("SKILL.md"), body)?;

    let now = crate::db::now_secs();
    let skill = crate::models::Skill {
        id: crate::db::new_id(),
        name: name.to_string(),
        repo_path: skill_dir,
        created_at: now,
        updated_at: now,
        status: crate::models::SkillStatus::Draft,
    };
    db.insert_skill(&skill)?;

    // Best-effort KG extraction.
    crate::kg::analyze_skill(db, device_id, &skill.id, &skill.repo_path.join("SKILL.md")).ok();

    Ok(Some(skill.id))
}

/// SPEC-C1 T2: sync a single skill to every enabled agent.
///
/// This is the adopt-and-sync direct path: when a discovery is accepted
/// (`action="accept"`) the freshly created skill is pushed to every agent
/// immediately so the user never has to open the sync panel. The same
/// per-target evaluate/apply logic as [`crate::sync::sync_all`] is reused,
/// scoped to one skill, and a fresh sync_target is created for each enabled
/// agent that does not yet have one (mirroring `ensure_sync_targets`).
///
/// Partial failures are reported, not fatal: the caller keeps the skill and
/// surfaces the failures in `sync_summary` (PRD-12 red line A-R1).
pub fn sync_single_skill(db: &Db, skill_id: &str) -> anyhow::Result<crate::sync::SyncReport> {
    use crate::models::{Agent, SyncMode, SyncStatus, SyncTarget};
    use crate::sync::{apply_sync_target_and_record, evaluate_sync_target, sync_failure_from_error};

    let skill = db
        .get_skills()?
        .into_iter()
        .find(|s| s.id == skill_id)
        .ok_or_else(|| anyhow::anyhow!("skill not found: {}", skill_id))?;

    let agents: std::collections::HashMap<String, Agent> = db
        .get_agents()?
        .into_iter()
        .filter(|a| a.is_enabled)
        .map(|a| (a.id.clone(), a))
        .collect();

    let existing: Vec<SyncTarget> = db
        .get_sync_targets()?
        .into_iter()
        .filter(|t| t.skill_id == skill_id)
        .collect();

    let now = crate::db::now_secs();

    let mut updated = Vec::new();
    let mut broken: Vec<SyncFailure> = Vec::new();

    for agent in agents.values() {
        // Ensure a target exists for this (skill, agent) pair.
        let target = match existing.iter().find(|t| t.agent_id == agent.id) {
            Some(t) => t.clone(),
            None => {
                let id = crate::db::new_id();
                let new_target = SyncTarget {
                    id: id.clone(),
                    skill_id: skill.id.clone(),
                    skill_name: Some(skill.name.clone()),
                    agent_id: agent.id.clone(),
                    agent_name: Some(agent.name.clone()),
                    mode: SyncMode::Symlink,
                    last_sync_at: None,
                    status: SyncStatus::CenterChanged,
                };
                if let Err(e) = db.insert_sync_target(&new_target) {
                    broken.push(SyncFailure {
                        target_id: id,
                        skill_id: skill.id.clone(),
                        skill_name: Some(skill.name.clone()),
                        agent_id: agent.id.clone(),
                        agent_name: Some(agent.name.clone()),
                        error: format!("无法创建同步目标：{}", e),
                        recovery_hint: None,
                    });
                    continue;
                }
                new_target
            }
        };

        let status = match evaluate_sync_target(&target, &skill, agent) {
            Ok(s) => s,
            Err(e) => {
                broken.push(sync_failure_from_error(&target, e));
                continue;
            }
        };

        // A freshly created skill is always CenterChanged relative to the
        // agent dir (which doesn't have it yet); apply unconditionally so the
        // file lands even when evaluate returned something unexpected.
        match apply_sync_target_and_record(db, &target, &skill, agent) {
            Ok(new_status) => updated.push(SyncTarget {
                status: new_status,
                last_sync_at: Some(now),
                ..target
            }),
            Err(e) => {
                broken.push(sync_failure_from_error(&target, e));
            }
        }
        let _ = status; // status evaluated but apply drives the final state
    }

    Ok(crate::sync::SyncReport { updated, broken })
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

    let projects = match db.get_project_usage_summary(device_id, 7, start as u64) {
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
