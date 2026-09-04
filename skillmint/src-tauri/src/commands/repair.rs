use super::*;

/// SPEC-F4 T12 / SPEC-F5 T5: migrate skills that live directly under the legacy
/// center root (`~/.skillmint/<skill>`) into the new repo subdir
/// (`~/.skillmint/repo/<skill>`) and update DB repo_path accordingly.
///
/// SPEC-F5 T5 fix: previously, when the target already existed this silently
/// skipped the DB update, so `repo_path` never converged. Now the move is
/// idempotent — if the target already exists (whether from a prior partial run
/// or a hand-placed copy), we still update the DB row so it points into the
/// repo. Structured per-skill outcomes are returned so the `repair_skill_paths`
/// command can surface them to the user.
pub fn migrate_skills_to_repo(
    db: &crate::db::Db,
    center_repo: &Path,
) -> Result<MigrationReport, String> {
    let skills = db.get_skills().map_err(|e| e.to_string())?;
    let Some(legacy_root) = center_repo.parent() else {
        return Ok(MigrationReport::default());
    };
    let mut report = MigrationReport::default();

    for mut skill in skills {
        let old_path = skill.repo_path.clone();
        if old_path.parent() != Some(legacy_root) {
            continue;
        }
        let new_path = center_repo.join(&skill.name);
        let outcome = migrate_one_skill(db, &mut skill, &old_path, &new_path);
        match outcome {
            MigrationOutcome::Done => report.migrated.push(skill.name.clone()),
            MigrationOutcome::Failed(reason) => report
                .failed
                .push(MigrationFailure {
                    skill: skill.name.clone(),
                    reason,
                }),
        }
    }
    Ok(report)
}

#[derive(Default, Clone, Debug)]
pub struct MigrationReport {
    pub migrated: Vec<String>,
    pub failed: Vec<MigrationFailure>,
}

