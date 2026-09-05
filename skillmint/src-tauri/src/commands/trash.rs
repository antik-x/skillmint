use super::*;

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
    let hub = crate::hub::global_hub_dir();
    restore_trash_item_impl(&db, &settings, &hub, id, conflict_strategy)
}

/// Testable core of [`restore_trash_item`]. P3 review fix (B3): the center
/// repo is retired, so restore means CONTENT RECOVERY into the global private
/// hub — getting the skill back into agent dirs is a separate, explicit
/// `npx skills add`. No skills-table/sync-target/binding rebuild happens here.
///
/// `hub` is passed explicitly so tests can point at a temp directory.
pub(crate) fn restore_trash_item_impl(
    db: &crate::db::Db,
    settings: &Settings,
    hub: &std::path::Path,
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

    crate::hub::ensure_hub(hub, "global").map_err(|e| e.to_string())?;

    let final_name = match hub.join(original_name).exists() {
        false => original_name.to_string(),
        true => match conflict_strategy.as_deref() {
            Some("overwrite") => {
                // Keep the clashing hub skill reversible: snapshot it into the
                // trash root next to the hub (~/.skillmint/trash in prod).
                let clash_dir = hub.join(original_name);
                let trash_root =
                    hub.parent().unwrap_or_else(|| std::path::Path::new(".")).join("trash");
                let now = current_timestamp();
                let snapshot = trash_root.join(format!("hub-{}-{}", original_name, now));
                std::fs::create_dir_all(&trash_root).map_err(|e| e.to_string())?;
                crate::fs::copy_dir_all(&clash_dir, &snapshot)
                    .map_err(|e| format!("重名快照失败，已中止恢复：{}", e))?;
                let expires_at =
                    now + (crate::discovery::config::DISMISS_COOLING_DAYS as u64) * 86400;
                let _ = db.insert_trash_item(
                    "skill",
                    &format!("hub-{}", original_name),
                    original_name,
                    &snapshot.to_string_lossy(),
                    &serde_json::json!({ "source": "hub_overwrite" }),
                    now,
                    expires_at,
                );
                let _ = std::fs::remove_dir_all(&clash_dir);
                original_name.to_string()
            }
            Some("rename") => format!("{}-restored", original_name),
            _ => {
                return Err(format!(
                    "restore_conflict: Hub 中已存在名为 '{}' 的目录，请选择覆盖/重命名策略",
                    original_name
                ));
            }
        },
    };

    let repo_path = hub.join(&final_name);
    if repo_path.exists() {
        let _ = std::fs::remove_dir_all(&repo_path);
    }
    crate::fs::copy_dir_all(&item.snapshot_path, &repo_path)
        .map_err(|e| format!("找回快照失败：{}", e))?;
    crate::hub::auto_commit(hub, &format!("restore: recover {} from trash", final_name))
        .map_err(|e| e.to_string())?;

    // Drop the trash row + snapshot now that the recovery succeeded.
    let _ = std::fs::remove_dir_all(&item.snapshot_path);
    db.delete_trash_item(id).map_err(|e| e.to_string())?;

    Ok(RestoreResult {
        restored_id: original_id.to_string(),
        skipped_bindings: Vec::new(),
        final_name,
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

