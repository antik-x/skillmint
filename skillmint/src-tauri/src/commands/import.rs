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