#[derive(Clone, Debug)]
pub struct MigrationFailure {
    pub skill: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
enum MigrationOutcome {
    Done,
    Failed(String),
}

/// Move a single legacy skill dir into the repo and update its DB row.
/// Idempotent: if the target already exists we still update the DB so repo_path
/// converges; a hash mismatch (different content at the target) is recorded as a
/// failure rather than silently overwritten.
fn migrate_one_skill(
    db: &crate::db::Db,
    skill: &mut crate::models::Skill,
    old_path: &Path,
    new_path: &Path,
) -> MigrationOutcome {
    if old_path.exists() {
        if new_path.exists() {
            // Target already populated — verify it matches the legacy copy so we
            // don't silently discard divergent user edits.
            if !dirs_equivalent(old_path, new_path) {
                return MigrationOutcome::Failed(format!(
                    "目标已存在且内容不一致：{:?}",
                    new_path
                ));
            }
            // Content matches — remove the legacy copy and converge the DB path.
            if let Err(e) = std::fs::remove_dir_all(old_path) {
                eprintln!("[migrate] failed to remove legacy dir {:?}: {}", old_path, e);
                // Even if cleanup fails, the DB update is still the meaningful
                // action; fall through so repo_path converges.
            }
        } else if let Err(e) = std::fs::rename(old_path, new_path) {
            eprintln!("[migrate] failed to move {:?} -> {:?}: {}", old_path, new_path, e);
            return MigrationOutcome::Failed(format!("移动目录失败：{e}"));
        }
    }
    skill.repo_path = new_path.to_path_buf();
    if let Err(e) = db.update_skill(skill) {
        eprintln!("[migrate] failed to update DB for {}: {}", skill.name, e);
        return MigrationOutcome::Failed(format!("更新数据库失败：{e}"));
    }
    MigrationOutcome::Done
}

/// Lightweight equivalence check: equal file count and equal SKILL.md bytes (the
/// only file produced for prompt-derived skills). Good enough to distinguish a
/// prior partial migration from a genuinely divergent copy without pulling in a
/// full hashing dependency.
fn dirs_equivalent(a: &Path, b: &Path) -> bool {
    let count = |p: &Path| -> Option<usize> {
        Some(std::fs::read_dir(p).ok()?.count())
    };
    match (count(a), count(b)) {
        (Some(ca), Some(cb)) => {
            if ca != cb {
                return false;
            }
            let read = |p: &Path| std::fs::read(p).unwrap_or_default();
            read(&a.join("SKILL.md")) == read(&b.join("SKILL.md"))
        }
        _ => false,
    }
}

/// F6: check whether the center repo is missing/empty while the DB still has
/// skills. Used by the frontend to show a recovery banner.
/// Retired as a command in P3-6b; kept as the repair-flow test oracle.
#[allow(dead_code)]
pub(crate) fn check_repo_integrity_inner(
    db: &crate::db::Db,
    settings: &crate::settings::Settings,
) -> Result<RepoIntegrity, String> {
    let has_skills = !db.get_skills().map_err(|e| e.to_string())?.is_empty();
    let repo_exists = settings.center_repo.exists();
    let repo_empty = if repo_exists {
        std::fs::read_dir(&settings.center_repo)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    } else {
        true
    };
    if has_skills && (!repo_exists || repo_empty) {
        return Ok(RepoIntegrity::MissingWithRecords);
    }
    // P1-5 reverse check: center has unregistered skill dirs (orphans).
    if repo_exists && !repo_empty {
        if !list_center_orphans(db, settings)?.is_empty() {
            return Ok(RepoIntegrity::OrphansPresent);
        }
    }
    Ok(RepoIntegrity::Healthy)
}

/// Undo record for one agent-side mutation performed during a rename.
enum AgentUndo {
    None,
    /// Created `created` as a new symlink; undo removes it.
    RemoveCreated { created: PathBuf },
    /// Replaced an old symlink with `created`; undo removes `created` and
    /// restores a link at `old_path` -> `old_target`.
    RestoreLink {
        created: PathBuf,
        old_path: PathBuf,
        old_target: PathBuf,
    },
    /// Renamed `original` to `current`; undo renames it back.
    RenameBack { current: PathBuf, original: PathBuf },
    /// Removed a stale symlink at `path` (-> `target`); undo recreates it.
    RecreateLink { path: PathBuf, target: PathBuf },
}

/// Apply one agent-side rename mutation for sync target `mode`.
/// Symlink targets get a fresh new-name link (entity dirs are moved, never
/// deleted); copy targets only have their directory renamed.
fn retarget_agent_entry(
    mode: SyncMode,
    agent_dir: &std::path::Path,
    old_name: &str,
    new_name: &str,
    old_dir: &std::path::Path,
    new_dir: &std::path::Path,
) -> anyhow::Result<AgentUndo> {
    let agent_old = agent_dir.join(old_name);
    let agent_new = agent_dir.join(new_name);
    match mode {
        SyncMode::Symlink => {
            if agent_old.is_symlink() {
                remove_path(&agent_old)?;
                if let Err(e) = crate::fs::create_symlink_strict(new_dir, &agent_new) {
                    // The old link is already gone — restore it on the spot so
                    // a failed rename never leaves the agent side worse off.
                    #[cfg(unix)]
                    let _ = std::os::unix::fs::symlink(old_dir, &agent_old);
                    return Err(e);
                }
                Ok(AgentUndo::RestoreLink {
                    created: agent_new,
                    old_path: agent_old,
                    old_target: old_dir.to_path_buf(),
                })
            } else if agent_old.is_dir() {
                std::fs::rename(&agent_old, &agent_new)?;
                Ok(AgentUndo::RenameBack {
                    current: agent_new,
                    original: agent_old,
                })
            } else {
                // Nothing on disk: create the link the target implies.
                crate::fs::create_symlink_strict(new_dir, &agent_new)?;
                Ok(AgentUndo::RemoveCreated { created: agent_new })
            }
        }
        SyncMode::Copy => {
            if agent_old.is_symlink() {
                // Stale link, dangling since the center rename; copy mode
                // never creates links — drop it, the next sync copies fresh.
                remove_path(&agent_old)?;
                Ok(AgentUndo::RecreateLink {
                    path: agent_old,
                    target: old_dir.to_path_buf(),
                })
            } else if agent_old.is_dir() {
                std::fs::rename(&agent_old, &agent_new)?;
                Ok(AgentUndo::RenameBack {
                    current: agent_new,
                    original: agent_old,
                })
            } else {
                Ok(AgentUndo::None)
            }
        }
    }
}

#[allow(unused_variables)]
fn undo_agent_op(undo: &AgentUndo) {
    match undo {
        AgentUndo::None => {}
        AgentUndo::RemoveCreated { created } => {
            let _ = remove_path(created);
        }
        AgentUndo::RestoreLink {
            created,
            old_path,
            old_target,
        } => {
            let _ = remove_path(created);
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(old_target, old_path);
        }
        AgentUndo::RenameBack { current, original } => {
            let _ = std::fs::rename(current, original);
        }
        AgentUndo::RecreateLink { path, target } => {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(target, path);
        }
    }
}

/// Best-effort rollback of the center-side rename (directory + front matter).
fn rollback_center_rename(
    new_dir: &std::path::Path,
    old_dir: &std::path::Path,
    new_name: &str,
    old_name: &str,
) {
    if new_dir.exists() && !old_dir.exists() {
        let _ = std::fs::rename(new_dir, old_dir);
        let skill_md = crate::fs::get_effective_skill_dir(old_dir, None).join("SKILL.md");
        let _ = crate::fs::rewrite_skill_md_name(&skill_md, new_name, old_name);
    }
}

/// Rollback for the already-moved case (P1-5): the center dir stays at
/// new_name; only the front matter rewrite is reverted.
fn rollback_fm_only(new_dir: &std::path::Path, new_name: &str, old_name: &str) {
    let skill_md = crate::fs::get_effective_skill_dir(new_dir, None).join("SKILL.md");
    let _ = crate::fs::rewrite_skill_md_name(&skill_md, new_name, old_name);
}

pub(crate) fn rename_skill_impl(
    db: &crate::db::Db,
    settings: &Settings,
    old_name: &str,
    new_name: &str,
) -> anyhow::Result<Skill> {
    // ---------- validation: rejects before touching anything ----------
    let new_name = new_name.trim();
    anyhow::ensure!(!new_name.is_empty(), "新名称不能为空");
    anyhow::ensure!(new_name != old_name, "新旧名称相同，无需改名");
    anyhow::ensure!(
        !new_name.contains('/') && !new_name.contains('\\') && new_name != "." && new_name != "..",
        "新名称包含非法字符: {new_name}"
    );
    let skill = db
        .get_skill_by_name(old_name)?
        .ok_or_else(|| anyhow::anyhow!("Skill 不存在: {old_name}"))?;
    anyhow::ensure!(
        db.get_skill_by_name(new_name)?.is_none(),
        "同名 Skill 已存在: {new_name}"
    );
    let old_dir = skill.repo_path.clone();
    let parent = old_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| settings.center_repo.clone());
    let new_dir = parent.join(new_name);
    // Idempotence (P1-5): the directory may already sit at new_name — the
    // user renamed it outside the app, or a previous rename died between the
    // fs move and the DB transaction. In that case this call converges DB +
    // agent links without moving the center dir again.
    let center_already_moved = !old_dir.is_dir() && new_dir.is_dir();
    anyhow::ensure!(
        old_dir.is_dir() || center_already_moved,
        "center 目录不存在: {}",
        old_dir.display()
    );
    anyhow::ensure!(
        center_already_moved || (!new_dir.exists() && !new_dir.is_symlink()),
        "center 已存在同名目录: {}",
        new_dir.display()
    );

