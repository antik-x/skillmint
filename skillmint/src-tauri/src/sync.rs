use std::path::PathBuf;

use anyhow::Result;

use crate::db::Db;
use crate::fs::{
    compute_hash, copy_dir_all, create_symlink_or_copy, is_broken_symlink, is_multi_version,
    remove_path, replace_path_atomic, resolve_symlink, snapshot_version, snapshot_version_with_note,
    get_effective_skill_dir, LATEST_VERSION,
};
use crate::models::{
    Agent, ResolvedSkill, Skill, SkillSource, SyncFailure, SyncMode, SyncStatus, SyncTarget,
};

pub fn evaluate_sync_target(
    target: &SyncTarget,
    skill: &Skill,
    agent: &Agent,
) -> Result<SyncStatus> {
    let center_path = PathBuf::from(&skill.repo_path);
    let agent_path = agent.skill_directory.join(&skill.name);

    if !center_path.exists() {
        return Ok(SyncStatus::Broken);
    }

    if is_broken_symlink(&agent_path) {
        return Ok(SyncStatus::Broken);
    }

    if !agent_path.exists() {
        return Ok(SyncStatus::CenterChanged);
    }

    let agent_real = resolve_symlink(&agent_path);

    if target.mode == SyncMode::Symlink && agent_real == center_path {
        return Ok(SyncStatus::Synced);
    }

    let center_hash = compute_hash(&center_path)?;
    let agent_hash = compute_hash(&agent_real)?;

    if center_hash == agent_hash {
        return Ok(SyncStatus::Synced);
    }

    // For copy mode, if status is already CenterChanged and hashes still differ,
    // it means the local agent copy was also modified -> conflict.
    // Otherwise, any hash mismatch is treated as a local change because the center
    // repo is the single source of truth.
    if target.mode == SyncMode::Copy && target.status == SyncStatus::CenterChanged {
        Ok(SyncStatus::Conflict)
    } else {
        Ok(SyncStatus::LocalChanged)
    }
}

pub fn apply_sync_target(target: &SyncTarget, skill: &Skill, agent: &Agent) -> Result<SyncStatus> {
    let center_path = PathBuf::from(&skill.repo_path);
    let agent_path = agent.skill_directory.join(&skill.name);

    match target.mode {
        SyncMode::Symlink => {
            if agent_path.exists() || agent_path.is_symlink() {
                remove_path(&agent_path)?;
            }
            create_symlink_or_copy(&center_path, &agent_path)?;
        }
        SyncMode::Copy => {
            if agent_path.exists() || agent_path.is_symlink() {
                remove_path(&agent_path)?;
            }
            copy_dir_all(&center_path, &agent_path)?;
        }
    }

    Ok(SyncStatus::Synced)
}

/// Apply a sync target and persist both the resulting status and the current
/// timestamp as `last_sync_at`. Use this instead of `apply_sync_target` when
/// the target already exists in the database so the dashboard reflects the
/// last successful sync.
pub fn apply_sync_target_and_record(
    db: &Db,
    target: &SyncTarget,
    skill: &Skill,
    agent: &Agent,
) -> Result<SyncStatus> {
    let status = apply_sync_target(target, skill, agent)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    db.update_sync_target_synced(&target.id, status, now)?;
    Ok(status)
}

/// G3-③: before the first overwrite of a skill's `latest`, automatically
/// snapshot the current content as `v1/` so the user always has a rollback
/// point. This is a one-time safety net: once any `v<x>/` exists, it is a
/// no-op.
pub fn ensure_first_version_snapshot(skill_root: &std::path::Path) -> Result<Option<String>> {
    if is_multi_version(skill_root) {
        return Ok(None);
    }
    let label = snapshot_version_with_note(skill_root, Some("同步前自动留存"))?;
    Ok(Some(label))
}

/// Build a SyncFailure with a recovery hint inferred from the error text.
pub fn sync_failure_from_error(target: &SyncTarget, error: anyhow::Error) -> SyncFailure {
    let err_text = error.to_string();
    let recovery_hint = if err_text.contains("权限") || err_text.contains("Permission") || err_text.contains("denied") || err_text.contains("readonly") {
        Some("请检查 Agent 目录的写权限（chmod/所有者）".to_string())
    } else if err_text.contains("Broken") || err_text.contains("目标路径已失效") || err_text.contains("No such file") {
        Some("目标路径已失效，请在 Skill 详情重新绑定或恢复该目录".to_string())
    } else {
        None
    };
    SyncFailure {
        target_id: target.id.clone(),
        skill_id: target.skill_id.clone(),
        skill_name: target.skill_name.clone(),
        agent_id: target.agent_id.clone(),
        agent_name: target.agent_name.clone(),
        error: err_text,
        recovery_hint,
    }
}

