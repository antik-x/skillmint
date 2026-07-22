use super::*;

#[tauri::command]
pub fn remove_skill(skill_id: String, state: State<'_, AppState>) -> Result<i64, String> {
    // SPEC-C3 T1: remove_skill is now "move to recycle bin". The skill and its
    // related rows (sync_targets, skill_project_bindings) are snapshotted to a
    // trash directory BEFORE any deletion; if the snapshot fails nothing is
    // touched. Returns the trash item id so the frontend can offer undo.
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    remove_skill_impl(&db, &settings, &skill_id)
}

/// Testable core of [`remove_skill`] that takes borrowed db + settings.
pub(crate) fn remove_skill_impl(
    db: &crate::db::Db,
    settings: &Settings,
    skill_id: &str,
) -> Result<i64, String> {
    let skill = db
        .get_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == skill_id)
        .ok_or("Skill not found".to_string())?;

    let targets = db.get_sync_targets().map_err(|e| e.to_string())?;
    let agents = db.get_agents().map_err(|e| e.to_string())?;

    // 1. Snapshot the center-repo skill directory into the trash bin. We never
    //    delete first: if the snapshot fails the user keeps their data.
    let trash_root = settings.center_repo.parent().unwrap_or_else(|| Path::new(".")).join("trash");
    let now = current_timestamp();
    let snapshot_dir = trash_root.join(format!("{}-{}", skill.id, now));
    std::fs::create_dir_all(&trash_root).map_err(|e| format!("无法创建回收站目录：{}", e))?;
    crate::fs::copy_dir_all(&skill.repo_path, &snapshot_dir)
        .map_err(|e| format!("快照失败，已中止删除（数据未改动）：{}", e))?;

    // 2. Serialize the related rows we will need to rebuild on restore.
    let sync_targets = db
        .raw_sync_targets_for_skill(&skill_id)
        .map_err(|e| e.to_string())?;
    let bindings = db
        .raw_bindings_for_skill(&skill_id)
        .map_err(|e| e.to_string())?;
    let skill_row = serde_json::json!({
        "id": skill.id,
        "name": skill.name,
        "repo_path": skill.repo_path.to_string_lossy(),
        "created_at": skill.created_at,
        "updated_at": skill.updated_at,
        "status": skill.status.to_string(),
    });
    let metadata = serde_json::json!({
        "skill": skill_row,
        "sync_targets": sync_targets,
        "bindings": bindings,
    });

    let expires_at = now + (crate::discovery::config::DISMISS_COOLING_DAYS as u64) * 86400;
    let trash_id = db
        .insert_trash_item(
            "skill",
            &skill.id,
            &skill.name,
            &snapshot_dir.to_string_lossy(),
            &metadata,
            now,
            expires_at,
        )
        .map_err(|e| e.to_string())?;

    // 3. Only now perform the existing destructive removal: copy back to
    //    agents, delete DB rows, remove the center-repo directory.
    for target in targets.iter().filter(|t| t.skill_id == skill_id) {
        if let Some(agent) = agents.iter().find(|a| a.id == target.agent_id) {
            let agent_path = agent.skill_directory.join(&skill.name);
            if agent_path.exists() || agent_path.is_symlink() {
                let _ = remove_path(&agent_path);
            }
            let _ = copy_dir_all(&skill.repo_path, &agent_path);
        }
    }

    db.delete_skill(&skill_id).map_err(|e| e.to_string())?;
    let _ = remove_path(&skill.repo_path);

    let _ = invalidate_directory_skill_cache(&db, None);
    Ok(trash_id)
}

/// SPEC-C3 T2: list recycle-bin items. The frontend computes remaining days
/// from `expires_at` / `deleted_at`.
#[tauri::command]
pub fn list_trash_items(state: State<'_, AppState>) -> Result<Vec<TrashItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_trash_items().map_err(|e| e.to_string())
}

/// SPEC-C3 T2: restore a trashed skill.
///
/// `conflict_strategy`:
/// - `None` when there is no name clash (the common case).
/// - `Some("overwrite")`: the existing skill is first moved to the trash bin
///   (T1 path, fully reversible), then this item is restored.
/// - `Some("rename")`: restored as `<name>-restored` with a fresh id.
/// On a clash without a strategy the command returns a business error tagged
/// `restore_conflict` so the frontend can surface the three-way picker.
#[tauri::command]
pub fn restore_trash_item(
    id: i64,
    conflict_strategy: Option<String>,
    state: State<'_, AppState>,
) -> Result<RestoreResult, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    restore_trash_item_impl(&db, &settings, id, conflict_strategy)
}