    let targets: Vec<SyncTarget> = db
        .get_sync_targets()?
        .into_iter()
        .filter(|t| t.skill_id == skill.id)
        .collect();
    let agents = db.get_agents()?;

    // ---------- file operations, best-effort rollback on failure ----------
    let mut undos: Vec<AgentUndo> = Vec::new();
    let fs_result: anyhow::Result<()> = (|| {
        if !center_already_moved {
            std::fs::rename(&old_dir, &new_dir)?;
        }
        // Repo convention: directory name == SKILL.md front matter name.
        // Idempotent: a no-op when the front matter already carries new_name.
        let skill_md = crate::fs::get_effective_skill_dir(&new_dir, None).join("SKILL.md");
        crate::fs::rewrite_skill_md_name(&skill_md, old_name, new_name)?;
        for target in &targets {
            let agent = match agents.iter().find(|a| a.id == target.agent_id) {
                Some(a) => a,
                None => continue,
            };
            let undo = retarget_agent_entry(
                target.mode,
                &agent.skill_directory,
                old_name,
                new_name,
                &old_dir,
                &new_dir,
            )
            .map_err(|e| anyhow::anyhow!("agent {}: {e}", agent.name))?;
            undos.push(undo);
        }
        Ok(())
    })();
    if let Err(e) = fs_result {
        for undo in undos.iter().rev() {
            undo_agent_op(undo);
        }
        if center_already_moved {
            rollback_fm_only(&new_dir, new_name, old_name);
        } else {
            rollback_center_rename(&new_dir, &old_dir, new_name, old_name);
        }
        return Err(e.context("改名文件操作失败，已尽力回滚"));
    }

