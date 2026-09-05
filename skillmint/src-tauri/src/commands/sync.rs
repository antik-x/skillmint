use super::*;


/// Scheduled-task entry point (TaskKind::SyncAll): run one legacy sync pass.
/// Kept as a thin wrapper — the sync engine has no interactive command
/// surface since P3-6, but existing scheduled tasks must keep working.
pub(crate) fn run_sync_core(state: &AppState) -> Result<SyncAllResult, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let report = crate::sync::sync_all(&db).map_err(|e| e.to_string())?;
    let conflict_count = report
        .updated
        .iter()
        .filter(|t| t.status == SyncStatus::Conflict)
        .count();
    let success_count = report.updated.len();
    let _ = invalidate_directory_skill_cache(&db, None);
    Ok(SyncAllResult {
        targets: report.updated,
        imported_skills: 0,
        import_conflicts: conflict_count,
        success_count,
        failure_count: report.broken.len(),
        failures: report.broken,
    })
}

/// P0-2: classify one agent-side directory entry against the center repo.
///
/// A symlink that resolves to the center repo's same-named directory is a
/// healthy synced link: report it as present + matching WITHOUT hashing.
/// (`compute_hash` on a symlink hashes the target-path string, which can never
/// equal the center content hash — that false mismatch is what labeled every
/// legit link "内容冲突".) The shortcut lives here at the scan layer rather
/// than in `compute_hash` because the sync state machine (`evaluate_sync_target`)
/// and the version/diff logic rely on the existing hash semantics.
fn classify_agent_skill_entry(
    path: &std::path::Path,
    center_path: &std::path::Path,
) -> (bool, Option<bool>) {
    if !center_path.exists() {
        return (false, None);
    }
    if crate::fs::is_symlink_to(path, center_path) {
        return (true, Some(true));
    }
    let agent_hash = crate::fs::compute_hash(path).ok();
    let center_hash = crate::fs::compute_hash(center_path).ok();
    (true, Some(agent_hash == center_hash && agent_hash.is_some()))
}

#[tauri::command]
pub fn scan_agent_skills(
    agent_id: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentSkillItem>, String> {
    let mut db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    let agent = db
        .get_agents()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or("Agent not found")?;

    if !force.unwrap_or(false) {
        if let Ok(cached) = db.get_directory_skills(&agent_id, &agent.skill_directory) {
            if !cached.is_empty() {
                return Ok(cached);
            }
        }
    }

    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&agent.skill_directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // P0-3: only real skills (SKILL.md present), never excluded names.
            if name.is_empty()
                || crate::scan::is_excluded_scan_name(&name, &settings.scan_exclude_names)
                || !crate::scan::is_skill_dir(&path)
            {
                continue;
            }
            let center_path = settings.center_repo.join(&name);
            let (exists, content_match) = classify_agent_skill_entry(&path, &center_path);
            items.push(AgentSkillItem {
                name,
                exists_in_center: exists,
                content_match,
            });
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    let _ = db.replace_directory_skills(
        &agent_id,
        &agent.skill_directory,
        &items,
        current_timestamp(),
    );
    Ok(items)
}

/// Batch count cached skills for multiple agents, scoped to each agent's primary
/// skill_directory. Returns an empty record for agents with no cached scan yet.
#[tauri::command]
pub fn get_agent_skill_counts(
    agent_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<HashMap<String, usize>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let agents = db.get_agents().map_err(|e| e.to_string())?;
    let mapping: Vec<(String, std::path::PathBuf)> = agent_ids
        .into_iter()
        .filter_map(|id| {
            agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| (id, a.skill_directory.clone()))
        })
        .collect();
    db.count_agent_directory_skills(&mapping)
        .map_err(|e| e.to_string())
}

/// PRD-06 §3.3 (P1): scan ONE directory (an agent's secondary directory, e.g.
/// Claude Code's `commands` dir) and list the skills it contains — same shape as
/// `scan_agent_skills`, but scoped to an explicit path rather than the agent's
/// primary `skill_directory`. Used by the per-directory "重新扫描" button so a
/// user can refresh a single directory without triggering a full global sync.
#[tauri::command]
pub fn scan_directory_skills(
    agent_id: String,
    path: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentSkillItem>, String> {
    let mut db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    scan_directory_skills_inner(&mut db, &settings, &agent_id, &path, force.unwrap_or(false))
}

/// Internal version of `scan_directory_skills` that accepts already-locked
/// `Db` and `Settings`. Used by the scheduled-task registry.
pub(crate) fn scan_directory_skills_inner(
    db: &mut crate::db::Db,
    settings: &crate::settings::Settings,
    agent_id: &str,
    path: &str,
    force: bool,
) -> Result<Vec<AgentSkillItem>, String> {
    let dir = expand_path(path);

    if !force {
        if let Ok(cached) = db.get_directory_skills(agent_id, &dir) {
            if !cached.is_empty() {
                return Ok(cached);
            }
        }
    }

    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // P0-3: only real skills (SKILL.md present), never excluded names.
            if name.is_empty()
                || crate::scan::is_excluded_scan_name(&name, &settings.scan_exclude_names)
                || !crate::scan::is_skill_dir(&p)
            {
                continue;
            }
            let center_path = settings.center_repo.join(&name);
            let (exists, content_match) = classify_agent_skill_entry(&p, &center_path);
            items.push(AgentSkillItem {
                name,
                exists_in_center: exists,
                content_match,
            });
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    let _ = db.replace_directory_skills(agent_id, &dir, &items, current_timestamp());
    Ok(items)
}

