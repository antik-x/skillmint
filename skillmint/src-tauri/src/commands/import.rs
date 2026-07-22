use super::*;

/// When the center repo has no skills yet, discover and import skills from enabled
/// agent directories. Returns (imported_count, conflict_count).
pub(crate) fn import_all_agent_skills(
    db: &crate::db::Db,
    settings: &Settings,
) -> anyhow::Result<(usize, usize)> {
    let existing_skills = db.get_skills()?;
    if !existing_skills.is_empty() {
        return Ok((0, 0));
    }

    let agents = db.get_agents()?;
    let enabled_agents: Vec<&Agent> = agents.iter().filter(|a| a.is_enabled).collect();

    // name -> (canonical source path, agent id)
    let mut discovered: std::collections::HashMap<String, (PathBuf, String)> =
        std::collections::HashMap::new();
    let mut conflicts: std::collections::HashSet<String> = std::collections::HashSet::new();

    for agent in &enabled_agents {
        if let Ok(entries) = std::fs::read_dir(&agent.skill_directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                // P0-3: only real skills (SKILL.md present), never excluded names.
                if name.is_empty()
                    || crate::scan::is_excluded_scan_name(&name, &settings.scan_exclude_names)
                    || !crate::scan::is_skill_dir(&path)
                {
                    continue;
                }

                let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());

                if let Some((existing_path, _)) = discovered.get(&name) {
                    if existing_path != &canonical {
                        let existing_hash =
                            crate::fs::compute_hash(existing_path).unwrap_or_default();
                        let new_hash = crate::fs::compute_hash(&canonical).unwrap_or_default();
                        if existing_hash != new_hash {
                            conflicts.insert(name);
                        }
                    }
                } else {
                    discovered.insert(name, (canonical, agent.id.clone()));
                }
            }
        }
    }

    let mut imported = 0;
    let mut imported_names: Vec<String> = Vec::new();
    for (name, (source, _agent_id)) in discovered {
        if conflicts.contains(&name) {
            continue;
        }

        let center_dest = settings.center_repo.join(&name);
        if center_dest.exists() {
            continue;
        }

        copy_dir_all(&source, &center_dest)?;

        let now = current_timestamp();
        let skill = Skill {
            id: new_id(),
            name: name.clone(),
            repo_path: center_dest,
            created_at: now,
            updated_at: now,
            status: crate::models::SkillStatus::Draft,
        };
        db.insert_skill(&skill)?;
        imported += 1;
        imported_names.push(name);
    }

    // P2-1: auto-commit the batch when enabled (no-op otherwise).
    maybe_auto_commit_after_import(settings, &imported_names);

    Ok((imported, conflicts.len()))
}

#[tauri::command]
pub fn import_skill(
    agent_id: String,
    skill_name: String,
    resolution: Option<String>,
    state: State<'_, AppState>,
) -> Result<Skill, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    let agent = db
        .get_agents()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or("Agent not found")?;

    let source = agent.skill_directory.join(&skill_name);
    let center_dest = settings.center_repo.join(&skill_name);

    // P0-2: an agent-side symlink that already points at this center directory
    // is a healthy synced link — nothing to copy or resolve. Skip the content
    // comparison (its hashes can never match, see classify_agent_skill_entry)
    // and go straight to (re-)registration below. This also removes the dead
    // end where importing such an item demanded a "local/center" resolution
    // that would have replaced the link with an entity copy.
    let already_linked = crate::fs::is_symlink_to(&source, &center_dest);

    if already_linked {
        // Already synced via symlink; fall through to registration.
    } else if center_dest.exists() {
        let agent_hash = crate::fs::compute_hash(&source).map_err(|e| e.to_string())?;
        let center_hash = crate::fs::compute_hash(&center_dest).map_err(|e| e.to_string())?;

        if agent_hash != center_hash {
            let choice = resolution.as_deref().unwrap_or("");
            if choice == "local" {
                // Replace center with local
                let _ = remove_path(&center_dest);
                copy_dir_all(&source, &center_dest).map_err(|e| e.to_string())?;
            } else if choice == "center" {
                // Keep center, will re-sync to agent later
            } else {
                return Err(format!(
                    "Skill '{}' already exists in center repo with different content. Please choose 'local' or 'center'.",
                    skill_name
                ));
            }
        }
    } else {
        copy_dir_all(&source, &center_dest).map_err(|e| e.to_string())?;
    }

    let now = current_timestamp();
    let skill = Skill {
        id: new_id(),
        name: skill_name.clone(),
        repo_path: center_dest,
        created_at: now,
        updated_at: now,
        status: crate::models::SkillStatus::Draft,
    };

    // If a skill with same name already exists in DB, update it
    if let Some(existing) = db
        .get_skill_by_name(&skill_name)
        .map_err(|e| e.to_string())?
    {
        let _ = db.delete_skill(&existing.id);
    }

    db.insert_skill(&skill).map_err(|e| e.to_string())?;

    // Create or update sync target for this agent
    let existing_target = db
        .get_sync_targets()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.skill_id == skill.id && t.agent_id == agent_id);

    if existing_target.is_none() {
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
    }

    // Apply sync
    let target = db
        .get_sync_targets()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.skill_id == skill.id && t.agent_id == agent_id)
        .ok_or("Failed to create sync target")?;
    apply_sync_target_and_record(&db, &target, &skill, &agent).map_err(|e| e.to_string())?;

    let _ = invalidate_directory_skill_cache(&db, Some(&agent_id));
    // P2-1: auto-commit when enabled (no-op otherwise).
    maybe_auto_commit_after_import(&settings, std::slice::from_ref(&skill_name));
    Ok(skill)
}