    // ---------- DB: single transaction, id preserved ----------
    if let Err(e) = db.rename_skill_tx(&skill.id, new_name, &new_dir) {
        for undo in undos.iter().rev() {
            undo_agent_op(undo);
        }
        if center_already_moved {
            rollback_fm_only(&new_dir, new_name, old_name);
        } else {
            rollback_center_rename(&new_dir, &old_dir, new_name, old_name);
        }
        return Err(e.context("DB 改名事务失败，已回滚文件操作"));
    }

    // ---------- refresh sync statuses with the P0-1 evaluator ----------
    let renamed_skill = Skill {
        name: new_name.to_string(),
        repo_path: new_dir.clone(),
        ..skill.clone()
    };
    for target in &targets {
        let agent = match agents.iter().find(|a| a.id == target.agent_id) {
            Some(a) => a,
            None => continue,
        };
        if let Ok(status) = crate::sync::evaluate_sync_target(target, &renamed_skill, agent) {
            if status != target.status {
                let _ = db.update_sync_target_status(&target.id, status);
            }
        }
    }

    let _ = invalidate_directory_skill_cache(db, None);

    Ok(Skill {
        name: new_name.to_string(),
        repo_path: new_dir,
        updated_at: current_timestamp(),
        ..skill
    })
}

/// SPEC-F5 T5: user-invoked diagnostic that re-runs the repo migration and
/// returns structured per-skill outcomes so the Settings → data-management
/// panel can toast a summary. Idempotent — safe to call repeatedly.
#[derive(serde::Serialize)]
pub struct RepairSkillPathsReport {
    pub migrated: Vec<String>,
    pub failed: Vec<RepairFailure>,
    /// P1-5: rename/orphan/legacy healing applied in the same pass.
    pub drift: RepairApplySummary,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RepairFailure {
    pub skill: String,
    pub reason: String,
}

/// P1-5: one suggested rename pairing — a DB row whose repo_path is gone,
/// matched to an unregistered center dir.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RepairRenamePair {
    pub skill_id: String,
    pub old_name: String,
    pub new_name: String,
    pub evidence: String,
}

/// P1-5: a center-repo skill directory not registered in the DB.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RepairOrphan {
    pub name: String,
    pub path: String,
}

/// P1-5: a dangling symlink in an agent dir pointing under a legacy layout
/// root (e.g. ~/.skillsync).
#[derive(Clone, Debug, serde::Serialize)]
pub struct RepairLegacyLink {
    pub path: String,
    pub target: String,
}

/// P1-5: full drift picture between the DB and the center repo / agent dirs.
/// Produced by [`scan_repair_drift`] (dry-run) and consumed by
/// [`apply_repair_drift`].
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RepairDriftReport {
    pub rename_pairs: Vec<RepairRenamePair>,
    pub rename_unresolved: Vec<RepairFailure>,
    pub orphans: Vec<RepairOrphan>,
    pub legacy_links: Vec<RepairLegacyLink>,
}

/// P1-5: outcome of applying a [`RepairDriftReport`].
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RepairApplySummary {
    pub renamed: Vec<String>,
    pub orphans_registered: Vec<String>,
    pub legacy_links_removed: Vec<String>,
    pub failed: Vec<RepairFailure>,
}

/// P1-5: center-repo directories that are real skills (P0-3 rules) but not
/// registered in the DB. Shared by the drift scan and the integrity check.
pub(crate) fn list_center_orphans(
    db: &crate::db::Db,
    settings: &Settings,
) -> Result<Vec<RepairOrphan>, String> {
    let registered: std::collections::HashSet<String> = db
        .get_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| s.name)
        .collect();
    let mut orphans = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&settings.center_repo) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.is_empty() || registered.contains(&name) {
                continue;
            }
            if crate::scan::is_excluded_scan_name(&name, &settings.scan_exclude_names) {
                continue;
            }
            if !crate::scan::is_skill_dir(&path) {
                continue;
            }
            orphans.push(RepairOrphan {
                name,
                path: path.to_string_lossy().to_string(),
            });
        }
    }
    orphans.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(orphans)
}