/// Testable core of [`restore_trash_item`].
pub(crate) fn restore_trash_item_impl(
    db: &crate::db::Db,
    settings: &Settings,
    id: i64,
    conflict_strategy: Option<String>,
) -> Result<RestoreResult, String> {
    let item = db
        .get_trash_item(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "回收站项目不存在".to_string())?;

    let metadata = &item.metadata;
    let skill_row = metadata.get("skill").ok_or_else(|| "快照元数据损坏".to_string())?;
    let original_name = skill_row
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "快照元数据缺少 name".to_string())?;
    let original_id = skill_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "快照元数据缺少 id".to_string())?;

    // Detect a name clash against the live skills table.
    let clash = db
        .get_skill_by_name(original_name)
        .map_err(|e| e.to_string())?;

    let (final_name, final_id, repo_path) = match (&clash, conflict_strategy.as_deref()) {
        (None, _) => (original_name.to_string(), original_id.to_string(), settings.center_repo.join(original_name)),
        (Some(_), Some("overwrite")) => {
            // Move the clashing skill to the trash first (T1 path).
            drop_existing_skill_to_trash(&db, &settings, clash.as_ref().unwrap())?;
            (original_name.to_string(), original_id.to_string(), settings.center_repo.join(original_name))
        }
        (Some(_), Some("rename")) => {
            let new_name = format!("{}-restored", original_name);
            let new_id = crate::db::new_id();
            (new_name.clone(), new_id, settings.center_repo.join(&new_name))
        }
        (Some(_), _) => {
            return Err(format!(
                "restore_conflict: 名为 '{}' 的 Skill 已存在，请选择覆盖/重命名策略",
                original_name
            ));
        }
    };

    if repo_path.exists() {
        let _ = remove_path(&repo_path);
    }
    std::fs::create_dir_all(&repo_path).map_err(|e| e.to_string())?;
    crate::fs::copy_dir_all(&item.snapshot_path, &repo_path)
        .map_err(|e| format!("恢复快照失败：{}", e))?;

    // Rebuild the skills row.
    let now = current_timestamp();
    let status_str = skill_row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("draft");
    let status = match status_str {
        "approved" => crate::models::SkillStatus::Approved,
        "candidate" => crate::models::SkillStatus::Candidate,
        "deprecated" => crate::models::SkillStatus::Deprecated,
        _ => crate::models::SkillStatus::Draft,
    };
    let skill = crate::models::Skill {
        id: final_id.clone(),
        name: final_name.clone(),
        repo_path: repo_path.clone(),
        created_at: skill_row
            .get("created_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(now),
        updated_at: now,
        status,
    };
    db.insert_skill(&skill).map_err(|e| e.to_string())?;

    // Rebuild sync_targets: restore them in center_changed so the user
    // explicitly re-syncs (PRD: restore must never auto-write agent dirs).
    if let Some(arr) = metadata.get("sync_targets").and_then(|v| v.as_array()) {
        for t in arr {
            let agent_id = t.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            let mode = t.get("mode").and_then(|v| v.as_str()).unwrap_or("symlink");
            let new_target = crate::models::SyncTarget {
                id: crate::db::new_id(),
                skill_id: final_id.clone(),
                skill_name: Some(final_name.clone()),
                agent_id: agent_id.to_string(),
                agent_name: t.get("agent_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                mode: match mode {
                    "copy" | "local_copy" => crate::models::SyncMode::Copy,
                    _ => crate::models::SyncMode::Symlink,
                },
                last_sync_at: None,
                status: crate::models::SyncStatus::CenterChanged,
            };
            let _ = db.insert_sync_target(&new_target);
        }
    }

    // Rebuild project bindings; skip ones whose project no longer exists.
    let mut skipped: Vec<String> = Vec::new();
    if let Some(arr) = metadata.get("bindings").and_then(|v| v.as_array()) {
        let live_projects: std::collections::HashSet<String> = db
            .get_projects(&settings.device_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id, _name, _path, _last_active)| id)
            .collect();
        for b in arr {
            let pid = b.get("project_id").and_then(|v| v.as_str());
            if let Some(pid) = pid {
                if !live_projects.contains(pid) {
                    if let Some(name) = b.get("project_name").and_then(|v| v.as_str()) {
                        skipped.push(name.to_string());
                    } else {
                        skipped.push(pid.to_string());
                    }
                    continue;
                }
            }
            let binding_id = crate::db::new_id();
            let _ = db.conn_ref().execute(
                "INSERT OR REPLACE INTO skill_project_bindings
                 (id, device_id, skill_id, project_id, agent_id, mode, local_path, is_enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    binding_id,
                    settings.device_id,
                    final_id,
                    pid,
                    b.get("agent_id").and_then(|v| v.as_str()),
                    b.get("mode").and_then(|v| v.as_str()).unwrap_or("symlink"),
                    b.get("local_path").and_then(|v| v.as_str()),
                    if b.get("is_enabled").and_then(|v| v.as_bool()).unwrap_or(true) { 1 } else { 0 },
                    now,
                    now,
                ],
            );
        }
    }

    // Drop the trash row + snapshot now that the restore succeeded.
    let _ = std::fs::remove_dir_all(&item.snapshot_path);
    db.delete_trash_item(id).map_err(|e| e.to_string())?;

    let _ = invalidate_directory_skill_cache(&db, None);
    Ok(RestoreResult {
        restored_id: final_id,
        skipped_bindings: skipped,
        final_name: final_name.clone(),
    })
}