/// Result of a `sync_all` run: successfully updated targets and failures.
pub struct SyncReport {
    pub updated: Vec<SyncTarget>,
    pub broken: Vec<crate::models::SyncFailure>,
}

pub fn sync_all(db: &Db) -> Result<SyncReport> {
    let skills = db.get_skills()?;
    let agents: std::collections::HashMap<String, Agent> = db
        .get_agents()?
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();

    let targets = db.get_sync_targets()?;
    let mut updated = Vec::new();
    let mut broken = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for target in targets {
        let skill = match skills.iter().find(|s| s.id == target.skill_id) {
            Some(s) => s,
            None => continue,
        };
        let agent = match agents.get(&target.agent_id) {
            Some(a) => a,
            None => continue,
        };

        let status = match evaluate_sync_target(&target, skill, agent) {
            Ok(s) => s,
            Err(e) => {
                broken.push(sync_failure_from_error(&target, e));
                continue;
            }
        };
        if status != target.status {
            if let Err(e) = db.update_sync_target_status(&target.id, status) {
                broken.push(sync_failure_from_error(&target, e.into()));
                continue;
            }
        }

        if status == SyncStatus::CenterChanged || status == SyncStatus::Broken {
            match apply_sync_target_and_record(db, &target, skill, agent) {
                Ok(new_status) => {
                    updated.push(SyncTarget {
                        status: new_status,
                        last_sync_at: Some(now),
                        ..target
                    });
                }
                Err(e) => {
                    broken.push(sync_failure_from_error(&target, e));
                }
            }
        } else if status == SyncStatus::Synced {
            // Even when nothing changed, record that the sync check ran now.
            // Otherwise the dashboard keeps saying "last sync 20614 days ago"
            // after a successful sync where no files needed writing.
            db.update_sync_target_synced(&target.id, SyncStatus::Synced, now)?;
            updated.push(SyncTarget {
                status,
                last_sync_at: Some(now),
                ..target
            });
        } else {
            updated.push(SyncTarget { status, ..target });
        }
    }

    Ok(SyncReport { updated, broken })
}

/// SPEC-F3: run a one-off consistency check at startup. Compares disk state
/// against every sync target and updates the DB when drift is detected.
/// Runs in a background thread and never blocks startup.
pub fn run_startup_consistency_check(db: Db) {
    std::thread::spawn(move || {
        let skills = match db.get_skills() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[startup-check] failed to load skills: {e}");
                return;
            }
        };
        let agents: std::collections::HashMap<String, Agent> = match db.get_agents() {
            Ok(a) => a.into_iter().map(|a| (a.id.clone(), a)).collect(),
            Err(e) => {
                eprintln!("[startup-check] failed to load agents: {e}");
                return;
            }
        };
        let targets = match db.get_sync_targets() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[startup-check] failed to load sync targets: {e}");
                return;
            }
        };
        for target in targets {
            let skill = match skills.iter().find(|s| s.id == target.skill_id) {
                Some(s) => s,
                None => continue,
            };
            let agent = match agents.get(&target.agent_id) {
                Some(a) => a,
                None => continue,
            };
            let status = match evaluate_sync_target(&target, skill, agent) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "[startup-check] failed to evaluate {} -> {}: {e}",
                        target.skill_name.as_deref().unwrap_or("?"),
                        target.agent_name.as_deref().unwrap_or("?")
                    );
                    continue;
                }
            };
            if status != target.status {
                if let Err(e) = db.update_sync_target_status(&target.id, status) {
                    eprintln!("[startup-check] failed to update status for {}: {e}", target.id);
                } else {
                    eprintln!(
                        "[startup-check] drift corrected: {} -> {} is now {:?}",
                        target.skill_name.as_deref().unwrap_or("?"),
                        target.agent_name.as_deref().unwrap_or("?"),
                        status
                    );
                }
            }
        }
    });
}

