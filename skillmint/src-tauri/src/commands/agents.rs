use super::*;

#[tauri::command]
pub fn save_agent(agent: Agent, state: State<'_, AppState>) -> Result<Agent, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.insert_agent(&agent).map_err(|e| e.to_string())?;
    Ok(agent)
}

// -----------------------------------------------------------------------------
// PRD-06 §3.3: agent directory (1:N) management — add / remove / update.
// The Agent owns multiple directories; these let the user manage them without
// touching the disk files themselves (removal only unbinds the record).
// -----------------------------------------------------------------------------

#[tauri::command]
pub fn add_agent_directory(
    agent_id: String,
    path: String,
    role: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::models::AgentDirectory, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Validate the agent exists.
    let agent = db
        .get_agent_by_id(&agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent 不存在：{agent_id}"))?;

    let expanded = expand_path(&path);

    // Default role: "skills" (the most common case) if the caller omits it.
    let role = role.unwrap_or_else(|| "skills".to_string());
    let dir_id = format!("adir-{agent_id}-{}", crate::db::new_id());
    let dir = crate::models::AgentDirectory {
        id: dir_id,
        agent_id: agent.id.clone(),
        path: expanded,
        role: Some(role),
        is_enabled: true,
        created_at: current_timestamp(),
    };
    db.insert_agent_directory(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
pub fn remove_agent_directory(
    directory_id: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    // PRD-06 §3.3e: only unbinds the directory record (and its sync_targets);
    // never deletes files on disk. Returns the count of sync_targets removed so
    // the UI can warn the user if skills were detached.
    let db = state.db.lock().map_err(|e| e.to_string())?;
    // Best-effort: if we can resolve the directory's agent, clear that agent's
    // cached scan results so the removed directory no longer appears in the UI.
    if let Ok(Some(dir)) = db.get_agent_directory_by_id(&directory_id) {
        let _ = invalidate_directory_skill_cache(&db, Some(&dir.agent_id));
    }
    db.delete_agent_directory(&directory_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_agent_directory(
    directory_id: String,
    role: Option<String>,
    is_enabled: Option<bool>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    if let Some(role) = role {
        db.update_agent_directory_role(&directory_id, Some(&role))
            .map_err(|e| e.to_string())?;
    }
    if let Some(enabled) = is_enabled {
        db.update_agent_directory_enabled(&directory_id, enabled)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

