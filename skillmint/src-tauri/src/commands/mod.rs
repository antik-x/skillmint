use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::new_id;
use crate::fs::{
    copy_dir_all, get_effective_skill_dir, merge_into_center, remove_path,
    unzip_to, zip_dir,
};
use crate::models::{
    Agent, AgentSkillItem, ApplyBundleResult, AppSettings, BundleExport, BundleExportSkill,
    ConflictContent, ConflictPayload, ConflictResolution, DiffStrategy, Discovery,
    DiscoveryDecisionResult, DiscoveryRunResult, GitCommit, GrowthMetrics,
    ProjectDetail, RepoIntegrity, ResolvedSkill, ResolveResult, RestoreResult,
    RestoreSummary, SafetyScanResult, ScheduledTask, SearchResult, Skill, SkillBundle,
    SkillBundleItem, SkillProjectBinding, SkillRemoteMeta, SkillStatus,
    SnapshotInfo, Source, SourceType, SyncAllResult, SyncMode, SyncStatus, SyncTarget, TaskRun, TrashItem,
    TriggerSource, WeeklyReport,
};
use crate::remote;
use crate::scan::{expand_path, scan_and_persist_agents};
use crate::settings::Settings;
use crate::sync::{apply_sync_target_and_record, resolve_skill_link, sync_all};
use crate::AppState;

mod agents;
mod git;
mod import;
mod npxskills;
mod repair;
mod scan;
mod sync;
mod trash;

pub use agents::*;
pub use git::*;
pub use import::*;
pub use npxskills::*;
pub use repair::*;
pub use scan::*;
pub use sync::*;
pub use trash::*;

fn state_to_model(settings: &Settings) -> AppSettings {
    let mut ai = settings.ai.clone();
    // Always surface the live keys from the keyring to the frontend.
    ai.load_keys();
    AppSettings {
        device_id: settings.device_id.clone(),
        auto_sync_interval_minutes: settings.auto_sync_interval_minutes,
        launch_at_login: settings.launch_at_login,
        show_dock_icon: settings.show_dock_icon,
        onboarding_completed: settings.onboarding_completed,
        remote_enabled: settings.remote_enabled,
        theme: settings.theme.clone(),
        ai,
        npx_package: settings.npx_package.clone(),
        skills_api_url: settings.skills_api_url.clone(),
        proxy_env: settings.proxy_env.clone(),
        disable_telemetry: settings.disable_telemetry,
        node_path_override: settings.node_path_override.clone(),
    }
}