/// SPEC-C3 T2: permanently delete a trash item. Requires the same confirmation
/// code as "clear collected data" (irreversible operation, heavy confirm).
#[tauri::command]
pub fn purge_trash_item(
    id: i64,
    confirm: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    purge_trash_item_impl(&db, id, &confirm)
}

/// Testable core of [`purge_trash_item`].
pub(crate) fn purge_trash_item_impl(
    db: &crate::db::Db,
    id: i64,
    confirm: &str,
) -> Result<(), String> {
    if confirm != DATA_MANAGEMENT_CONFIRM_CODE {
        return Err(format!("确认码不正确，请输入 {}", DATA_MANAGEMENT_CONFIRM_CODE));
    }
    let item = db
        .get_trash_item(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "回收站项目不存在".to_string())?;
    let _ = std::fs::remove_dir_all(&item.snapshot_path);
    db.delete_trash_item(id).map_err(|e| e.to_string())?;
    Ok(())
}

/// SPEC-C3 T2 helper: move an existing live skill into the trash bin via the
/// T1 path. Used by the "overwrite" conflict strategy so overwrites are
/// bidirectionally reversible.
pub(crate) fn drop_existing_skill_to_trash(
    db: &crate::db::Db,
    settings: &Settings,
    skill: &Skill,
) -> Result<(), String> {
    let trash_root = settings.center_repo.parent().unwrap_or_else(|| Path::new(".")).join("trash");
    let now = current_timestamp();
    let snapshot_dir = trash_root.join(format!("{}-{}", skill.id, now));
    std::fs::create_dir_all(&trash_root).map_err(|e| e.to_string())?;
    crate::fs::copy_dir_all(&skill.repo_path, &snapshot_dir).map_err(|e| e.to_string())?;

    let sync_targets = db.raw_sync_targets_for_skill(&skill.id).map_err(|e| e.to_string())?;
    let bindings = db.raw_bindings_for_skill(&skill.id).map_err(|e| e.to_string())?;
    let metadata = serde_json::json!({
        "skill": serde_json::json!({
            "id": skill.id,
            "name": skill.name,
            "repo_path": skill.repo_path.to_string_lossy(),
            "created_at": skill.created_at,
            "updated_at": skill.updated_at,
            "status": skill.status.to_string(),
        }),
        "sync_targets": sync_targets,
        "bindings": bindings,
    });
    let expires_at = now + (crate::discovery::config::DISMISS_COOLING_DAYS as u64) * 86400;
    db.insert_trash_item(
        "skill",
        &skill.id,
        &skill.name,
        &snapshot_dir.to_string_lossy(),
        &metadata,
        now,
        expires_at,
    )
    .map_err(|e| e.to_string())?;

    // Remove the live rows (no agent copy-back: the skill is being overwritten).
    db.delete_skill(&skill.id).map_err(|e| e.to_string())?;
    let _ = remove_path(&skill.repo_path);
    Ok(())
}