#[tauri::command]
pub fn add_skill(
    source_path: String,
    agent_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Skill, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;

    let source = expand_path(&source_path);
    let dest = move_into_center(&source, &settings.center_repo).map_err(|e| e.to_string())?;

    let now = current_timestamp();
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return Err("无法解析 Skill 名称".to_string());
    }
    let skill = Skill {
        id: new_id(),
        name,
        repo_path: dest,
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

    // PRD-03: auto-trigger knowledge-graph extraction on skill creation (non-blocking best-effort).
    trigger_kg_analysis(&db, &settings.device_id, &skill);
    let _ = invalidate_directory_skill_cache(&db, None);
    Ok(skill)
}

/// FR-D: install a center-repo skill into a project's agent(s) via symlink
/// (fallback copy). Persists a binding AND creates the physical link atomically.
/// Core logic of `install_skill_to_project`: create symlinks/copies + bindings.
pub fn install_skill_core(
    db: &crate::db::Db,
    device_id: &str,
    skill_id: &str,
    project_id: &str,
    agent_ids: &[String],
    mode: &str,
) -> Result<Vec<SkillProjectBinding>, anyhow::Error> {
    let skill = db
        .get_skill_by_id(skill_id)?
        .ok_or_else(|| anyhow::anyhow!("Skill not found"))?;
    let skill_root = PathBuf::from(&skill.repo_path);
    let source_dir = get_effective_skill_dir(&skill_root, None);

    // Normalize mode for storage: DB CHECK allows ('local_copy','symlink','reference').
    // Frontend sends 'symlink'/'copy'; map 'copy' -> 'local_copy'.
    let stored_mode = if mode == "copy" { "local_copy" } else { mode };
    let physical_copy = stored_mode == "local_copy";

    let agents = db.get_agents()?;
    let mut created = Vec::new();
    for aid in agent_ids {
        let agent = match agents.iter().find(|a| a.id == *aid) {
            Some(a) => a,
            None => continue,
        };
        let dst = agent.skill_directory.join(&skill.name);
        if dst.exists() || dst.is_symlink() {
            anyhow::bail!(
                "{} 下已存在 {}，请先移除或改用「处理差异」",
                agent.name,
                skill.name
            );
        }
        if physical_copy {
            copy_dir_all(&source_dir, &dst)?;
        } else {
            crate::fs::create_symlink_or_copy(&source_dir, &dst)?;
        }
        let binding = SkillProjectBinding {
            id: new_id(),
            device_id: device_id.to_string(),
            skill_id: skill.id.clone(),
            skill_name: Some(skill.name.clone()),
            project_id: Some(project_id.to_string()),
            project_name: None,
            agent_id: Some(agent.id.clone()),
            mode: stored_mode.to_string(),
            local_path: Some(dst.to_string_lossy().to_string()),
            is_enabled: true,
            pinned_version: None,
        };
        db.upsert_skill_project_binding(&binding)?;
        created.push(binding);
    }
    Ok(created)
}