// =============================================================================
// PRD-01 patch FR-C/FR-F: skill resolution + multi-version diff handling
// =============================================================================

/// Resolve one skill physically present in an agent directory (FR-C).
///
/// Determines source type (symlink / local_copy / local / broken), the symlink
/// target if any, and whether the content matches the center-repo version.
/// When `pinned_version` is Some, the comparison is made against that snapshot;
/// otherwise it compares against `latest` (or the flat root for legacy skills).
/// Does NOT touch the DB; callers enrich `is_registered`/`pinned_version` separately.
pub fn resolve_skill_link(
    agent_dir: &std::path::Path,
    skill_name: &str,
    center_repo: &std::path::Path,
    project_skill_dir_name: &str,
    pinned_version: Option<&str>,
) -> Result<ResolvedSkill> {
    let agent_path = agent_dir.join(skill_name);

    // Broken symlink / missing path.
    if is_broken_symlink(&agent_path) || !agent_path.exists() && !agent_path.is_symlink() {
        return Ok(ResolvedSkill {
            name: skill_name.to_string(),
            source: SkillSource::Broken.to_string(),
            target_path: None,
            content_match: None,
            is_registered: false,
            pinned_version: None,
        });
    }

    let is_symlink = agent_path.is_symlink();
    let target = if is_symlink {
        Some(resolve_symlink(&agent_path))
    } else {
        None
    };

    // Determine whether this is a project-local skill (.skillmint/skills/...).
    // We treat a copy as "local" when its canonical path sits under a
    // `<project>/<project_skill_dir_name>` segment.
    let canonical = std::fs::canonicalize(&agent_path).unwrap_or_else(|_| agent_path.clone());
    // Treat a copy as project-local when its canonical path contains the
    // project skill dir name (e.g. ".skillmint/skills").
    let dir_marker = project_skill_dir_name.replace('\\', "/");
    let is_project_local = canonical
        .to_string_lossy()
        .contains(&dir_marker);

    let source = if is_symlink {
        SkillSource::Symlink
    } else if is_project_local {
        SkillSource::Local
    } else {
        SkillSource::LocalCopy
    };

    // Content match against the relevant center-repo version.
    // If the binding pins a version, compare to that snapshot; otherwise latest.
    let center_skill_root = center_repo.join(skill_name);
    let content_match = if center_skill_root.exists() {
        let center_compare = get_effective_skill_dir(&center_skill_root, pinned_version);
        let center_compare = if center_compare.exists() {
            Some(center_compare)
        } else if pinned_version.is_some() {
            // Pinned version missing: fall back to latest/flat so we still produce a match value.
            let latest = get_effective_skill_dir(&center_skill_root, None);
            if latest.exists() { Some(latest) } else { Some(center_skill_root.clone()) }
        } else {
            Some(center_skill_root.clone())
        };
        let agent_real = resolve_symlink(&agent_path);
        match (compute_hash(center_compare.as_ref().unwrap()), compute_hash(&agent_real)) {
            (Ok(a), Ok(b)) => Some(a == b),
            _ => None,
        }
    } else {
        None
    };

    Ok(ResolvedSkill {
        name: skill_name.to_string(),
        source: source.to_string(),
        target_path: target.map(|p| p.to_string_lossy().to_string()),
        content_match,
        is_registered: false,
        pinned_version: None,
    })
}

/// Outcome of [`resolve_skill_diff`]: where things landed after the operation.
pub struct DiffOutcome {
    pub new_version: Option<String>,
    pub project_dir: PathBuf,
    pub latest_dir: PathBuf,
}