#[tauri::command]
pub fn init_app(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    std::fs::create_dir_all(&settings.center_repo).map_err(|e| e.to_string())?;
    // SPEC-F4 T12 / SPEC-F5 T5: migrate legacy flat skill dirs into the repo/
    // subdir on startup. Idempotent — failures are logged but never block startup.
    match migrate_skills_to_repo(&db, &settings.center_repo) {
        Ok(report) => {
            if !report.failed.is_empty() {
                eprintln!(
                    "[migrate] {} skill(s) could not be migrated at startup: {:?}",
                    report.failed.len(),
                    report.failed
                );
            }
        }
        Err(e) => eprintln!("[migrate] startup migration failed: {}", e),
    }
    scan_and_persist_agents(&db).map_err(|e| e.to_string())?;
    // Issue #1 data cleanup: older builds persisted one row per (name, source)
    // so the same directory accrued many duplicate agent rows (e.g. dozens of
    // "Agent" rows at ~/.skillmint/skills). Remove the leftovers now that
    // discover_agents dedups by directory path.
    match db.dedup_agent_directories() {
        Ok(n) if n > 0 => eprintln!("[migrate] removed {} duplicate agent row(s)", n),
        Ok(_) => {}
        Err(e) => eprintln!("[migrate] agent dedup failed: {}", e),
    }
    // PRD-06: a newly discovered Agent (e.g. ZCode, registered only after sessions
    // were already collected) must be attributed to those historical sessions.
    // Re-link so the project detail page reflects every Agent that touched a project,
    // mirroring what `collect_usage_data` / `scan_projects` already do on demand.
    let device_id = settings.device_id.clone();
    let center_repo = settings.center_repo.clone();
    // P2-3: seed the built-in official example source on first run.
    let _ = db.ensure_official_example_source(&center_repo);
    drop(settings);
    db.link_sessions_to_projects(&device_id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_skills(state: State<'_, AppState>) -> Result<Vec<Skill>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_skills().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_agents(state: State<'_, AppState>) -> Result<Vec<Agent>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_agents().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_sync_targets(state: State<'_, AppState>) -> Result<Vec<SyncTarget>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_sync_targets().map_err(|e| e.to_string())
}

const DATA_MANAGEMENT_CONFIRM_CODE: &str = "DELETE";

#[tauri::command]
pub fn clear_collected_data(confirm: String, state: State<'_, AppState>) -> Result<(), String> {
    if confirm != DATA_MANAGEMENT_CONFIRM_CODE {
        return Err("确认码错误，操作已取消".to_string());
    }
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.clear_collected_data().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reset_database(confirm: String, state: State<'_, AppState>) -> Result<(), String> {
    if confirm != DATA_MANAGEMENT_CONFIRM_CODE {
        return Err("确认码错误，操作已取消".to_string());
    }
    let mut db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    db.reset_database(&device_id).map_err(|e| e.to_string())
}

/// Apply project-level skill bindings: for each binding, sync the skill to the
/// bound agent directory (reusing apply_sync_target). Bindings coexist with
/// global sync_targets per decision A in DECISIONS-implementation.md.
fn apply_project_bindings(db: &crate::db::Db, settings: &Settings) -> anyhow::Result<()> {
    let bindings = db.get_skill_project_bindings(&settings.device_id)?;
    if bindings.is_empty() {
        return Ok(());
    }
    let skills = db.get_skills()?;
    let agents = db.get_agents()?;
    for b in &bindings {
        if !b.is_enabled {
            continue;
        }
        // Resolve the skill.
        let skill = match skills.iter().find(|s| s.id == b.skill_id) {
            Some(s) => s,
            None => continue,
        };
        // If a specific agent is bound, sync to it; otherwise to all enabled agents.
        let target_agents: Vec<&Agent> = match &b.agent_id {
            Some(aid) => agents.iter().filter(|a| a.id == *aid).collect(),
            None => agents.iter().filter(|a| a.is_enabled).collect(),
        };
        for agent in target_agents {
            let mode = match b.mode.as_str() {
                "copy" => crate::models::SyncMode::Copy,
                "local_copy" => crate::models::SyncMode::Copy,
                _ => crate::models::SyncMode::Symlink,
            };
            let target = crate::models::SyncTarget {
                id: format!("binding:{}:{}", b.id, agent.id),
                skill_id: skill.id.clone(),
                skill_name: Some(skill.name.clone()),
                agent_id: agent.id.clone(),
                agent_name: Some(agent.name.clone()),
                mode,
                last_sync_at: None,
                status: crate::models::SyncStatus::CenterChanged,
            };
            let _ = crate::sync::apply_sync_target(&target, skill, agent);
        }
    }
    Ok(())
}

/// Best-effort KG extraction for a freshly created/added skill. Non-fatal on error.
/// (PRD-03 §1.4 / §3.1: extraction triggered on create, non-blocking.)
fn trigger_kg_analysis(db: &crate::db::Db, device_id: &str, skill: &Skill) {
    let md = skill.repo_path.join("SKILL.md");
    if md.exists() {
        if let Err(e) = crate::kg::analyze_skill(db, device_id, &skill.id, &md) {
            eprintln!("[kg] auto-analysis failed for {}: {}", skill.name, e);
        }
    }
}

/// Create missing sync targets so that every skill is synced to every enabled agent.
fn ensure_sync_targets(db: &crate::db::Db, settings: &Settings) -> anyhow::Result<()> {
    let skills = db.get_skills()?;
    let agents = db.get_agents()?;
    let targets = db.get_sync_targets()?;
    let existing: std::collections::HashSet<(String, String)> = targets
        .into_iter()
        .map(|t| (t.skill_id, t.agent_id))
        .collect();

    for skill in &skills {
        for agent in agents.iter().filter(|a| a.is_enabled) {
            if !existing.contains(&(skill.id.clone(), agent.id.clone())) {
                let target = SyncTarget {
                    id: new_id(),
                    skill_id: skill.id.clone(),
                    skill_name: Some(skill.name.clone()),
                    agent_id: agent.id.clone(),
                    agent_name: Some(agent.name.clone()),
                    mode: settings.default_sync_mode,
                    last_sync_at: None,
                    status: SyncStatus::CenterChanged,
                };
                db.insert_sync_target(&target)?;
                // Apply immediately so the agent gets the skill right away
                let _ = apply_sync_target_and_record(&db, &target, skill, agent);
            }
        }
    }

    Ok(())
}

fn update_tray_status(state: &AppState, conflict_count: usize) -> Result<(), String> {
    let tray_guard = state.tray.lock().map_err(|e| e.to_string())?;
    if let Some(tray) = tray_guard.as_ref() {
        let icon_name = if conflict_count > 0 {
            "tray-warning.png"
        } else {
            "tray-normal.png"
        };
        let icon_path = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default()
            .join("../Resources")
            .join(icon_name);
        if let Ok(image) = tauri::image::Image::from_path(&icon_path) {
            let _ = tray.set_icon(Some(image));
            let tooltip = if conflict_count > 0 {
                format!("SkillMint · {} 个冲突", conflict_count)
            } else {
                "SkillMint · 同步正常".to_string()
            };
            let _ = tray.set_tooltip(Some(&tooltip));
        }
    }
    Ok(())
}

/// Invalidate cached directory-skill scan results. `agent_id=None` clears the
/// entire cache (used after global sync or center-repo changes).
fn invalidate_directory_skill_cache(
    db: &crate::db::Db,
    agent_id: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(id) = agent_id {
        db.delete_agent_directory_skills(id)?;
    } else {
        db.clear_directory_skill_cache()?;
    }
    Ok(())
}

// PRD-09: built-in Markdown editor ---------------------------------------------------------------

/// SPEC-F2 T11: open the parent directory of a path in the default terminal.
#[tauri::command]
pub fn open_path_in_terminal(path: String, handle: AppHandle) -> Result<(), String> {
    let p = PathBuf::from(path);
    let dir = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
    handle
        .opener()
        .open_path(dir.to_string_lossy().to_string(), Some("Terminal"))
        .map_err(|e| format!("无法打开终端：{}", e))
}

/// Save frontmatter + body back to SKILL.md, optionally rename the skill directory,
/// and trigger knowledge-graph re-analysis asynchronously.
#[tauri::command]
pub fn save_skill_content(
    skill_id: String,
    frontmatter: serde_json::Value,
    body: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Skill, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    let mut skill = db
        .get_skill_by_id(&skill_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Skill not found".to_string())?;

    // Derive the new skill name from frontmatter if present; otherwise keep current.
    let new_name = frontmatter
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(&skill.name)
        .to_string();

    if new_name != skill.name {
        // Ensure target directory does not already exist (case-insensitive on macOS).
        let new_repo_path = settings.center_repo.join(&new_name);
        if new_repo_path.exists() {
            return Err(format!("已存在同名 Skill 目录「{}」", new_name));
        }
        // Move directory and update sync target agent symlinks/copies.
        let agents = db.get_agents().map_err(|e| e.to_string())?;
        let targets = db.get_sync_targets().map_err(|e| e.to_string())?;
        for target in targets.iter().filter(|t| t.skill_id == skill_id) {
            if let Some(agent) = agents.iter().find(|a| a.id == target.agent_id) {
                let old_agent_path = agent.skill_directory.join(&skill.name);
                if old_agent_path.exists() || old_agent_path.is_symlink() {
                    let _ = remove_path(&old_agent_path);
                }
            }
        }
        std::fs::rename(&skill.repo_path, &new_repo_path).map_err(|e| e.to_string())?;
        skill.repo_path = new_repo_path;
        skill.name = new_name.clone();
        // Note: sync_targets has no skill_name column — the name is derived
        // from a JOIN with skills.name, so updating the skill row below is
        // enough (P1-4 removed a broken update against that phantom column).
        // Re-create agent targets at the new name.
        let agents = db.get_agents().map_err(|e| e.to_string())?;
        let targets = db.get_sync_targets().map_err(|e| e.to_string())?;
        for target in targets.iter().filter(|t| t.skill_id == skill_id) {
            if let Some(agent) = agents.iter().find(|a| a.id == target.agent_id) {
                let _ = apply_sync_target_and_record(&db, target, &skill, agent);
            }
        }
    }

    // Detect external modification to avoid accidentally overwriting concurrent edits.
    // SPEC-F2 T10: callers may pass force=true to keep the in-memory version.
    let md_path = skill.repo_path.join("SKILL.md");
    if !force.unwrap_or(false) {
        let disk_mtime = std::fs::metadata(&md_path)
            .and_then(|m| m.modified())
            .ok();
        let db_mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(skill.updated_at);
        if let Some(disk) = disk_mtime {
            if disk > db_mtime + std::time::Duration::from_secs(2) {
                return Err(
                    "SKILL.md 已被外部编辑器修改，请刷新后重试。".to_string(),
                );
            }
        }
    }

    // G3-③: one-time safety snapshot before the first overwrite of `latest`.
    if let Err(e) = crate::sync::ensure_first_version_snapshot(&skill.repo_path) {
        eprintln!("[skillmint] failed to create first-version snapshot: {}", e);
    }

    // Write the file.
    let content = serialize_skill_md(&frontmatter, &body);
    std::fs::write(&md_path, content).map_err(|e| e.to_string())?;

    // Update DB metadata.
    let now = current_timestamp();
    skill.updated_at = now;
    db.update_skill(&skill).map_err(|e| e.to_string())?;

    // PRD-09: trigger KG analysis after save. We already hold the locks; run it
    // synchronously here because `analyze_skill` is bounded (~300ms per skill) and
    // returning the updated skill to the UI matters more than deferring.
    let device_id = settings.device_id.clone();
    trigger_kg_analysis(&db, &device_id, &skill);
    let _ = invalidate_directory_skill_cache(&db, None);

    Ok(skill)
}

fn parse_skill_md(raw: &str) -> (serde_json::Value, String) {
    let trimmed = raw.trim_start();
    if trimmed.starts_with("---") {
        // Find the closing --- after the opening delimiter.
        if let Some(end_idx) = raw[3..].find("\n---") {
            let yaml_text = raw[3..3 + end_idx].trim();
            let body_start = 3 + end_idx + 4; // after \n---
            let body = raw[body_start..].trim_start().to_string();
            let frontmatter = serde_yaml::from_str(yaml_text)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            return (frontmatter, body);
        }
    }
    (serde_json::Value::Object(Default::default()), raw.to_string())
}

fn serialize_skill_md(frontmatter: &serde_json::Value, body: &str) -> String {
    let empty = frontmatter.as_object().map(|o| o.is_empty()).unwrap_or(false);
    if empty {
        return body.to_string();
    }
    let yaml = serde_yaml::to_string(frontmatter).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, body.trim_start())
}

#[tauri::command]
pub fn open_skill_in_editor(
    skill_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let skill = db
        .get_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == skill_id)
        .ok_or("Skill not found")?;

    let skill_md = skill.repo_path.join("SKILL.md");
    app.opener()
        .open_path(skill_md.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(state_to_model(&settings))
}

#[tauri::command]
pub fn get_device_id(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.device_id.clone())
}

#[tauri::command]
pub fn collect_usage_data(state: State<'_, AppState>) -> Result<Vec<crate::models::CollectionStats>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    collect_usage_data_inner(&db, &device_id)
}

/// Internal version of `collect_usage_data` used by the scheduled-task registry.
pub(crate) fn collect_usage_data_inner(
    db: &crate::db::Db,
    device_id: &str,
) -> Result<Vec<crate::models::CollectionStats>, String> {
    collect_usage_data_inner_with_progress(db, device_id, |_source, _stats| {}, || false)
}

/// Version with a per-source progress callback. Used by the async background job.
pub(crate) fn collect_usage_data_inner_with_progress<F, S>(
    db: &crate::db::Db,
    device_id: &str,
    mut on_progress: F,
    should_stop: S,
) -> Result<Vec<crate::models::CollectionStats>, String>
where
    F: FnMut(&str, &crate::models::CollectionStats),
    S: Fn() -> bool,
{
    let now = current_timestamp();

    let mut all_stats = Vec::new();
    for c in crate::collector::discover_all_collectors() {
        if should_stop() {
            return Err("cancelled".to_string());
        }
        let source = c.source().to_string();
        let kind = c.collector_kind().to_string();
        let path = c.data_path();
        if !c.is_available() {
            let _ = db.upsert_collected_source(&crate::models::CollectedSource {
                source: source.clone(),
                collector_kind: kind,
                data_path: path,
                status: "not_found".to_string(),
                last_collected_at: Some(now),
                record_count: 0,
            });
            let stats = crate::models::CollectionStats {
                source: source.clone(),
                ..Default::default()
            };
            on_progress(&source, &stats);
            all_stats.push(stats);
            continue;
        }
        match c.collect(db, device_id) {
            Ok(stats) => {
                // PRD-05: cursor has no sessions — count its code-contribution rows instead.
                let count = if source == "cursor" {
                    db.count_collected_code_contributions(&source)
                        .unwrap_or(stats.sessions)
                } else {
                    db.count_collected_sessions(&source).unwrap_or(stats.sessions)
                };
                let _ = db.upsert_collected_source(&crate::models::CollectedSource {
                    source: source.clone(),
                    collector_kind: kind,
                    data_path: path,
                    status: "ok".to_string(),
                    last_collected_at: Some(now),
                    record_count: count,
                });
                // Best-effort cleanup of file-state rows for files that no longer exist.
                let _ = crate::collector::prune_missing_file_states(db, &source);
                on_progress(&source, &stats);
                all_stats.push(stats);
            }
            Err(e) => {
                // A collector failure must not block the others.
                eprintln!("[collector] {} failed: {}", source, e);
                let _ = db.upsert_collected_source(&crate::models::CollectedSource {
                    source: source.clone(),
                    collector_kind: kind,
                    data_path: path,
                    status: "error".to_string(),
                    last_collected_at: Some(now),
                    record_count: 0,
                });
                // Even on failure some files may have been processed; clean up stale states.
                let _ = crate::collector::prune_missing_file_states(db, &source);
                let stats = crate::models::CollectionStats {
                    source: source.clone(),
                    ..Default::default()
                };
                on_progress(&source, &stats);
                all_stats.push(stats);
            }
        }
    }
    // After all collectors run, link sessions to projects + upsert agent_instances.
    // (Attribution is done inline during Claude Code collection.)
    let _ = db.link_sessions_to_projects(device_id);
    Ok(all_stats)
}

#[tauri::command]
pub fn get_collection_status(state: State<'_, AppState>) -> Result<Vec<crate::models::CollectedSource>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_collected_sources().map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
struct CollectionProgressPayload {
    job_id: String,
    source: String,
    stats: crate::models::CollectionStats,
}

#[derive(Clone, serde::Serialize)]
struct CollectionCompletedPayload {
    job_id: String,
    stats: Vec<crate::models::CollectionStats>,
}

#[derive(Clone, serde::Serialize)]
struct CollectionFailedPayload {
    job_id: String,
    error: String,
}

/// Start a background collection job and return its id immediately.
/// Progress, completion and failure are emitted as Tauri events.
#[tauri::command]
pub fn start_collection_job(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    // P0: prevent concurrent collection jobs.
    if let Some(running) = db
        .list_recent_collection_jobs(1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|j| j.status == "running")
    {
        return Ok(running.id);
    }
    let job_id = crate::db::new_id();
    db.create_collection_job(&job_id, current_timestamp())
        .map_err(|e| e.to_string())?;
    drop(db);

    // Register a cancellation flag for this job.
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut flags = state.collection_cancel_flags.lock().map_err(|e| e.to_string())?;
        flags.insert(job_id.clone(), cancel_flag.clone());
    }

    let app_thread = app.clone();
    let job_id_thread = job_id.clone();
    std::thread::spawn(move || {
        let state = app_thread.state::<AppState>();
        let cancel_ref = {
            let flags = state.collection_cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
            flags.get(&job_id_thread).cloned()
        };
        let result = (|| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let device_id = {
                let s = state.settings.lock().map_err(|e| e.to_string())?;
                s.device_id.clone()
            };
            let flag = cancel_ref.clone();
            collect_usage_data_inner_with_progress(
                &db,
                &device_id,
                |source, stats| {
                    let payload = CollectionProgressPayload {
                        job_id: job_id_thread.clone(),
                        source: source.to_string(),
                        stats: stats.clone(),
                    };
                    let _ = app_thread.emit("collection:progress", payload);
                },
                move || flag.as_ref().map(|f| f.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(false),
            )
        })();

        let now = current_timestamp();
        let state = app_thread.state::<AppState>();
        {
            let mut flags = state.collection_cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
            flags.remove(&job_id_thread);
        }
        match result {
            Ok(stats) => {
                let result_json = serde_json::to_string(&stats).unwrap_or_default();
                let _ = (|| -> Result<(), String> {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    db.complete_collection_job(&job_id_thread, now, &result_json)
                        .map_err(|e| e.to_string())
                })();
                let payload = CollectionCompletedPayload {
                    job_id: job_id_thread.clone(),
                    stats,
                };
                let _ = app_thread.emit("collection:completed", payload);
            }
            Err(e) if e == "cancelled" => {
                let _ = (|| -> Result<(), String> {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    db.cancel_collection_job(&job_id_thread, now)
                        .map_err(|e| e.to_string())
                })();
                let payload = CollectionFailedPayload {
                    job_id: job_id_thread,
                    error: "任务已取消".to_string(),
                };
                let _ = app_thread.emit("collection:failed", payload);
            }
            Err(e) => {
                let _ = (|| -> Result<(), String> {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    db.fail_collection_job(&job_id_thread, now, &e)
                        .map_err(|e| e.to_string())
                })();
                let payload = CollectionFailedPayload {
                    job_id: job_id_thread,
                    error: e,
                };
                let _ = app_thread.emit("collection:failed", payload);
            }
        }
    });

    Ok(job_id)
}

#[tauri::command]
pub fn get_collection_job(id: String, state: State<'_, AppState>) -> Result<Option<crate::models::CollectionJob>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_collection_job(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_collection_job(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.cancel_collection_job(&id, current_timestamp())
        .map_err(|e| e.to_string())?;
    drop(db);
    if let Ok(flags) = state.collection_cancel_flags.lock() {
        if let Some(flag) = flags.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_recent_collection_jobs(
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::CollectionJob>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_recent_collection_jobs(limit).map_err(|e| e.to_string())
}

/// Test an ACP connection by id. Returns the agent's stdout / status text.
#[tauri::command]
pub fn test_acp_transport(id: String, state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    crate::acp::test_transport(&settings.ai.acp_connections, &id).map_err(|e| e.to_string())
}

/// P1-1: scan PATH for locally installed ACP-compatible agents.
#[tauri::command]
pub fn detect_local_agents() -> Result<Vec<crate::acp::DetectedAgent>, String> {
    Ok(crate::acp::detect_available_agents())
}

/// P0: list recent LLM/ACP request audit logs.
#[tauri::command]
pub fn list_llm_request_logs(
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::LlmRequestLog>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_llm_request_logs(limit).map_err(|e| e.to_string())
}

/// Test a configured AI model by id with a minimal chat request.
#[tauri::command]
pub fn test_ai_model(model_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    let cfg = &settings.ai;
    let model = cfg
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or("Model not found")?;
    if model.api_key.trim().is_empty() {
        return Err("该模型未配置 API Key".to_string());
    }
    if !model.capabilities.contains(&"chat".to_string()) {
        return Err("该模型不支持对话能力".to_string());
    }
    let reply = crate::llm::chat(
        cfg,
        "你是一个连接测试助手。请只回复：连接正常。",
        "测试连接，请只回复：连接正常。",
    );
    match reply {
        Some(text) if text.contains("连接正常") || text.to_lowercase().contains("ok") => Ok("连接正常".to_string()),
        Some(text) => Ok(text),
        None => Err("模型无响应，请检查网络与 API Key".to_string()),
    }
}

#[tauri::command]
pub fn get_agent_usage(
    source: String,
    days: u32,
    state: State<'_, AppState>,
) -> Result<crate::models::AgentUsageSummary, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_agent_usage_summary(&source, days, current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_agent_detail(
    agent_id: String,
    state: State<'_, AppState>,
) -> Result<crate::models::AgentDetail, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    let agent = db
        .get_agent_by_id(&agent_id)
        .map_err(|e| e.to_string())?
        .ok_or("Agent not found")?;
    let (last_used_at, project_count) = db.get_agent_usage_meta(&agent_id).map_err(|e| e.to_string())?;
    // PRD-06 §3.2: source is now a real column on agents — use it directly instead
    // of guessing from the agent name (`name.contains("Claude")`).
    let source = agent.source.clone();
    let usage_7d = match &source {
        Some(s) => db
            .get_agent_usage_summary(s, 7, current_timestamp())
            .map_err(|e| e.to_string())
            .ok(),
        None => None,
    };
    let projects = db
        .get_agent_projects(&device_id, &agent_id)
        .map_err(|e| e.to_string())?;
    // PRD-06 §3.3: list every directory owned by this agent (1:N).
    let directories = db
        .get_agent_directories(&agent_id)
        .unwrap_or_default();
    Ok(crate::models::AgentDetail {
        id: agent.id,
        name: agent.name,
        skill_directory: agent.skill_directory.to_string_lossy().to_string(),
        description: agent.description,
        is_enabled: agent.is_enabled,
        last_used_at,
        project_count,
        source,
        usage_7d,
        projects,
        directories,
    })
}

#[tauri::command]
pub fn get_projects_usage(
    days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::ProjectUsageSummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    db.get_project_usage_summary(&device_id, days, current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_skill_usage(
    days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::SkillUsageSummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_skill_usage_summary(days, current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_usage_timeseries(
    source: String,
    days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<(String, i64)>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_usage_timeseries(&source, days).map_err(|e| e.to_string())
}

/// PRD-08 §3.1: the full window-metrics payload (4 KPI values + 环比/同比
/// comparison table + Token/Prompt dimensions + leverage) for a window kind
/// and reference date. `source` filters to one Agent (empty string = all).
/// This is the single call that powers the upgraded Usage page.
#[tauri::command]
pub fn get_window_metrics(
    kind: String,
    ref_date: String,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::models::WindowMetrics, String> {
    let kind = crate::window::normalize_kind(&kind)?;
    let ref_date = chrono::NaiveDate::parse_from_str(&ref_date, "%Y-%m-%d")
        .map_err(|e| format!("invalid ref_date (expected YYYY-MM-DD): {e}"))?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    // Build the pricing resolver from the seeded `digest_model` rows; falls back
    // to the builtin constant when the table is empty/missing.
    let pricing = crate::pricing::Pricing::from_db(
        db.load_model_pricing().unwrap_or_default(),
    );
    let src_filter = source.as_deref().filter(|s| !s.is_empty());
    crate::window::window_metrics(&db, kind, ref_date, src_filter, &pricing)
        .map_err(|e| e.to_string())
}

/// PRD-08 §3.3 (P1): classify the four semantic axes of unclassified prompts
/// via the configured LLM. Returns counts; with no API key, reports `skipped`.
#[tauri::command]
pub fn classify_prompts(
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::classifier::ClassifyResult, String> {
    let cfg = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.ai.clone()
    };
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let src = source.as_deref().filter(|s| !s.is_empty());
    crate::classifier::classify_prompts(&db, src, &cfg).map_err(|e| e.to_string())
}

/// PRD-08 §3.6 (P1): generate (or re-generate) the LLM daily summary for a date.
/// Returns the structured outcome — generated summary, no-key, no-sessions, or
/// failed — so the UI can show a tailored message.
#[tauri::command]
pub fn generate_daily_summary(
    date: String,
    state: State<'_, AppState>,
) -> Result<crate::analyzer::Outcome, String> {
    let cfg = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.ai.clone()
    };
    let db = state.db.lock().map_err(|e| e.to_string())?;
    crate::analyzer::generate_daily_summary(&db, &date, &cfg).map_err(|e| e.to_string())
}

/// PRD-08 §3.6 (P1): load a cached daily summary for a date (if generated
/// before), without calling the LLM. Powers the summary panel on open.
#[tauri::command]
pub fn get_daily_summary(
    date: String,
    state: State<'_, AppState>,
) -> Result<Option<crate::analyzer::DailySummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_daily_summary(&date).map_err(|e| e.to_string())
}

/// PRD-11: list cached daily summaries within a date range (inclusive),
/// ordered by date descending. Powers the date list in the AI daily-summary
/// value module.
#[tauri::command]
pub fn list_daily_summaries(
    start_date: String,
    end_date: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::DailySummaryMeta>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_daily_summaries(&start_date, &end_date)
        .map_err(|e| e.to_string())
}

/// PRD-11: compute aggregate value metrics from cached daily summaries.
#[tauri::command]
pub fn get_summary_value_metrics(
    state: State<'_, AppState>,
) -> Result<crate::models::SummaryValueMetrics, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_summary_value_metrics().map_err(|e| e.to_string())
}

/// PRD-08 §3.8 (P2): detect an AI-Digest `~/.digest/digest.db` and return its
/// path + row counts for the import preview UI. Returns null when absent.
#[tauri::command]
pub fn detect_digest_db() -> Result<Option<DigestDbPreview>, String> {
    Ok(crate::db::Db::detect_digest_db().map(|(path, stats)| DigestDbPreview {
        path: path.to_string_lossy().to_string(),
        sessions: stats.sessions,
        prompts: stats.prompts,
        token_usage: stats.token_usage,
    }))
}

/// PRD-08 §3.8 (P2): import an AI-Digest digest.db (v1 tables) into the local
/// `collected_*` tables. Idempotent merge; imported rows keep AI-Digest's
/// four-axis prompt labels. Returns counts of inserted rows.
#[tauri::command]
pub fn import_digest_db(state: State<'_, AppState>) -> Result<crate::db::DigestImportSummary, String> {
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    // Read the external digest.db WITHOUT holding the DB mutex, so the UI and
    // other commands stay responsive during the (potentially large) import.
    let data = crate::db::Db::read_digest_db().map_err(|e| e.to_string())?;
    let mut db = state.db.lock().map_err(|e| e.to_string())?;
    db.import_digest_data(&device_id, data).map_err(|e| e.to_string())
}

/// Preview payload for `detect_digest_db` (path + counts).
#[derive(Debug, serde::Serialize)]
pub struct DigestDbPreview {
    pub path: String,
    pub sessions: i64,
    pub prompts: i64,
    pub token_usage: i64,
}

/// PRD-08 §3.4 (P3): heatmap cell drill-down. Returns the classified prompts that
/// back a given (source, action) cell in the current window. `action` empty = the
/// whole tool row. Powers the role-profile cell-click → prompt list drawer.
#[tauri::command]
pub fn get_prompts_for_cell(
    kind: String,
    ref_date: String,
    source: String,
    action: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::db::CellPrompt>, String> {
    let kind = crate::window::normalize_kind(&kind)?;
    let ref_date = chrono::NaiveDate::parse_from_str(&ref_date, "%Y-%m-%d")
        .map_err(|e| format!("invalid ref_date (expected YYYY-MM-DD): {e}"))?;
    let (start, end) = crate::window::window_bounds_for(kind, ref_date);
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.query_prompts_for_cell(start, end, &source, action.as_deref(), 200)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_skill_health(
    skill_name: String,
    days: u32,
    state: State<'_, AppState>,
) -> Result<(f64, String), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_skill_health(&skill_name, days, current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(
    new_settings: AppSettings,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<AppSettings, String> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?;
    std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;

    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;

    // P3-6: center repo / sync-mode / scope-mode settings retired — the skills
    // library is driven by the npx locks + private hubs now. The remaining
    // internal `settings.center_repo` only serves dormant legacy commands.
    settings.auto_sync_interval_minutes = new_settings.auto_sync_interval_minutes;
    settings.launch_at_login = new_settings.launch_at_login;
    settings.show_dock_icon = new_settings.show_dock_icon;
    settings.onboarding_completed = new_settings.onboarding_completed;
    settings.remote_enabled = new_settings.remote_enabled;
    settings.theme = new_settings.theme;
    // SECURITY: persist each model's API key to the keyring, never to settings.json.
    // Keep the user-supplied keys in memory so the UI echoes them back immediately.
    for model in &new_settings.ai.models {
        crate::settings::AiConfig::write_key(&model.id, &model.api_key);
    }
    settings.ai = new_settings.ai;
    // P3: npx skills integration settings.
    settings.npx_package = if new_settings.npx_package.trim().is_empty() {
        "skills@latest".to_string()
    } else {
        new_settings.npx_package.trim().to_string()
    };
    settings.skills_api_url = new_settings.skills_api_url;
    settings.proxy_env = new_settings.proxy_env;
    settings.disable_telemetry = new_settings.disable_telemetry;
    settings.node_path_override = new_settings.node_path_override;
    // device_id is server-owned and read-only here: ignore whatever the frontend sent.

    settings.save(&app_dir).map_err(|e| e.to_string())?;

    // Auto-sync interval may have changed: restart the background scheduler so
    // the new cadence (or 0 = disabled) takes effect immediately (PRD-0 §4.7).
    crate::restart_sync_scheduler(&app);

    Ok(state_to_model(&settings))
}

#[tauri::command]
pub fn get_conflict_contents(
    sync_target_id: String,
    state: State<'_, AppState>,
) -> Result<ConflictContent, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let target = db
        .get_sync_target_by_id(&sync_target_id)
        .map_err(|e| e.to_string())?
        .ok_or("Sync target not found")?;

    let skill = db
        .get_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == target.skill_id)
        .ok_or("Skill not found")?;

    let agent = db
        .get_agents()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == target.agent_id)
        .ok_or("Agent not found")?;

    let center_md = skill.repo_path.join("SKILL.md");
    let agent_md = agent.skill_directory.join(&skill.name).join("SKILL.md");

    let center_content = std::fs::read_to_string(&center_md).unwrap_or_default();
    let local_content = std::fs::read_to_string(&agent_md).unwrap_or_default();

    let center_updated_at = file_mtime_secs(&center_md).or(Some(skill.updated_at));
    let local_updated_at = file_mtime_secs(&agent_md);

    Ok(ConflictContent {
        sync_target_id,
        skill_name: skill.name,
        agent_name: agent.name,
        center_content,
        local_content,
        center_updated_at,
        local_updated_at,
        center_author: None,
        local_author: None,
    })
}

fn file_mtime_secs(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[tauri::command]
pub fn resolve_conflict(
    payload: ConflictPayload,
    state: State<'_, AppState>,
) -> Result<SyncTarget, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let target = db
        .get_sync_target_by_id(&payload.sync_target_id)
        .map_err(|e| e.to_string())?
        .ok_or("Sync target not found")?;

    let skill = db
        .get_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == target.skill_id)
        .ok_or("Skill not found")?;

    let agent = db
        .get_agents()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == target.agent_id)
        .ok_or("Agent not found")?;

    match payload.resolution {
        ConflictResolution::KeepCenter => {
            apply_sync_target_and_record(&db, &target, &skill, &agent).map_err(|e| e.to_string())?;
        }
        ConflictResolution::KeepLocal => {
            let agent_path = agent.skill_directory.join(&skill.name);
            // Stage the agent copy next to the center repo, then atomically swap it
            // in so a crash never leaves the center repo half-written.
            let staging = skill.repo_path.with_extension("skillmint-staging");
            if staging.exists() || staging.is_symlink() {
                let _ = remove_path(&staging);
            }
            copy_dir_all(&agent_path, &staging).map_err(|e| e.to_string())?;
            crate::fs::replace_path_atomic(&staging, &skill.repo_path).map_err(|e| e.to_string())?;
            apply_sync_target_and_record(&db, &target, &skill, &agent).map_err(|e| e.to_string())?;
        }
        ConflictResolution::Skip => {
            // do nothing
        }
    }

    if payload.resolution != ConflictResolution::Skip {
        let _ = invalidate_directory_skill_cache(&db, Some(&target.agent_id));
    }

    let result = db
        .get_sync_target_by_id(&target.id)
        .map_err(|e| e.to_string())?
        .ok_or("Sync target disappeared".to_string())?;

    // Update tray status after conflict resolution
    let targets = db.get_sync_targets().map_err(|e| e.to_string())?;
    let conflict_count = targets
        .iter()
        .filter(|t| t.status == SyncStatus::Conflict)
        .count();
    update_tray_status(&state, conflict_count)?;

    Ok(result)
}

// =============================================================================
// PRD-01: project-level skill management
// =============================================================================

#[tauri::command]
pub fn get_projects(state: State<'_, AppState>) -> Result<Vec<crate::models::ProjectUsageSummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    db.get_project_usage_summary(&device_id, 0, current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scan_projects(state: State<'_, AppState>) -> Result<usize, String> {
    // Project discovery happens during collection; this re-runs the linking step.
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    db.link_sessions_to_projects(&device_id)
        .map(|n| n as usize)
        .map_err(|e| e.to_string())
}

// =============================================================================
// PRD-01 patch: project detail page + multi-version skill management (FR-A..F)
// =============================================================================

// --- pure core helpers (testable without a Tauri runtime) --------------------

/// Build the project detail payload. Core logic of `get_project_detail`.
pub fn build_project_detail(
    db: &crate::db::Db,
    device_id: &str,
    project_id: &str,
) -> Result<ProjectDetail, anyhow::Error> {
    let (name, path, last_active_at): (String, String, Option<u64>) = db
        .conn_ref()
        .query_row(
            "SELECT name, path, last_active_at FROM projects WHERE id = ?1 AND device_id = ?2",
            rusqlite::params![project_id, device_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, Option<i64>>(2)?.map(|t| t as u64),
                ))
            },
        )?;

    let mut agents = db.get_project_agents(device_id, project_id)?;
    for entry in agents.iter_mut() {
        let dir = std::path::PathBuf::from(&entry.skill_directory);
        entry.skill_count = Some(
            std::fs::read_dir(&dir)
                .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count() as i64)
                .unwrap_or(0),
        );
    }
    let (session_count, total_tokens) = db.get_project_aggregate(device_id, project_id)?;

    Ok(ProjectDetail {
        project_id: project_id.to_string(),
        name,
        path,
        last_active_at,
        session_count,
        total_tokens,
        agents,
    })
}

/// Resolve one skill's physical nature + registration. Core logic of
/// `resolve_skill_link_command`.
pub fn build_resolved_skill(
    db: &crate::db::Db,
    device_id: &str,
    settings: &Settings,
    project_id: &str,
    agent_id: &str,
    skill_name: &str,
) -> Result<ResolvedSkill, anyhow::Error> {
    let agent = db
        .get_agents()?
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

    // Look up the binding first so resolve_skill_link compares against the
    // pinned version when appropriate (FR-F).
    let binding = db
        .get_skill_project_bindings(device_id)?
        .into_iter()
        .find(|b| {
            b.project_id.as_deref() == Some(project_id)
                && b.agent_id.as_deref() == Some(agent_id)
                && b.skill_name.as_deref() == Some(skill_name)
        });
    let pinned_version = binding.as_ref().and_then(|b| b.pinned_version.as_deref());

    let mut resolved = resolve_skill_link(
        &agent.skill_directory,
        skill_name,
        &settings.center_repo,
        &settings.project_skill_dir_name,
        pinned_version,
    )?;

    if let Some(b) = binding {
        resolved.is_registered = true;
        resolved.pinned_version = b.pinned_version.clone();
    }
    Ok(resolved)
}

#[tauri::command]
pub fn get_project_detail(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<ProjectDetail, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    build_project_detail(&db, &device_id, &project_id).map_err(|e| e.to_string())
}

/// Core logic of `resolve_skill_diff_command`.
///
/// `strategy` is the raw string from the frontend (`keep_center` / `keep_project` /
/// `keep_project_backup` / `versionize`); `keep_project_backup` is KeepProject with
/// `backup=true` (PRD §4.5c 留底, DECISIONION 16). `note` attaches to a Versionize
/// snapshot sidecar (PRD §4.5c).
pub fn resolve_diff_core(
    db: &crate::db::Db,
    binding_id: &str,
    strategy: &str,
    note: Option<&str>,
) -> Result<ResolveResult, anyhow::Error> {
    let (enum_strategy, backup) = match strategy {
        "keep_center" => (DiffStrategy::KeepCenter, false),
        "keep_project" => (DiffStrategy::KeepProject, false),
        "keep_project_backup" => (DiffStrategy::KeepProject, true),
        "versionize" => (DiffStrategy::Versionize, false),
        other => anyhow::bail!("unknown strategy: {}", other),
    };

    let binding = db
        .get_skill_project_binding(binding_id)?
        .ok_or_else(|| anyhow::anyhow!("Binding not found"))?;
    let skill = db
        .get_skill_by_id(&binding.skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;
    let agent_id = binding
        .agent_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Binding has no agent; cannot resolve a physical diff"))?;
    let agent = db
        .get_agents()?
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

    let project_skill_path = agent.skill_directory.join(&skill.name);
    let outcome = crate::sync::resolve_skill_diff(
        enum_strategy,
        &skill,
        &project_skill_path,
        &agent,
        db,
        note,
        backup,
    )?;

    match enum_strategy {
        DiffStrategy::Versionize => {
            if let Some(ref v) = outcome.new_version {
                db.set_binding_pinned_version(binding_id, Some(v))?;
            }
        }
        _ => {
            // Merge strategies follow latest (clear pin).
            db.set_binding_pinned_version(binding_id, None)?;
        }
    }

    Ok(ResolveResult {
        strategy: strategy.to_string(),
        new_version: outcome.new_version,
        project_path: outcome.project_dir.to_string_lossy().to_string(),
        latest_path: outcome.latest_dir.to_string_lossy().to_string(),
    })
}

/// Core logic of `pin_binding_version` (testable without Tauri runtime).
pub fn pin_binding_version_core(
    db: &crate::db::Db,
    binding_id: &str,
    version: Option<&str>,
) -> anyhow::Result<SkillProjectBinding> {
    let binding = db
        .get_skill_project_binding(binding_id)?
        .ok_or_else(|| anyhow::anyhow!("Binding not found"))?;

    let skill = db
        .get_skill_by_id(&binding.skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;

    let agent_id = binding
        .agent_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Binding has no agent"))?;
    let agent = db
        .get_agent_by_id(&agent_id)?
        .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

    let skill_root = PathBuf::from(&skill.repo_path);
    let source_dir = crate::fs::get_effective_skill_dir(&skill_root, version);
    let source_dir = if source_dir.exists() {
        source_dir
    } else {
        crate::fs::get_effective_skill_dir(&skill_root, None)
    };

    let project_copy = agent.skill_directory.join(&skill.name);
    match binding.mode.as_str() {
        "reference" => { /* no physical copy to move */ }
        "local_copy" => {
            if project_copy.exists() || project_copy.is_symlink() {
                remove_path(&project_copy)?;
            }
            copy_dir_all(&source_dir, &project_copy)?;
        }
        _ => {
            // symlink (default)
            if project_copy.exists() || project_copy.is_symlink() {
                remove_path(&project_copy)?;
            }
            crate::fs::create_symlink_or_copy(&source_dir, &project_copy)?;
        }
    }

    db.set_binding_pinned_version(binding_id, version)?;

    db.get_skill_project_binding(binding_id)?
        .ok_or_else(|| anyhow::anyhow!("Binding disappeared after pin"))
}

/// Core logic of `delete_skill_version`.
pub fn delete_skill_version_core(
    db: &crate::db::Db,
    skill_id: &str,
    version: &str,
) -> anyhow::Result<()> {
    if version == "latest" {
        anyhow::bail!("不能删除 latest 版本");
    }
    let pinned = db.get_pinned_projects_for_version(skill_id, version)?;
    if !pinned.is_empty() {
        anyhow::bail!(
            "版本 {} 仍被以下项目固定：{}，请先迁移或解固。",
            version,
            pinned.join("、")
        );
    }

    let skill = db
        .get_skill_by_id(skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;
    let version_dir = PathBuf::from(&skill.repo_path).join(version);
    if version_dir.exists() {
        remove_path(&version_dir)?;
    }
    Ok(())
}

/// Core logic of `set_version_note`.
pub fn set_version_note_core(
    db: &crate::db::Db,
    skill_id: &str,
    version: &str,
    note: Option<&str>,
) -> anyhow::Result<()> {
    let skill = db
        .get_skill_by_id(skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;
    let version_dir = PathBuf::from(&skill.repo_path).join(version);
    if !version_dir.exists() {
        anyhow::bail!("版本目录不存在：{}", version_dir.display());
    }
    crate::fs::write_version_note(&version_dir, note)
}

/// Core logic of `get_version_note`.
pub fn get_version_note_core(
    db: &crate::db::Db,
    skill_id: &str,
    version: &str,
) -> anyhow::Result<Option<String>> {
    let skill = db
        .get_skill_by_id(skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;
    let note_path = PathBuf::from(&skill.repo_path)
        .join(version)
        .join(crate::fs::VERSION_NOTE_FILE);
    if !note_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&note_path)?;
    let trimmed = content.trim().to_string();
    Ok(if trimmed.is_empty() { None } else { Some(trimmed) })
}

// =============================================================================
// PRD-01 §3.2c/§7.2: project state classification + context-menu actions
// =============================================================================

#[tauri::command]
pub fn classify_projects(state: State<'_, AppState>) -> Result<i64, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.classify_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_project_in_finder(path: String, app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_project_in_terminal(path: String) -> Result<(), String> {
    // macOS: open a Terminal at the path via AppleScript.
    std::process::Command::new("osascript")
        .args(["-e", &format!("tell application \"Terminal\" to do script \"cd {}\"", path)])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn remove_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_project(&id).map_err(|e| e.to_string())
}

// =============================================================================
// PRD-02: prompt-driven skill generation + reports
// =============================================================================

#[tauri::command]
pub fn get_high_value_prompts(
    min_repeat: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::HighValuePrompt>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let min = min_repeat.unwrap_or(3);
    db.get_high_value_prompts(min, 20).map_err(|e| e.to_string())
}

/// SPEC-F2 T1: quality gate for turning a prompt into a Skill.
/// Returns the suggested skill name, or a user-facing reason why it should be rejected.
pub(crate) fn validate_prompt_for_skill(prompt_text: &str) -> Result<String, String> {
    let trimmed = prompt_text.trim();

    // 1. Placeholder checks: starts with an attachment placeholder or placeholders dominate.
    let lower = trimmed.to_lowercase();
    let placeholder_prefixes = ["[image:", "[file:", "[attachment:"];
    for prefix in &placeholder_prefixes {
        if lower.starts_with(prefix) {
            return Err("此 Prompt 内容无法沉淀为 Skill（包含图片/文件/附件占位符）".to_string());
        }
    }

    // Compute placeholder coverage: sum of `[Image:...]`, `[File:...]`, `[Attachment:...]` blocks.
    let mut placeholder_len = 0usize;
    let mut chars = trimmed.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c == '[' {
            if let Some((end, _)) = trimmed[start..].find(']').map(|i| (start + i, ']')) {
                let inner = &trimmed[start + 1..end];
                let inner_lower = inner.to_lowercase();
                if inner_lower.starts_with("image:")
                    || inner_lower.starts_with("file:")
                    || inner_lower.starts_with("attachment:")
                {
                    placeholder_len += end - start + 1;
                }
            }
        }
    }
    let total_len = trimmed.chars().count();
    if total_len > 0 && placeholder_len * 100 > total_len * 50 {
        return Err("此 Prompt 内容无法沉淀为 Skill（占位符占比过高）".to_string());
    }

    // 2. Length check after whitespace removal.
    let without_space: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if without_space.len() < 20 {
        return Err("此 Prompt 内容无法沉淀为 Skill（内容过短，无法提取有效语义）".to_string());
    }

    // 3. Must contain at least one Chinese or English letter.
    let has_letter = trimmed.chars().any(|c| {
        c.is_alphabetic() && (c.is_ascii_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&c))
    });
    if !has_letter {
        return Err(
            "此 Prompt 内容无法沉淀为 Skill（未包含有效文字，仅由符号、数字或链接组成）"
                .to_string(),
        );
    }

    // Suggested name: strip placeholders and punctuation, tokenize, kebab-case.
    let mut cleaned = String::new();
    for ch in trimmed.chars() {
        if ch.is_alphanumeric()
            || ch == '-'
            || ch == '_'
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
        {
            cleaned.push(ch);
        } else {
            cleaned.push(' ');
        }
    }
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let name = tokens.join("-");
    let name = name.chars().take(48).collect::<String>();
    // Trim trailing hyphen if the 48-char cut landed on a delimiter.
    let name = name.trim_end_matches('-').to_string();
    if name.is_empty() || !name.chars().any(|c| c.is_alphabetic()) {
        return Err("此 Prompt 内容无法沉淀为 Skill（无法提取有效 Skill 名称）".to_string());
    }
    Ok(name)
}

/// SPEC-C1 T4: record a `rule_gate` rejection in the gate_rejections ledger.
/// Called when `generate_skill_from_prompt` fails structural validation so the
/// inbox can surface "why did this prompt never become a Skill?".
pub(crate) fn record_rule_gate_rejection(
    db: &crate::db::Db,
    prompt_text: &str,
    reason: &str,
) {
    let _ = db.insert_gate_rejection(
        "rule_gate",
        "high_value_prompt",
        0.5, // rule-gate rejections carry no detector confidence; midpoint
        &serde_json::json!({
            "prompt_preview": prompt_text.chars().take(120).collect::<String>(),
            "reason": reason,
        }),
    );
}

/// SPEC-F3: produce a structured description from a prompt.
/// - One-sentence purpose.
/// - One applicable scenario line.
/// Trims to a readable length without copying the raw prompt verbatim.
pub(crate) fn generate_skill_description(prompt_text: &str) -> String {
    let sentence: String = prompt_text
        .chars()
        .take(120)
        .collect::<String>()
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_string();
    let scenario = if sentence.len() >= prompt_text.chars().count() {
        "适用于从该 prompt 直接复现的工作流。".to_string()
    } else {
        format!("适用于「{}…」等相似场景。", sentence.chars().take(40).collect::<String>())
    };
    format!("{}\n\n适用场景：{}", sentence, scenario)
}

#[tauri::command]
/// Internal creation helper for the discovery-adoption pipeline (PRD-02/PRD-12:
/// Inbox + Usage still author through the legacy `skills` store). NOT a Tauri
/// command since P3-6b — the GUI authoring path is `hub_create_skill` (P3-5).
/// Migration of this pipeline into the private hubs is tracked as P3-6c.
pub fn create_skill_internal(
    name: String,
    agent_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Skill, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    let skill_path = settings.center_repo.join(&name);
    std::fs::create_dir_all(&skill_path).map_err(|e| e.to_string())?;

    let skill_md = skill_path.join("SKILL.md");
    if !skill_md.exists() {
        let template = format!("# {}\n\n## Description\n\nDescribe what this Skill does.\n\n## Usage\n\nExplain how Agent should use it.\n", name);
        std::fs::write(&skill_md, template).map_err(|e| e.to_string())?;
    }

    let now = current_timestamp();
    let skill = Skill {
        id: new_id(),
        name,
        repo_path: skill_path,
        created_at: now,
        updated_at: now,
        status: crate::models::SkillStatus::Draft,
    };

    db.insert_skill(&skill).map_err(|e| e.to_string())?;

    let agents = db.get_agents().map_err(|e| e.to_string())?;
    for agent_id in agent_ids {
        if let Some(agent) = agents.iter().find(|a| a.id == agent_id) {
            let target = SyncTarget {
                id: new_id(),
                skill_id: skill.id.clone(),
                skill_name: Some(skill.name.clone()),
                agent_id: agent.id.clone(),
                agent_name: Some(agent.name.clone()),
                mode: settings.default_sync_mode,
                last_sync_at: None,
                status: SyncStatus::CenterChanged,
            };
            db.insert_sync_target(&target).map_err(|e| e.to_string())?;
            let _ = apply_sync_target_and_record(&db, &target, &skill, agent).map_err(|e| e.to_string())?;
        }
    }

    // PRD-03: auto-trigger knowledge-graph extraction on skill creation.
    trigger_kg_analysis(&db, &settings.device_id, &skill);
    let _ = invalidate_directory_skill_cache(&db, None);
    Ok(skill)
}

#[tauri::command]
pub fn generate_skill_from_prompt(
    prompt_text: String,
    agent_ids: Option<Vec<String>>,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Skill, String> {
    // SPEC-C1 T4: record rule-gate rejections. When the prompt fails the
    // structural validation we log it as a `rule_gate` gate_rejection so the
    // inbox can explain why a candidate never became a Skill.
    let name = validate_prompt_for_skill(&prompt_text).map_err(|err| {
        if let Ok(db) = state.db.lock() {
            record_rule_gate_rejection(&db, &prompt_text, &err);
        }
        err
    })?;

    let db = state.db.lock().map_err(|e| e.to_string())?;
    // PRD-02 §3.3 c: handle "already exists" conflict — return error instead of silent overwrite.
    if db.get_skill_by_name(&name).map_err(|e| e.to_string())?.is_some() {
        return Err(format!(
            "已存在同名 Skill「{}」，请改名或查看已有 Skill。",
            name
        ));
    }
    drop(db); // release lock before calling create_skill (which re-locks)

    let prompt_trimmed = prompt_text.trim();
    let desc = generate_skill_description(prompt_trimmed);
    let agents = agent_ids.unwrap_or_default();
    let skill = create_skill_internal(name, agents, state.clone())?;
    // Overwrite the templated SKILL.md with the prompt-derived description.
    let skill_md = skill.repo_path.join("SKILL.md");
    let body = format!(
        "# {}\n\n## Description\n\n{}\n\n## Usage\n\nDerived from a repeated prompt.\n",
        skill.name, desc
    );
    let _ = std::fs::write(&skill_md, body);

    // PRD-01: optionally bind to a project.
    if let Some(pid) = project_id {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        let binding = crate::models::SkillProjectBinding {
            id: new_id(),
            device_id: settings.device_id.clone(),
            skill_id: skill.id.clone(),
            skill_name: Some(skill.name.clone()),
            project_id: Some(pid),
            project_name: None,
            agent_id: None,
            mode: "reference".to_string(),
            local_path: None,
            is_enabled: true,
            pinned_version: None,
        };
        db.upsert_skill_project_binding(&binding).map_err(|e| e.to_string())?;
    }
    Ok(skill)
}

/// SPEC-F5 T3: preview the skill name + description that would be generated
/// from a prompt, without persisting anything. Returns the tokenized name
/// (reusing `validate_prompt_for_skill`) and the description (reusing
/// `generate_skill_description`). On validation failure returns Err with the
/// user-facing reason, so the sediment dialog can show why and disable confirm.
#[derive(Debug, serde::Serialize)]
pub struct SkillPromptPreview {
    pub name: String,
    pub description: String,
}

#[tauri::command]
pub fn preview_skill_from_prompt(prompt_text: String) -> Result<SkillPromptPreview, String> {
    let name = validate_prompt_for_skill(&prompt_text)?;
    let description = generate_skill_description(prompt_text.trim());
    Ok(SkillPromptPreview { name, description })
}

#[tauri::command]
pub fn get_skill_suggestion_report(state: State<'_, AppState>) -> Result<crate::models::SkillSuggestionReport, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let top_prompts = db.get_high_value_prompts(3, 10).map_err(|e| e.to_string())?;
    // Underused: skills with 0 attributions; top: skills with most attributions.
    let usage = db.get_skill_usage_summary(0, current_timestamp()).map_err(|e| e.to_string())?;
    let underused_skills = usage
        .iter()
        .filter(|u| u.usage_count == 0)
        .map(|u| u.skill_name.clone())
        .collect();
    let top_skills = usage
        .iter()
        .filter(|u| u.usage_count > 0)
        .take(5)
        .map(|u| u.skill_name.clone())
        .collect();
    Ok(crate::models::SkillSuggestionReport {
        generated_at: current_timestamp(),
        top_prompts,
        underused_skills,
        top_skills,
    })
}

// =============================================================================
// SPEC-I2: discovery inbox + weekly reports
// =============================================================================

static DISCOVERY_RUNNING: OnceLock<Mutex<bool>> = OnceLock::new();

fn discovery_running() -> &'static Mutex<bool> {
    DISCOVERY_RUNNING.get_or_init(|| Mutex::new(false))
}

/// Run the discovery pipeline now. Serialized via a static Mutex so repeated
/// manual triggers get a clear "已在运行" response instead of stacking up.
#[tauri::command]
pub fn run_discovery_pipeline(state: State<'_, AppState>) -> Result<DiscoveryRunResult, String> {
    let mut guard = discovery_running().lock().map_err(|e| e.to_string())?;
    if *guard {
        return Err("发现管线已在运行".to_string());
    }
    *guard = true;
    drop(guard);

    let result = (|| {
        let mut db = state.db.lock().map_err(|e| e.to_string())?;
        let cfg = {
            let settings = state.settings.lock().map_err(|e| e.to_string())?;
            settings.ai.clone()
        };
        crate::discovery::run_pipeline(&mut db, "", &cfg).map_err(|e| e.to_string())
    })();

    let mut guard = discovery_running().lock().map_err(|e| e.to_string())?;
    *guard = false;

    result
}

#[tauri::command]
pub fn list_discoveries(
    status: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Discovery>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_discoveries(status.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn decide_discovery(
    id: String,
    action: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<DiscoveryDecisionResult, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let (device_id, center_repo) = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        (settings.device_id.clone(), settings.center_repo.clone())
    };
    crate::discovery::decide_discovery(&db, &device_id, &center_repo, &id, &action, reason.as_deref())
        .map_err(|e| e.to_string())
}

/// SPEC-C1 T4: list gate-rejection ledger rows. Pass `reason` to filter
/// (e.g. `"below_threshold"` for the low-confidence band) and `limit` to cap.
#[tauri::command]
pub fn list_gate_rejections(
    reason: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::GateRejection>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_gate_rejections(reason.as_deref(), limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_growth_metrics(state: State<'_, AppState>) -> Result<GrowthMetrics, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let (device_id, now) = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        (settings.device_id.clone(), current_timestamp())
    };
    compute_growth_metrics(&db, &device_id, now).map_err(|e| e.to_string())
}

fn compute_growth_metrics(
    db: &crate::db::Db,
    device_id: &str,
    now: u64,
) -> anyhow::Result<GrowthMetrics> {
    use crate::models::{ActiveSkillMetric, CapabilityCategory, CompoundingCurve};
    use std::collections::BTreeMap;

    // 1. Skill count by source: from discovery vs hand-written vs remote.
    let mut skill_count_by_source: BTreeMap<String, i64> = BTreeMap::new();
    skill_count_by_source.insert("handwritten".to_string(), 0);
    skill_count_by_source.insert("from_discovery".to_string(), 0);
    skill_count_by_source.insert("remote".to_string(), 0);

    let accepted = db.list_discoveries(Some("accepted"))?;
    let accepted_dedup: std::collections::HashSet<String> =
        accepted.iter().map(|d| d.dedup_key.clone()).collect();
    for skill in db.get_skills()? {
        let source = if accepted_dedup.contains(&format!("skill: {}", skill.name)) {
            "from_discovery"
        } else if skill.repo_path.to_string_lossy().contains("sources") {
            "remote"
        } else {
            "handwritten"
        };
        *skill_count_by_source.entry(source.to_string()).or_insert(0) += 1;
    }

    // 2. Compounding curves: for each Skill from a repeat-pattern discovery,
    //    count occurrences week-by-week before and after creation.
    let mut compounding_curves = Vec::new();
    let repeat_accepted: Vec<_> = accepted
        .iter()
        .filter(|d| matches!(d.kind, crate::models::DiscoveryKind::RepeatPattern))
        .collect();
    for d in repeat_accepted {
        let skill_id = match &d.resulting_skill_id {
            Some(id) => id,
            None => continue,
        };
        let skill = match db.get_skill_by_id(skill_id)? {
            Some(s) => s,
            None => continue,
        };

        let pattern = d
            .payload
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if pattern.is_empty() {
            continue;
        }

        let mut weeks: Vec<String> = Vec::new();
        let mut counts: Vec<i64> = Vec::new();
        // Last 8 weeks.
        for offset in (0..8).rev() {
            let week_end = now as i64 - offset * 7 * 86400;
            let week_start = week_end - 7 * 86400;
            let label = chrono::DateTime::from_timestamp(week_start, 0)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH)
                .format("%Y-W%W")
                .to_string();
            let count: i64 = db.conn_ref().query_row(
                "SELECT COUNT(*) FROM collected_prompts
                 WHERE device_id = ?1 AND prompt_text IS NOT NULL AND lower(prompt_text) LIKE ?2
                   AND IFNULL(started_at, 0) >= ?3 AND IFNULL(started_at, 0) < ?4",
                rusqlite::params![device_id, format!("%{}%", pattern), week_start, week_end],
                |row| row.get(0),
            )?;
            weeks.push(label);
            counts.push(count);
        }

        compounding_curves.push(CompoundingCurve {
            skill_id: skill_id.clone(),
            skill_name: skill.name,
            weeks,
            counts,
        });
    }

    // 3. Active skills ranking.
    let mut active_skills: Vec<ActiveSkillMetric> = db
        .get_skill_usage_summary(90, now)?
        .into_iter()
        .map(|u| ActiveSkillMetric {
            skill_id: u.skill_id,
            skill_name: u.skill_name,
            usage_count: u.usage_count,
            last_used_at: None,
            dormant: u.usage_count == 0,
        })
        .collect();
    active_skills.sort_by(|a, b| b.usage_count.cmp(&a.usage_count));

    // 4. Capability map: prompt counts per category + whether a Skill exists.
    let mut capability_map = Vec::new();
    let start = now as i64 - 7 * 86400;
    let category_hits = crate::discovery::capability_gap::skill_category_hits(db)?;
    for cat in crate::discovery::keywords::categories() {
        let mut count = 0i64;
        let mut stmt = db.conn_ref().prepare(
            "SELECT prompt_text FROM collected_prompts
             WHERE device_id = ?1 AND IFNULL(started_at, 0) >= ?2 AND prompt_text IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![device_id, start], |row| {
            row.get::<_, String>(0)
        })?;
        for text in rows.filter_map(|r| r.ok()) {
            if cat.keywords.iter().any(|k| text.to_lowercase().contains(&k.to_lowercase())) {
                count += 1;
            }
        }
        capability_map.push(CapabilityCategory {
            category: cat.label.to_string(),
            has_skill: category_hits.contains(cat.key),
            prompt_count_7d: count,
        });
    }

    Ok(GrowthMetrics {
        skill_count_by_source,
        compounding_curves,
        active_skills,
        capability_map,
    })
}

/// Generate (or regenerate) the weekly report for a given Monday.
#[tauri::command]
pub fn generate_weekly_report(
    week: Option<String>,
    state: State<'_, AppState>,
) -> Result<WeeklyReport, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let cfg = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.ai.clone()
    };
    let week = week.unwrap_or_else(crate::discovery::this_monday_utc);
    crate::discovery::generate_weekly_report(&db, "", &cfg, &week).map_err(|e| e.to_string())
}

/// Read a generated weekly report.
#[tauri::command]
pub fn get_weekly_report(
    week: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<WeeklyReport>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let week = week.unwrap_or_else(crate::discovery::this_monday_utc);
    db.get_weekly_report(&week).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_report(
    days: u32,
    fmt: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    let projects = db
        .get_project_usage_summary(&device_id, days, current_timestamp())
        .map_err(|e| e.to_string())?;
    let skill_usage = db
        .get_skill_usage_summary(days, current_timestamp())
        .map_err(|e| e.to_string())?;
    let generated_at = current_timestamp();
    match fmt.as_str() {
        "json" => {
            // PRD-02 §5.3: portable JSON contract with `version`.
            let export = serde_json::json!({
                "version": 1,
                "device_id": device_id,
                "generated_at": generated_at,
                "days": days,
                "projects": projects,
                "skills": skill_usage,
            });
            serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
        }
        _ => {
            // markdown report with both projects and skills.
            let mut md = String::from("# SkillMint 使用报告\n\n");
            md.push_str(&format!("时间范围：近 {} 天\n\n", days));
            md.push_str("## 项目\n\n| 项目 | 会话数 | Token |\n|---|---|---|\n");
            for p in &projects {
                md.push_str(&format!("| {} | {} | {} |\n", p.name, p.session_count, p.total_tokens));
            }
            md.push_str("\n## Skill 使用\n\n| Skill | 使用次数 | 会话 | 项目 |\n|---|---|---|---|\n");
            for s in &skill_usage {
                md.push_str(&format!("| {} | {} | {} | {} |\n", s.skill_name, s.usage_count, s.session_count, s.project_count));
            }
            Ok(md)
        }
    }
}

// =============================================================================
// PRD-0 §4.8: Center Repo zip backup / restore
// =============================================================================

/// Pack the Center Repo into a zip at the user-chosen path. PRD-0 §4.8 export.
#[tauri::command]
pub fn backup_center_repo(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    backup_center_repo_inner(&center_repo, PathBuf::from(&path).as_path())
}

/// Internal version of `backup_center_repo` used by the scheduled-task registry.
pub(crate) fn backup_center_repo_inner(center_repo: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if !center_repo.exists() {
        return Err(format!("中心仓库不存在：{}", center_repo.display()));
    }
    zip_dir(center_repo, dst).map_err(|e| format!("备份失败：{}", e))
}

/// Restore a Center Repo zip with smart-merge semantics: a skill in the zip is
/// imported only if no same-named skill already exists in the center repo;
/// existing skills are never overwritten. Returns the imported/skipped lists.
/// PRD-0 §4.8 import.
#[tauri::command]
pub fn restore_center_repo(path: String, state: State<'_, AppState>) -> Result<RestoreSummary, String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    if !center_repo.exists() {
        std::fs::create_dir_all(&center_repo).map_err(|e| e.to_string())?;
    }
    let zip_path = PathBuf::from(&path);
    if !zip_path.exists() {
        return Err(format!("备份文件不存在：{}", zip_path.display()));
    }

    // Unzip into a temp staging dir, then smart-merge into the center repo.
    let staging = tempfile::tempdir().map_err(|e| format!("创建临时目录失败：{}", e))?;
    unzip_to(&zip_path, staging.path()).map_err(|e| format!("解压失败：{}", e))?;
    let (imported, skipped) = merge_into_center(staging.path(), &center_repo)
        .map_err(|e| format!("恢复失败：{}", e))?;

    // Newly imported skills need DB rows + sync targets so they show up and sync.
    if !imported.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        let now = current_timestamp();
        for name in &imported {
            let repo_path = center_repo.join(name);
            let skill = Skill {
                id: new_id(),
                name: name.clone(),
                repo_path,
                created_at: now,
                updated_at: now,
                status: crate::models::SkillStatus::Draft,
            };
            if let Err(e) = db.insert_skill(&skill) {
                eprintln!("[restore] failed to register skill {}: {}", name, e);
            }
        }
        // Reconcile sync targets for the new skills (non-fatal).
        if settings.skill_scope_mode != crate::models::SkillScopeMode::Project {
            let _ = ensure_sync_targets(&db, &settings);
        }
        let _ = apply_project_bindings(&db, &settings);
    }

    Ok(RestoreSummary { imported, skipped })
}

// =============================================================================
// PRD-03: knowledge graph
// =============================================================================

#[tauri::command]
pub fn analyze_knowledge_graph(state: State<'_, AppState>) -> Result<(usize, usize), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let (device_id, center_repo) = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        (settings.device_id.clone(), settings.center_repo.clone())
    };
    crate::kg::analyze_all_skills(&db, &device_id, &center_repo).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_knowledge_graph(
    max_nodes: Option<i64>,
    state: State<'_, AppState>,
) -> Result<crate::models::KgGraph, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_kg_graph(max_nodes.unwrap_or(200)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn confirm_kg_edge(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.confirm_kg_edge(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reject_kg_edge(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reject_kg_edge(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recommend_skills_for_task(
    task: String,
    state: State<'_, AppState>,
) -> Result<crate::models::TaskRecommendation, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    crate::kg::recommend_for_task(&db, &device_id, &task).map_err(|e| e.to_string())
}

/// PRD-03 FR-5.1: get skills related to a given skill (from kg_edges).
#[tauri::command]
pub fn get_related_skills(
    skill_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::SkillRecommendation>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let graph = db.get_kg_graph(500).map_err(|e| e.to_string())?;
    // Find the practice node for this skill, then its edges.
    let skill_name = db
        .get_skill_names()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(id, _)| id == &skill_id)
        .map(|(_, n)| n);
    let practice_id = match skill_name {
        Some(ref n) => crate::kg::node_id_for(n, "practice"),
        None => return Ok(vec![]),
    };
    let mut related = Vec::new();
    for e in &graph.edges {
        if e.source_id == practice_id || e.target_id == practice_id {
            let other_id = if e.source_id == practice_id { &e.target_id } else { &e.source_id };
            let other_label = graph.nodes.iter().find(|n| &n.id == other_id).map(|n| n.label.clone());
            if let Some(label) = other_label {
                related.push(crate::models::SkillRecommendation {
                    skill_id: other_id.clone(),
                    skill_name: label,
                    reason: e.reason.clone().unwrap_or_else(|| e.relation.clone()),
                    matched_concepts: vec![],
                });
            }
        }
    }
    Ok(related)
}

/// PRD-03 §8: export the knowledge graph as JSON (with `version`).
#[tauri::command]
pub fn export_knowledge_graph(state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let graph = db.get_kg_graph(10000).map_err(|e| e.to_string())?;
    let export = serde_json::json!({
        "version": 1,
        "nodes": graph.nodes,
        "edges": graph.edges,
    });
    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

/// PRD-03 §8: import a knowledge graph from JSON.
#[tauri::command]
pub fn import_knowledge_graph(json: String, state: State<'_, AppState>) -> Result<(usize, usize), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.device_id.clone()
    };
    let parsed: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let nodes = parsed.get("nodes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let edges = parsed.get("edges").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut n_nodes = 0;
    for n in nodes {
        let node = crate::models::KgNode {
            id: n.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            label: n.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            node_type: n.get("type").and_then(|v| v.as_str()).unwrap_or("concept").to_string(),
            source: n.get("source").and_then(|v| v.as_str()).map(|s| s.to_string()),
            description: n.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
        };
        if !node.id.is_empty() {
            db.upsert_kg_node(&node).map_err(|e| e.to_string())?;
            n_nodes += 1;
        }
    }
    let mut n_edges = 0;
    for e in edges {
        let edge = crate::models::KgEdge {
            id: e.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            device_id: device_id.clone(),
            source_id: e.get("source_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            target_id: e.get("target_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            relation: e.get("relation").and_then(|v| v.as_str()).unwrap_or("related").to_string(),
            weight: e.get("weight").and_then(|v| v.as_f64()),
            reason: e.get("reason").and_then(|v| v.as_str()).map(|s| s.to_string()),
            is_manual: e.get("is_manual").and_then(|v| v.as_bool()).unwrap_or(false),
            is_rejected: e.get("is_rejected").and_then(|v| v.as_bool()).unwrap_or(false),
        };
        if !edge.id.is_empty() && !edge.source_id.is_empty() {
            db.upsert_kg_edge(&edge).map_err(|e| e.to_string())?;
            n_edges += 1;
        }
    }
    Ok((n_nodes, n_edges))
}

// =============================================================================
// PRD-07: remote sources, unified discovery, safety scan
// =============================================================================

/// Return the app data dir as a PathBuf (used to scope source caches).
fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())
}

/// PRD-07 §3.1: connect a new remote source. Fetches (for github/git) or copies
/// (for local), scans for skills, and persists the `sources` row. Respects the
/// `remote_enabled` master switch: returns an error guiding the user to enable
/// it first (the frontend controls the actual toggle via `save_settings`).
#[tauri::command]
pub fn add_source(
    name: String,
    source_type: String,
    url: String,
    ref_spec: Option<String>,
    subpath: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Source, String> {
    {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        if !settings.remote_enabled {
            return Err("远程功能未开启。请在「偏好设置」中开启「远程功能」后再连接源。".into());
        }
    }
    let app_dir = app_data_dir(&app)?;
    let stype = SourceType::from_str(&source_type);
    let id = new_id();
    let cache_dir = remote::source_cache_dir(&app_dir, &id);

    // For github/git sources, try to extract a branch/tag + subpath from a
    // pasted web URL (e.g. /tree/main/skills). We only apply them when the
    // caller didn't explicitly pass a value, so explicit args always win.
    let (mut ref_spec, mut subpath) = (ref_spec, subpath);
    if matches!(stype, SourceType::Github | SourceType::Git) {
        if let Some((_, _, parsed_ref, parsed_sub)) = parse_github_url(&url) {
            if ref_spec.is_none() {
                ref_spec = parsed_ref;
            }
            if subpath.is_none() {
                subpath = parsed_sub;
            }
        }
    }
    let ref_spec = ref_spec.unwrap_or_else(|| "main".to_string());
    let subpath = subpath.unwrap_or_default();

    let commit_sha = match stype {
        SourceType::Github => {
            let (owner, repo) = parse_github_shorthand(&url)
                .ok_or_else(|| "GitHub 源格式应为 owner/repo 或完整 GitHub URL".to_string())?;
            remote::fetch_github_tarball(&owner, &repo, &ref_spec, &cache_dir)
                .map_err(|e| e.to_string())?
        }
        SourceType::Git => {
            // git source: we still rely on GitHub-style tarball for MVP; a full
            // generic `git clone` path is P2. For now, require a github URL.
            let (owner, repo) = parse_github_shorthand(&url).ok_or_else(|| {
                "当前 git 源仅支持 GitHub URL（owner/repo）；完整 git clone 支持规划在 P2".to_string()
            })?;
            remote::fetch_github_tarball(&owner, &repo, &ref_spec, &cache_dir)
                .map_err(|e| e.to_string())?
        }
        SourceType::Local => {
            let src = expand_path(&url);
            remote::cache_local_source(&src, &cache_dir).map_err(|e| e.to_string())?
        }
    };

    let now = current_timestamp();
    let source = Source {
        id: id.clone(),
        name,
        source_type: stype,
        url,
        ref_spec,
        subpath,
        cache_path: cache_dir.to_string_lossy().to_string(),
        commit_sha: commit_sha.clone(),
        added_at: now,
        last_fetched_at: Some(now),
        pull_policy: "manual".to_string(),
        remote_revision: commit_sha,
    };

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.insert_source(&source).map_err(|e| e.to_string())?;
    Ok(source)
}

/// Parse `owner/repo` or a `github.com/owner/repo(...)` URL into (owner, repo).
fn parse_github_shorthand(input: &str) -> Option<(String, String)> {
    parse_github_url(input).map(|(o, r, _, _)| (o, r))
}

/// Parse a GitHub reference into `(owner, repo, Option<ref_spec>, Option<subpath>)`.
///
/// Recognizes URLs pasted from the GitHub web UI that carry a branch/tag and a
/// subpath, e.g. `https://github.com/o/r/tree/main/skills/foo` or
/// `git@github.com:o/r.git#develop`. The extra segments are returned so the
/// caller can pre-fill `ref_spec` and `subpath` instead of silently dropping
/// them (which previously pulled the whole repo root against user intent).
fn parse_github_url(input: &str) -> Option<(String, String, Option<String>, Option<String>)> {
    let s = input.trim();
    // Split off an optional `#ref` fragment (some users paste it that way).
    let (main_part, hash_ref) = match s.split_once('#') {
        Some((a, b)) => (a, Some(b.trim().to_string())),
        None => (s, None),
    };
    // Strip a leading scheme + optional github.com host.
    let stripped = main_part
        .strip_prefix("https://")
        .or_else(|| main_part.strip_prefix("http://"))
        .unwrap_or(main_part);
    let after_host = stripped.strip_prefix("github.com/").unwrap_or(stripped);
    let after_git = after_host.strip_prefix("git@github.com:").unwrap_or(after_host);
    let path = after_git
        .split('?')
        .next()?
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let owner = parts[0].to_string();
    let repo = parts[1].to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    // Web-UI tree/blob URLs: /o/r/tree/<ref>/[sub/path...]
    let mut ref_spec = hash_ref;
    let mut subpath: Option<String> = None;
    if parts.len() >= 4 && (parts[2] == "tree" || parts[2] == "blob") {
        if ref_spec.is_none() {
            ref_spec = Some(parts[3].to_string());
        }
        if parts.len() > 4 {
            let sp = parts[4..].join("/");
            if !sp.is_empty() {
                // Point at the containing dir if the URL targets a file (blob).
                let sp = if parts[2] == "blob" {
                    sp.rsplit_once('/').map(|(dir, _)| dir.to_string()).unwrap_or(sp)
                } else {
                    sp
                };
                if !sp.is_empty() {
                    subpath = Some(sp);
                }
            }
        }
    }
    Some((owner, repo, ref_spec, subpath))
}

/// PRD-07 §3.1: list all connected sources.
#[tauri::command]
pub fn get_sources(state: State<'_, AppState>) -> Result<Vec<Source>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_sources().map_err(|e| e.to_string())
}

/// PRD-07 §3.1: remove a source and delete its cache dir. Skills already
/// installed into the center repo are untouched.
#[tauri::command]
pub fn remove_source(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let source = db
        .get_source_by_id(&id)
        .map_err(|e| e.to_string())?
        .ok_or("源不存在")?;
    db.delete_source(&id).map_err(|e| e.to_string())?;
    drop(db);
    // Best-effort cache cleanup.
    let cache = PathBuf::from(&source.cache_path);
    if cache.exists() {
        let _ = remove_path(&cache);
    }
    let _ = app; // keep the handle for symmetry / future events
    Ok(())
}

/// PRD-07 §3.1: re-fetch a source (github/git) to refresh its cache and update
/// the `remote_revision`. Returns the refreshed Source row.
#[tauri::command]
pub fn refresh_source(
    id: String,
    state: State<'_, AppState>,
) -> Result<Source, String> {
    refresh_source_inner(&state, &id)
}

/// Internal version of `refresh_source` used by the scheduled-task registry.
pub(crate) fn refresh_source_inner(state: &AppState, id: &str) -> Result<Source, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut source = db
        .get_source_by_id(id)
        .map_err(|e| e.to_string())?
        .ok_or("源不存在")?;

    let cache_dir = PathBuf::from(&source.cache_path);
    match source.source_type {
        SourceType::Github | SourceType::Git => {
            let (owner, repo) = parse_github_shorthand(&source.url)
                .ok_or_else(|| "源 URL 无法解析为 owner/repo".to_string())?;
            // Clear and re-fetch.
            if cache_dir.exists() {
                let _ = remove_path(&cache_dir);
            }
            let sha = remote::fetch_github_tarball(&owner, &repo, &source.ref_spec, &cache_dir)
                .map_err(|e| e.to_string())?;
            source.commit_sha = sha.clone();
            source.remote_revision = sha;
        }
        SourceType::Local => {
            let src = expand_path(&source.url);
            remote::cache_local_source(&src, &cache_dir).map_err(|e| e.to_string())?;
        }
    }
    source.last_fetched_at = Some(current_timestamp());
    db.insert_source(&source).map_err(|e| e.to_string())?;
    Ok(source)
}

/// PRD-07 §3.1: list skills in a source (scanned from cache), annotated with
/// whether each is already installed in the center repo.
#[tauri::command]
pub fn list_source_skills(
    source_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillRemoteMeta>, String> {
    let (cache_dir, subpath, source_id) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let source = db
            .get_source_by_id(&source_id)
            .map_err(|e| e.to_string())?
            .ok_or("源不存在")?;
        (
            PathBuf::from(&source.cache_path),
            source.subpath.clone(),
            source.id.clone(),
        )
    };
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    let mut metas = remote::scan_source_skills(&cache_dir, &subpath, &source_id, false)
        .map_err(|e| e.to_string())?;
    // Annotate installed_locally.
    for m in metas.iter_mut() {
        m.installed_locally = center_repo.join(&m.skill_name).exists();
    }
    Ok(metas)
}

/// PRD-07 §3.2: scan a skill's SKILL.md for high-risk instructions. Reads from
/// the source cache (by source_id + skill_path).
#[tauri::command]
pub fn scan_skill_safety(
    source_id: String,
    skill_path: String,
    state: State<'_, AppState>,
) -> Result<SafetyScanResult, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let source = db
        .get_source_by_id(&source_id)
        .map_err(|e| e.to_string())?
        .ok_or("源不存在")?;
    let cache_dir = PathBuf::from(&source.cache_path);
    let skill_dir = cache_dir.join(&skill_path);
    let body = remote::read_skill_body(&skill_dir);
    Ok(remote::scan_safety(&body))
}

/// Read a skill's SKILL.md (case-insensitive) if it exists.
fn read_skill_md(skill_dir: &Path) -> Option<String> {
    for candidate in ["SKILL.md", "skill.md"] {
        let p = skill_dir.join(candidate);
        if p.is_file() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                return Some(content);
            }
        }
    }
    None
}

/// Check whether a SKILL.md frontmatter matches the query on description,
/// tags, concepts, or scenarios. Returns the match field name ("description" or
/// "tags") on hit.
pub(crate) fn match_frontmatter(skill_dir: &Path, q: &str) -> Option<String> {
    let md = read_skill_md(skill_dir)?;
    let (frontmatter, _) = parse_skill_md(&md);

    if let Some(desc) = frontmatter.get("description").and_then(|v| v.as_str()) {
        if desc.to_lowercase().contains(q) {
            return Some("description".to_string());
        }
    }

    for key in ["tags", "concepts", "scenarios"] {
        if let Some(v) = frontmatter.get(key) {
            if json_value_contains(v, q) {
                return Some("tags".to_string());
            }
        }
    }

    None
}

fn json_value_contains(v: &serde_json::Value, q: &str) -> bool {
    match v {
        serde_json::Value::String(s) => s.to_lowercase().contains(q),
        serde_json::Value::Array(arr) => arr.iter().any(|item| json_value_contains(item, q)),
        _ => false,
    }
}

/// Search the body of a SKILL.md for the query and return a short snippet with
/// the matched term wrapped in «» for frontend highlighting.
pub(crate) fn search_body(skill_dir: &Path, q: &str) -> Option<String> {
    let md = read_skill_md(skill_dir)?;
    let (_, body) = parse_skill_md(&md);
    let body_lower = body.to_lowercase();
    let idx = body_lower.find(q)?;
    let start = idx.saturating_sub(50);
    let end = (idx + q.len() + 50).min(body.len());
    let raw = &body[start..end];

    let match_start = idx - start;
    let match_end = match_start + q.len();
    let mut highlighted = String::with_capacity(raw.len() + 4);
    highlighted.push_str(&raw[..match_start]);
    highlighted.push('«');
    highlighted.push_str(&raw[match_start..match_end]);
    highlighted.push('»');
    highlighted.push_str(&raw[match_end..]);

    Some(format!("…{highlighted}…"))
}

/// Map a match field to its sort priority (lower = more relevant).
fn match_field_rank(field: Option<&str>) -> u8 {
    match field {
        Some("name") => 0,
        Some("description") => 1,
        Some("tags") => 2,
        Some("body") => 3,
        _ => 4,
    }
}

/// PRD-07 §3.3: unified search across local skills and all connected sources'
/// caches. De-duplicates by skill name (local wins). Optionally filters by a
/// `source_id` to scope to one source. Empty query returns local skills + a
/// representative first page of each source (for the "browse" tab).
///
/// Performance: locks are held only for the short DB reads; the cache scan +
/// filesystem checks happen lock-free, and the per-skill content hash is
/// skipped (search never needs it). This keeps the command responsive even
/// with many sources × many skills, and avoids blocking the sync scheduler.
#[tauri::command]
pub fn search_all(
    query: String,
    source_filter: Option<String>, // Some(id) to scope; None to search all
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let q = query.trim().to_lowercase();

    // --- Phase 1: gather everything we need under locks, then release them ---
    let (local_skills, metrics_map, sources, remote_enabled, center_repo) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let settings = state.settings.lock().map_err(|e| e.to_string())?;

        let local_skills = db.get_skills().map_err(|e| e.to_string())?;
        // Local usage + correction metrics for ranking (best-effort; 0 if no data).
        let metrics_map = db
            .get_skill_discovery_metrics(7, current_timestamp())
            .map_err(|e| e.to_string())?;
        let sources = if settings.remote_enabled {
            db.get_sources().map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        (
            local_skills,
            metrics_map,
            sources,
            settings.remote_enabled,
            settings.center_repo.clone(),
        )
    };

    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // --- local skills ---
    for s in &local_skills {
        let key = s.name.to_lowercase();
        let (usage, correction) = *metrics_map.get(&key).unwrap_or(&(0, 0));

        let fast_match = if q.is_empty() {
            None
        } else if s.name.to_lowercase().contains(&q) {
            Some("name".to_string())
        } else {
            match_frontmatter(Path::new(&s.repo_path), &q)
        };

        if !q.is_empty() && fast_match.is_none() {
            // Slow path: search the SKILL.md body.
            if let Some(snippet) = search_body(Path::new(&s.repo_path), &q) {
                seen.insert(key);
                results.push(SearchResult {
                    skill_name: s.name.clone(),
                    origin: "local".to_string(),
                    source_id: None,
                    source_name: None,
                    description: None,
                    installed_locally: true,
                    usage_count: usage,
                    correction_count: correction,
                    skill_path: String::new(),
                    match_field: Some("body".to_string()),
                    snippet: Some(snippet),
                });
            }
            continue;
        }

        seen.insert(key);
        results.push(SearchResult {
            skill_name: s.name.clone(),
            origin: "local".to_string(),
            source_id: None,
            source_name: None,
            description: None,
            installed_locally: true,
            usage_count: usage,
            correction_count: correction,
            skill_path: String::new(),
            match_field: fast_match,
            snippet: None,
        });
    }

    // --- remote sources (only if remote_enabled, to honor the privacy gate) ---
    if remote_enabled {
        for src in &sources {
            if let Some(ref filter) = source_filter {
                if &src.id != filter {
                    continue;
                }
            }
            let cache_dir = PathBuf::from(&src.cache_path);
            // skip_hash = true: search only needs name/description/path.
            let metas = remote::scan_source_skills(&cache_dir, &src.subpath, &src.id, true)
                .unwrap_or_default();
            for m in metas {
                let key = m.skill_name.to_lowercase();
                // De-duplicate by skill name: local wins.
                if seen.contains(&key) {
                    continue;
                }

                let fast_match = if q.is_empty() {
                    None
                } else if m.skill_name.to_lowercase().contains(&q) {
                    Some("name".to_string())
                } else if m
                    .description
                    .as_ref()
                    .map(|d| d.to_lowercase().contains(&q))
                    .unwrap_or(false)
                {
                    Some("description".to_string())
                } else {
                    None
                };

                if !q.is_empty() && fast_match.is_none() {
                    let skill_dir = cache_dir.join(&m.skill_path);
                    if let Some(snippet) = search_body(&skill_dir, &q) {
                        let installed = center_repo.join(&m.skill_name).exists();
                        let (usage, correction) = *metrics_map.get(&key).unwrap_or(&(0, 0));
                        seen.insert(key);
                        results.push(SearchResult {
                            skill_name: m.skill_name.clone(),
                            origin: "remote".to_string(),
                            source_id: Some(src.id.clone()),
                            source_name: Some(src.name.clone()),
                            description: m.description.clone(),
                            installed_locally: installed,
                            usage_count: usage,
                            correction_count: correction,
                            skill_path: m.skill_path.clone(),
                            match_field: Some("body".to_string()),
                            snippet: Some(snippet),
                        });
                    }
                    continue;
                }

                let installed = center_repo.join(&m.skill_name).exists();
                let (usage, correction) = *metrics_map.get(&key).unwrap_or(&(0, 0));
                seen.insert(key);
                results.push(SearchResult {
                    skill_name: m.skill_name.clone(),
                    origin: "remote".to_string(),
                    source_id: Some(src.id.clone()),
                    source_name: Some(src.name.clone()),
                    description: m.description,
                    installed_locally: installed,
                    usage_count: usage,
                    correction_count: correction,
                    skill_path: m.skill_path,
                    match_field: fast_match,
                    snippet: None,
                });
            }
        }
    }

    // PRD-09 §3.3c: metric-driven ranking, with P2-1 match-field priority first.
    // 1) name, 2) description, 3) tags, 4) body, then within each band:
    // installed & recently used > installed unused > remote hot > everything else,
    // then usage desc, correction desc, name asc.
    results.sort_by(|a, b| {
        fn rank(r: &SearchResult) -> u8 {
            match (r.origin.as_str(), r.installed_locally, r.usage_count > 0) {
                (_, true, true) => 0,
                (_, true, false) => 1,
                ("remote", false, true) => 2,
                _ => 3,
            }
        }
        match_field_rank(a.match_field.as_deref())
            .cmp(&match_field_rank(b.match_field.as_deref()))
            .then_with(|| rank(a).cmp(&rank(b)))
            .then_with(|| b.usage_count.cmp(&a.usage_count))
            .then_with(|| b.correction_count.cmp(&a.correction_count))
            .then_with(|| a.skill_name.cmp(&b.skill_name))
    });
    Ok(results)
}

// -----------------------------------------------------------------------------
// PRD-02 Phase 2: skill bundle commands
// -----------------------------------------------------------------------------

#[tauri::command]
pub fn list_bundles(state: State<'_, AppState>) -> Result<Vec<SkillBundle>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.device_id.clone()
    };
    db.list_bundles(&device_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_bundle_detail(
    id: String,
    state: State<'_, AppState>,
) -> Result<(SkillBundle, Vec<SkillBundleItem>), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let bundle = db
        .get_bundle(&id)
        .map_err(|e| e.to_string())?
        .ok_or("技能集不存在")?;
    let items = db.get_bundle_items(&id).map_err(|e| e.to_string())?;
    Ok((bundle, items))
}

#[tauri::command]
pub fn create_bundle(
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<SkillBundle, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.device_id.clone()
    };
    let id = crate::db::new_id();
    db.create_bundle(&device_id, &id, &name, description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_bundle(
    id: String,
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<SkillBundle, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_bundle(&id, &name, description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_bundle(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_bundle(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_skill_to_bundle(
    bundle_id: String,
    skill_id: String,
    state: State<'_, AppState>,
) -> Result<SkillBundleItem, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.device_id.clone()
    };
    if db
        .get_skill_by_id(&skill_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err("Skill 不存在".to_string());
    }
    let items = db.get_bundle_items(&bundle_id).map_err(|e| e.to_string())?;
    let sort_order = items.iter().map(|i| i.sort_order).max().unwrap_or(-1) + 1;
    let id = crate::db::new_id();
    db.add_bundle_item(&device_id, &id, &bundle_id, &skill_id, sort_order)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_skill_from_bundle(
    bundle_id: String,
    skill_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_bundle_item(&bundle_id, &skill_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn apply_bundle_to_project(
    bundle_id: String,
    project_id: String,
    agent_ids: Vec<String>,
    mode: String,
    state: State<'_, AppState>,
) -> Result<ApplyBundleResult, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.device_id.clone()
    };
    let items = db.get_bundle_items(&bundle_id).map_err(|e| e.to_string())?;
    let mut applied = Vec::new();
    let mut skipped = Vec::new();

    for item in items {
        match install_skill_core(&db, &device_id, &item.skill_id, &project_id, &agent_ids, &mode) {
            Ok(_) => applied.push(item.skill_name),
            Err(e) => skipped.push((item.skill_name, e.to_string())),
        }
    }
    Ok(ApplyBundleResult { applied, skipped })
}

#[tauri::command]
pub fn export_bundle(id: String, state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let bundle = db
        .get_bundle(&id)
        .map_err(|e| e.to_string())?
        .ok_or("技能集不存在")?;
    let items = db.get_bundle_items(&id).map_err(|e| e.to_string())?;
    let export = BundleExport {
        name: bundle.name,
        description: bundle.description,
        exported_at: current_timestamp(),
        skills: items
            .into_iter()
            .map(|i| BundleExportSkill {
                name: i.skill_name,
                required: true,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

/// P2-3: export a bundle as a human-readable directory:
/// `<output_dir>/<bundle.name>/manifest.json` + `skills/<skill_name>/SKILL.md`.
#[tauri::command]
pub fn export_bundle_directory(
    id: String,
    output_dir: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let bundle = db
        .get_bundle(&id)
        .map_err(|e| e.to_string())?
        .ok_or("技能集不存在")?;
    let items = db.get_bundle_items(&id).map_err(|e| e.to_string())?;

    let base = std::path::PathBuf::from(output_dir).join(sanitize_dir_name(&bundle.name));
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;

    let manifest = crate::models::BundleExport {
        name: bundle.name.clone(),
        description: bundle.description.clone(),
        exported_at: current_timestamp(),
        skills: items
            .iter()
            .map(|i| crate::models::BundleExportSkill {
                name: i.skill_name.clone(),
                required: true,
            })
            .collect(),
    };
    std::fs::write(
        base.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let skills_dir = base.join("skills");
    for item in &items {
        if let Some(skill) = db.get_skill_by_id(&item.skill_id).map_err(|e| e.to_string())? {
            let dest = skills_dir.join(sanitize_dir_name(&skill.name));
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let src = skill.repo_path.join("SKILL.md");
            if src.exists() {
                std::fs::copy(&src, dest.join("SKILL.md")).map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(base.to_string_lossy().to_string())
}

fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[tauri::command]
pub fn import_bundle(json: String, state: State<'_, AppState>) -> Result<SkillBundle, String> {
    let export: BundleExport = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let device_id = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.device_id.clone()
    };

    let bundle_id = crate::db::new_id();
    let bundle = db
        .create_bundle(
            &device_id,
            &bundle_id,
            &export.name,
            export.description.as_deref(),
        )
        .map_err(|e| e.to_string())?;

    for skill_export in export.skills {
        let skill = db
            .get_skill_by_name(&skill_export.name)
            .map_err(|e| e.to_string())?;
        match skill {
            Some(s) => {
                let items = db.get_bundle_items(&bundle_id).map_err(|e| e.to_string())?;
                let sort_order = items.iter().map(|i| i.sort_order).max().unwrap_or(-1) + 1;
                let item_id = crate::db::new_id();
                db.add_bundle_item(&device_id, &item_id, &bundle_id, &s.id, sort_order)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                if skill_export.required {
                    return Err(format!("缺少必需的 Skill：{}", skill_export.name));
                }
            }
        }
    }
    Ok(bundle)
}

/// PRD-07 §3.3: preview a remote skill's SKILL.md body (for the preview pane).
#[tauri::command]
pub fn preview_remote_skill(
    source_id: String,
    skill_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let source = db
        .get_source_by_id(&source_id)
        .map_err(|e| e.to_string())?
        .ok_or("源不存在")?;
    let cache_dir = PathBuf::from(&source.cache_path);
    let skill_dir = cache_dir.join(&skill_path);
    Ok(remote::read_skill_body(&skill_dir))
}

pub(crate) fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// =============================================================================
// PRD-09 §4: snapshots + Git version control
// =============================================================================

fn snapshots_root(app_dir: &PathBuf) -> PathBuf {
    app_dir.join("snapshots")
}

fn snapshot_dir(app_dir: &PathBuf, id: &str) -> PathBuf {
    snapshots_root(app_dir).join(id)
}

fn snapshot_meta_path(app_dir: &PathBuf, id: &str) -> PathBuf {
    snapshot_dir(app_dir, id).join(".snapshot-meta.json")
}

fn count_skills_in_repo(repo: &PathBuf) -> i64 {
    if !repo.is_dir() {
        return 0;
    }
    let mut count = 0;
    for entry in walkdir::WalkDir::new(repo).max_depth(3).into_iter().flatten() {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == "skill.md" {
                count += 1;
            }
        }
    }
    count
}

/// PRD-09 §4: create a local point-in-time snapshot of the Center Repo.
#[tauri::command]
pub fn create_snapshot(
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<SnapshotInfo, String> {
    let (center_repo, app_dir) = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        (settings.center_repo.clone(), state.app_dir.clone())
    };
    if !center_repo.is_dir() {
        return Err("中心仓库不存在".to_string());
    }

    let id = format!("{}-{}", current_timestamp(), crate::db::new_id().replace('-', ""));
    let root = snapshots_root(&app_dir);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let dest = snapshot_dir(&app_dir, &id);

    copy_dir_all(&center_repo, &dest).map_err(|e| format!("快照复制失败：{e}"))?;

    let meta = serde_json::json!({
        "id": id,
        "created_at": current_timestamp(),
        "note": note,
    });
    std::fs::write(snapshot_meta_path(&app_dir, &id), meta.to_string())
        .map_err(|e| e.to_string())?;

    Ok(SnapshotInfo {
        id,
        created_at: current_timestamp(),
        note,
        skill_count: count_skills_in_repo(&dest),
    })
}

/// PRD-09 §4: list local Center Repo snapshots, newest first.
#[tauri::command]
pub fn list_snapshots(state: State<'_, AppState>) -> Result<Vec<SnapshotInfo>, String> {
    let app_dir = state.app_dir.clone();
    let root = snapshots_root(&app_dir);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let meta_path = snapshot_meta_path(&app_dir, &id);
        let (created_at, note) = if meta_path.is_file() {
            let raw = std::fs::read_to_string(&meta_path).unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            (
                v.get("created_at").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("note").and_then(|x| x.as_str()).map(|s| s.to_string()),
            )
        } else {
            (0, None)
        };
        let dest = snapshot_dir(&app_dir, &id);
        out.push(SnapshotInfo {
            id,
            created_at,
            note,
            skill_count: count_skills_in_repo(&dest),
        });
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// PRD-09 §4: restore the Center Repo from a local snapshot.
/// The current repo is first backed up as `center-repo-pre-restore-<ts>` under
/// the snapshots root so the operation is reversible.
#[tauri::command]
pub fn restore_snapshot(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let (center_repo, app_dir) = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        (settings.center_repo.clone(), state.app_dir.clone())
    };
    let src = snapshot_dir(&app_dir, &id);
    if !src.is_dir() {
        return Err("快照不存在".to_string());
    }

    let root = snapshots_root(&app_dir);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    // Backup current repo before overwriting.
    if center_repo.is_dir() {
        let backup = root.join(format!("center-repo-pre-restore-{}", current_timestamp()));
        copy_dir_all(&center_repo, &backup)
            .map_err(|e| format!("备份当前仓库失败：{e}"))?;
    }

    // Remove current repo and replace with snapshot.
    if center_repo.exists() {
        remove_path(&center_repo).map_err(|e| format!("清理当前仓库失败：{e}"))?;
    }
    copy_dir_all(&src, &center_repo).map_err(|e| format!("恢复快照失败：{e}"))?;
    Ok(())
}

// -----------------------------------------------------------------------------
// PRD-10: scheduled tasks
// -----------------------------------------------------------------------------

#[tauri::command]
pub fn get_scheduled_tasks(state: State<'_, AppState>) -> Result<Vec<ScheduledTask>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_scheduled_tasks().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_scheduled_task(
    task: ScheduledTask,
    state: State<'_, AppState>,
) -> Result<ScheduledTask, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut task = task;
    let now = current_timestamp();
    if task.id.is_empty() {
        task.id = new_id();
        task.created_at = now;
    }
    task.updated_at = now;
    // Recompute next run whenever the task is saved.
    task.next_run_at = crate::scheduler::engine::compute_next_run(&task, now);
    db.upsert_scheduled_task(&task).map_err(|e| e.to_string())?;
    Ok(task)
}

#[tauri::command]
pub fn delete_scheduled_task(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_scheduled_task(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_scheduled_task_now(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TaskRun, String> {
    // Verify the task exists before spawning.
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        if db.get_scheduled_task(&id).map_err(|e| e.to_string())?.is_none() {
            return Err("Task not found".to_string());
        }
    }
    crate::scheduler::run_task_now(&app, &id, TriggerSource::Manual)
        .await
}

#[tauri::command]
pub fn get_task_runs(
    task_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<Vec<TaskRun>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_task_runs(&task_id, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pause_scheduled_task(
    id: String,
    state: State<'_, AppState>,
) -> Result<ScheduledTask, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut task = db
        .get_scheduled_task(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Task not found".to_string())?;
    task.enabled = false;
    task.updated_at = current_timestamp();
    db.upsert_scheduled_task(&task).map_err(|e| e.to_string())?;
    Ok(task)
}

#[tauri::command]
pub fn resume_scheduled_task(
    id: String,
    state: State<'_, AppState>,
) -> Result<ScheduledTask, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut task = db
        .get_scheduled_task(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Task not found".to_string())?;
    task.enabled = true;
    task.updated_at = current_timestamp();
    task.next_run_at = crate::scheduler::engine::compute_next_run(&task, current_timestamp());
    db.upsert_scheduled_task(&task).map_err(|e| e.to_string())?;
    Ok(task)
}

/// P2-2: return the data dictionary (db path + table descriptions).
#[tauri::command]
pub fn export_data_dictionary(state: State<'_, AppState>) -> Result<crate::models::DataDictionary, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.export_data_dictionary().map_err(|e| e.to_string())
}

/// P2-2: export selected tables as a JSON payload.
#[tauri::command]
pub fn export_raw_data(
    tables: Vec<String>,
    state: State<'_, AppState>,
) -> Result<crate::models::RawDataExport, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.export_raw_data(&tables).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_github_url;

    #[test]
    fn parse_github_url_owner_repo_only() {
        let (o, r, rf, sp) = parse_github_url("owner/repo").unwrap();
        assert_eq!(o, "owner");
        assert_eq!(r, "repo");
        assert!(rf.is_none());
        assert!(sp.is_none());
    }

    #[test]
    fn parse_github_url_full_tree_url_fills_ref_and_subpath() {
        let url = "https://github.com/vercel-labs/agent-skills/tree/main/skills/frontend";
        let (o, r, rf, sp) = parse_github_url(url).unwrap();
        assert_eq!(o, "vercel-labs");
        assert_eq!(r, "agent-skills");
        assert_eq!(rf.as_deref(), Some("main"));
        assert_eq!(sp.as_deref(), Some("skills/frontend"));
    }

    #[test]
    fn parse_github_url_blob_url_uses_containing_dir() {
        let url = "https://github.com/o/r/blob/develop/skills/foo/SKILL.md";
        let (o, r, rf, sp) = parse_github_url(url).unwrap();
        assert_eq!(o, "o");
        assert_eq!(r, "r");
        assert_eq!(rf.as_deref(), Some("develop"));
        assert_eq!(sp.as_deref(), Some("skills/foo"));
    }

    #[test]
    fn parse_github_url_ssh_with_git_suffix_and_hash_ref() {
        let (o, r, rf, sp) = parse_github_url("git@github.com:owner/repo.git#develop").unwrap();
        assert_eq!(o, "owner");
        assert_eq!(r, "repo");
        assert_eq!(rf.as_deref(), Some("develop"));
        assert!(sp.is_none());
    }

    #[test]
    fn parse_github_url_rejects_too_few_segments() {
        assert!(parse_github_url("just-owner").is_none());
        assert!(parse_github_url("").is_none());
    }
}