/// P1-5: read-only drift detection between the DB and the filesystem.
///
/// - rename pairing: DB rows whose repo_path vanished are matched against
///   unregistered center dirs. Evidence: the orphan's SKILL.md front matter
///   still carries the old name, or an agent-side entity copy of the old
///   name hashes equal to the orphan. Only a UNIQUE candidate becomes a
///   pair — anything else lands in `rename_unresolved` (never guess).
/// - orphan discovery: unregistered center skill dirs not consumed by a pair.
/// - legacy leftovers: dangling symlinks in agent dirs pointing under a
///   legacy layout root (~/.skillsync).
pub fn scan_repair_drift(
    db: &crate::db::Db,
    settings: &Settings,
) -> Result<RepairDriftReport, String> {
    let home = dirs::home_dir().unwrap_or_default();
    let legacy_roots = [home.join(".skillsync")];
    scan_repair_drift_with_roots(db, settings, &legacy_roots)
}

/// Inner scan with explicit legacy roots (tests inject a temp root).
pub(crate) fn scan_repair_drift_with_roots(
    db: &crate::db::Db,
    settings: &Settings,
    legacy_roots: &[PathBuf],
) -> Result<RepairDriftReport, String> {
    let mut report = RepairDriftReport::default();
    let skills = db.get_skills().map_err(|e| e.to_string())?;
    let orphans = list_center_orphans(db, settings)?;
    let agents = db.get_agents().map_err(|e| e.to_string())?;

    let mut reserved: std::collections::HashSet<String> = std::collections::HashSet::new();
    for skill in skills.iter().filter(|s| !s.repo_path.exists()) {
        let mut candidates: Vec<(String, String)> = Vec::new();
        for orphan in &orphans {
            let orphan_path = PathBuf::from(&orphan.path);
            // Evidence 1: dir renamed but SKILL.md front matter left behind.
            let fm_md = crate::fs::get_effective_skill_dir(&orphan_path, None).join("SKILL.md");
            if let Ok(content) = std::fs::read_to_string(&fm_md) {
                if crate::fs::skill_md_front_matter_name(&content).as_deref()
                    == Some(skill.name.as_str())
                {
                    candidates.push((
                        orphan.name.clone(),
                        "SKILL.md front matter name 与旧名一致".to_string(),
                    ));
                    continue;
                }
            }
            // Evidence 2: an agent-side entity copy of the old name hashes
            // equal to the orphan dir.
            if let Ok(orphan_hash) = crate::fs::compute_hash(&orphan_path) {
                let hash_matches = agents.iter().any(|agent| {
                    let agent_copy = agent.skill_directory.join(&skill.name);
                    agent_copy.is_dir()
                        && !agent_copy.is_symlink()
                        && crate::fs::compute_hash(&agent_copy)
                            .map(|h| h == orphan_hash)
                            .unwrap_or(false)
                });
                if hash_matches {
                    candidates.push((
                        orphan.name.clone(),
                        "与 agent 侧旧名副本内容一致 (hash)".to_string(),
                    ));
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        // Candidate dirs are reserved for this missing row even when the
        // pairing is ambiguous — they must not be registered as orphans.
        for (name, _) in &candidates {
            reserved.insert(name.clone());
        }
        match candidates.len() {
            1 => {
                let (new_name, evidence) = candidates.pop().unwrap();
                report.rename_pairs.push(RepairRenamePair {
                    skill_id: skill.id.clone(),
                    old_name: skill.name.clone(),
                    new_name,
                    evidence,
                });
            }
            0 => report.rename_unresolved.push(RepairFailure {
                skill: skill.name.clone(),
                reason: "center 中找不到等价目录，无法配对".to_string(),
            }),
            n => report.rename_unresolved.push(RepairFailure {
                skill: skill.name.clone(),
                reason: format!(
                    "配对候选不唯一（{n} 个）：{}",
                    candidates
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
        }
    }

    // Orphans consumed by a rename pair (or reserved by an ambiguous one)
    // are not reported for registration.
    report.orphans = orphans
        .into_iter()
        .filter(|o| !reserved.contains(&o.name))
        .collect();

    // Legacy leftovers: broken links in agent dirs pointing at legacy roots.
    for agent in &agents {
        if let Ok(entries) = std::fs::read_dir(&agent.skill_directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !crate::fs::is_broken_symlink(&path) {
                    continue;
                }
                let target = std::fs::read_link(&path).unwrap_or_default();
                if legacy_roots.iter().any(|root| target.starts_with(root)) {
                    report.legacy_links.push(RepairLegacyLink {
                        path: path.to_string_lossy().to_string(),
                        target: target.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
    Ok(report)
}

/// Register one orphan center dir, mirroring the import flow: insert the
/// skill row, create sync targets for every enabled agent, and apply them.
fn register_orphan_skill(
    db: &crate::db::Db,
    settings: &Settings,
    orphan: &RepairOrphan,
    enabled_agents: &[&Agent],
) -> Result<(), String> {
    if db
        .get_skill_by_name(&orphan.name)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("DB 中已存在同名记录，跳过".to_string());
    }
    let now = current_timestamp();
    let skill = Skill {
        id: new_id(),
        name: orphan.name.clone(),
        repo_path: PathBuf::from(&orphan.path),
        created_at: now,
        updated_at: now,
        status: SkillStatus::Draft,
    };
    db.insert_skill(&skill).map_err(|e| e.to_string())?;
    for agent in enabled_agents {
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
        let _ = apply_sync_target_and_record(db, &target, &skill, agent);
    }
    Ok(())
}

/// P1-5: execute a previously-scanned drift report. Renames go through
/// rename_skill (P1-4, with its own rollback); orphans are registered like
/// imports; only confirmed-dangling legacy symlinks are removed — entity
/// directories are never touched.
pub fn apply_repair_drift(
    db: &crate::db::Db,
    settings: &Settings,
    report: &RepairDriftReport,
) -> Result<RepairApplySummary, String> {
    let mut summary = RepairApplySummary::default();

    for pair in &report.rename_pairs {
        match rename_skill_impl(db, settings, &pair.old_name, &pair.new_name) {
            Ok(_) => summary
                .renamed
                .push(format!("{} -> {}", pair.old_name, pair.new_name)),
            Err(e) => summary.failed.push(RepairFailure {
                skill: pair.old_name.clone(),
                reason: format!("rename 失败：{e}"),
            }),
        }
    }

    let agents = db.get_agents().map_err(|e| e.to_string())?;
    let enabled: Vec<&Agent> = agents.iter().filter(|a| a.is_enabled).collect();
    for orphan in &report.orphans {
        match register_orphan_skill(db, settings, orphan, &enabled) {
            Ok(()) => summary.orphans_registered.push(orphan.name.clone()),
            Err(reason) => summary.failed.push(RepairFailure {
                skill: orphan.name.clone(),
                reason,
            }),
        }
    }

    for link in &report.legacy_links {
        let path = PathBuf::from(&link.path);
        if !crate::fs::is_broken_symlink(&path) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => summary.legacy_links_removed.push(link.path.clone()),
            Err(e) => summary.failed.push(RepairFailure {
                skill: link.path.clone(),
                reason: format!("删除悬空链失败：{e}"),
            }),
        }
    }

    let _ = invalidate_directory_skill_cache(db, None);
    Ok(summary)
}

#[tauri::command]
pub fn repair_skill_paths(state: State<'_, AppState>) -> Result<RepairSkillPathsReport, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    let report = migrate_skills_to_repo(&db, &settings.center_repo)?;
    // P1-5: the same repair pass now also heals rename drift, orphans and
    // legacy leftovers (the binary shares this via scan/apply_repair_drift).
    let drift = scan_repair_drift(&db, &settings)?;
    let drift_applied = apply_repair_drift(&db, &settings, &drift)?;
    Ok(RepairSkillPathsReport {
        migrated: report.migrated,
        failed: report
            .failed
            .into_iter()
            .map(|f| RepairFailure {
                skill: f.skill,
                reason: f.reason,
            })
            .collect(),
        drift: drift_applied,
    })
}