/// Execute a diff resolution strategy (FR-F §4.5c).
///
/// - `KeepCenter`: overwrite the project copy with center `latest`.
/// - `KeepProject`: overwrite center `latest` with the project copy. If `backup`
///   is true, the old `latest` is snapshotted to a new `v<N>` first (留底,
///   PRD §4.5c). The returned `new_version` is that backup label.
/// - `Versionize`: snapshot center `latest` → new `v<N>` (with optional `note`
///   sidecar, PRD §4.5c), relink project copy to that snapshot, and pin the
///   binding (caller updates DB).
///
/// `project_skill_path` is the skill's path inside the agent/project directory
/// (the divergent copy). `skill` is the center-repo skill record.
pub fn resolve_skill_diff(
    strategy: crate::models::DiffStrategy,
    skill: &Skill,
    project_skill_path: &std::path::Path,
    _agent: &Agent,
    db: &Db,
    note: Option<&str>,
    backup: bool,
) -> Result<DiffOutcome> {
    let center_root = PathBuf::from(&skill.repo_path);
    let latest_dir = get_effective_skill_dir(&center_root, None);
    // Ensure latest_dir is real (promote flat layout if needed for KeepProject/Versionize).
    let ensure_latest = |root: &std::path::Path| -> Result<PathBuf> {
        let latest = root.join(LATEST_VERSION);
        if latest.is_dir() {
            return Ok(latest);
        }
        // Promote flat → multi-version.
        let _ = snapshot_version(root)?;
        Ok(root.join(LATEST_VERSION))
    };

    match strategy {
        crate::models::DiffStrategy::KeepCenter => {
            // Overwrite project copy with center latest (or flat root if no versions).
            let src = if latest_dir.exists() {
                latest_dir.clone()
            } else {
                center_root.clone()
            };
            // Stage the new project copy next to its final path, then switch
            // atomically so a failure never leaves the project copy missing.
            let staging = project_skill_path.with_extension("skillmint-staging");
            if staging.exists() || staging.is_symlink() {
                remove_path(&staging)?;
            }
            create_symlink_or_copy(&src, &staging)?;
            replace_path_atomic(&staging, project_skill_path)?;
            Ok(DiffOutcome {
                new_version: None,
                project_dir: project_skill_path.to_path_buf(),
                latest_dir: src,
            })
        }
        crate::models::DiffStrategy::KeepProject => {
            // Overwrite center latest with the project copy (feed back upstream).
            // Capture the project copy's REAL content BEFORE any layout promotion.
            let project_real = resolve_symlink(project_skill_path);

            // Promote flat → multi-version first so we write into latest/.
            let latest = if crate::fs::is_multi_version(&center_root) {
                latest_dir
            } else {
                ensure_latest(&center_root)?
            };

            // PRD §4.5c: optionally snapshot the OLD latest before overwriting (留底).
            let backup_label = if backup {
                Some(snapshot_version(&center_root)?)
            } else {
                None
            };

            // Stage the new latest content outside the center repo, then atomically
            // switch latest/ to it. A crash leaves the old latest untouched.
            let staging = center_root.with_extension("skillmint-latest-staging");
            if staging.exists() || staging.is_symlink() {
                remove_path(&staging)?;
            }
            copy_dir_all(&project_real, &staging)?;
            replace_path_atomic(&staging, &latest)?;

            // Relink project copy to the (now-updated) latest so they stay in sync.
            let project_staging = project_skill_path.with_extension("skillmint-staging");
            if project_staging.exists() || project_staging.is_symlink() {
                remove_path(&project_staging)?;
            }
            create_symlink_or_copy(&latest, &project_staging)?;
            replace_path_atomic(&project_staging, project_skill_path)?;
            Ok(DiffOutcome {
                new_version: backup_label,
                project_dir: project_skill_path.to_path_buf(),
                latest_dir: latest,
            })
        }
        crate::models::DiffStrategy::Versionize => {
            // Snapshot current latest → new v<N>, relink project copy to the snapshot.
            // snapshot_version handles flat→multi-version promotion internally using
            // copy-in semantics, so the original skill root is never at risk.
            // PRD §4.5c: attach the optional note sidecar to the new snapshot.
            let new_label = snapshot_version_with_note(&center_root, note)?;
            let pinned_dir = center_root.join(&new_label);
            let project_staging = project_skill_path.with_extension("skillmint-staging");
            if project_staging.exists() || project_staging.is_symlink() {
                remove_path(&project_staging)?;
            }
            create_symlink_or_copy(&pinned_dir, &project_staging)?;
            replace_path_atomic(&project_staging, project_skill_path)?;
            // Caller pins the binding; we just report the new label.
            let _ = db; // DB pin handled by command layer.
            let latest_after = get_effective_skill_dir(&center_root, None);
            Ok(DiffOutcome {
                new_version: Some(new_label),
                project_dir: project_skill_path.to_path_buf(),
                latest_dir: latest_after,
            })
        }
    }
}

