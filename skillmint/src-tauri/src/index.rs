//! P3-3: the unified, rebuildable skill index — SkillMint's read model over
//! on-disk truth (Mole model: the app owns no install state of its own).
//!
//! Sources merged on every rebuild:
//! 1. Global lock `~/.agents/.skill-lock.json` + the canonical global store
//!    `~/.agents/skills/` → npx-managed global installs.
//! 2. Project lock `<root>/skills-lock.json` + `<root>/.agents/skills/` →
//!    npx-managed project installs.
//! 3. Every agent dir from the static matrix → per-skill agent links
//!    (symlinks resolving into the canonical store) plus standalone/unmanaged
//!    skills that no lock knows about.
//! 4. Private hubs (global + project) → authored skills.
//!
//! The result is written to the `skill_index` table (replace-all per project
//! key) — the DB is a disposable cache; deleting it is always safe.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::agents_table::{self, AgentDef};
use crate::skills_lock::{self, GlobalLock, ProjectLock};

pub const SCOPE_GLOBAL: &str = "global";
pub const SCOPE_PROJECT: &str = "project";
pub const SCOPE_HUB_GLOBAL: &str = "hub-global";
pub const SCOPE_HUB_PROJECT: &str = "hub-project";

/// One row of the rebuilt index.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillIndexEntry {
    pub name: String,
    /// `global` | `project` | `hub-global` | `hub-project`
    pub scope: String,
    /// `npx` (lock-known install) | `unmanaged` (no lock record) | `hub` (authored)
    pub managed_by: String,
    /// Absolute path of the skill folder (canonical store for npx installs).
    pub path: String,
    pub skill_md_path: String,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub source_type: Option<String>,
    pub ref_spec: Option<String>,
    /// Lock hash when the lock has one; otherwise our own content hash.
    pub hash: String,
    /// `ok` | `modified` (content changed since the previous scan) | `broken`
    pub status: String,
    /// Agent CLI keys linked to this skill.
    pub agents: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexSummary {
    pub project_key: String,
    pub total: usize,
    pub npx_global: usize,
    pub npx_project: usize,
    pub unmanaged: usize,
    pub hub: usize,
    pub modified: usize,
    pub broken: usize,
    pub fingerprint: String,
    pub rebuilt_at: u64,
}

/// Stable sha256 over a skill folder's files (rel path + bytes, sorted) —
/// our own change detector, independent of the CLI's hashing algorithm.
pub fn content_hash(dir: &Path) -> String {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    let mut hasher = Sha256::new();
    for f in files {
        let rel = f.strip_prefix(dir).unwrap_or(&f).to_string_lossy();
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        if let Ok(bytes) = std::fs::read(&f) {
            hasher.update(&bytes);
        }
        hasher.update(b"\0");
    }
    format!("sha256-{}", hex::encode(hasher.finalize()))
}

/// Best-effort SKILL.md frontmatter summary: (name, description).
pub fn read_summary(skill_md: &Path) -> (Option<String>, Option<String>) {
    let content = match std::fs::read_to_string(skill_md) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            let d = v.trim().trim_matches('"').trim_matches('\'').to_string();
            description = Some(d);
        }
    }
    (name, description)
}

/// Cheap change probe for the auto-refresh tick: mtimes of the two locks, the
/// canonical stores, and the hubs. When the string is unchanged, a rebuild
/// would be a no-op.
pub fn fingerprint(project_root: Option<&Path>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut probe = |p: PathBuf| {
        let mtime = std::fs::symlink_metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis().to_string())
            .unwrap_or_else(|| "0".into());
        parts.push(format!("{}:{}", p.display(), mtime));
    };
    probe(skills_lock::global_lock_path());
    probe(skills_lock::global_canonical_path());
    probe(crate::hub::global_hub_dir());
    if let Some(root) = project_root {
        probe(root.join(skills_lock::PROJECT_LOCK_FILE));
        probe(root.join(skills_lock::PROJECT_CANONICAL_DIR));
        probe(crate::hub::project_hub_dir(root));
    }
    parts.join("|")
}

fn agent_dirs_global() -> Vec<(&'static AgentDef, PathBuf)> {
    agents_table::AGENTS
        .iter()
        .filter_map(|a| a.global_dir.map(|g| (a, agents_table::expand_global(g))))
        .collect()
}

fn agent_dirs_project(root: &Path) -> Vec<(&'static AgentDef, PathBuf)> {
    agents_table::AGENTS
        .iter()
        .map(|a| (a, root.join(a.project_dir)))
        .collect()
}

/// Which agents point at `canonical_entry` (a path in the canonical store)?
/// Universal agents own the canonical dir itself, so their entry IS the
/// canonical one; every other agent counts when its dir holds a link that
/// resolves to the same physical directory. This mirrors how `skills ls`
/// reports per-skill agents.
fn agents_linking(canonical_entry: &Path, dirs: &[(&'static AgentDef, PathBuf)], _canonical_dir: &Path, is_canonical_dir: bool) -> Vec<String> {
    let mut keys = Vec::new();
    for (a, dir) in dirs {
        if agents_table::is_universal(a) {
            if is_canonical_dir {
                keys.push(a.key.to_string());
            }
            continue;
        }
        let entry = dir.join(canonical_entry.file_name().unwrap_or_default());
        if !entry.exists() && std::fs::symlink_metadata(&entry).is_err() {
            continue;
        }
        if let Ok(resolved) = entry.canonicalize() {
            if resolved == canonical_entry {
                keys.push(a.key.to_string());
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

struct RebuildAcc {
    entries: Vec<SkillIndexEntry>,
    npx_global: usize,
    npx_project: usize,
    unmanaged: usize,
    hub: usize,
}

impl RebuildAcc {
    fn push(&mut self, e: SkillIndexEntry) {
        match (e.scope.as_str(), e.managed_by.as_str()) {
            (SCOPE_GLOBAL, "npx") => self.npx_global += 1,
            (SCOPE_PROJECT, "npx") => self.npx_project += 1,
            (_, "hub") => self.hub += 1,
            _ => self.unmanaged += 1,
        }
        self.entries.push(e);
    }
}

fn scan_scope(
    acc: &mut RebuildAcc,
    scope: &str,
    canonical_dir: &Path,
    dirs: &[(&'static AgentDef, PathBuf)],
    lock: Option<&BTreeMap<String, LockView>>,
) {
    // 1. Canonical store entries = npx-managed (or unmanaged when absent from
    //    the lock — e.g. hand-placed folders the CLI never installed).
    if canonical_dir.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(canonical_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        for name in names {
            let path = canonical_dir.join(&name);
            if !crate::scan::is_skill_dir(&path) {
                continue;
            }
            if crate::scan::is_excluded_scan_name(&name, &[]) {
                continue;
            }
            let (fm_name, description) = read_summary(&path.join("SKILL.md"));
            let agents = agents_linking(&path, dirs, canonical_dir, true);
            let (source, source_url, source_type, ref_spec, hash) = match lock.and_then(|l| l.get(&name)) {
                Some(v) => (
                    v.source.clone(),
                    v.source_url.clone(),
                    v.source_type.clone(),
                    v.r#ref.clone(),
                    v.hash.clone(),
                ),
                None => (None, None, None, None, String::new()),
            };
            let managed = if lock.as_ref().map(|l| l.contains_key(&name)).unwrap_or(false) {
                "npx"
            } else {
                "unmanaged"
            };
            acc.push(SkillIndexEntry {
                name: fm_name.unwrap_or_else(|| name.clone()),
                scope: scope.into(),
                managed_by: managed.into(),
                path: path.to_string_lossy().to_string(),
                skill_md_path: path.join("SKILL.md").to_string_lossy().to_string(),
                source,
                source_url,
                source_type,
                ref_spec,
                hash,
                status: "ok".into(),
                agents,
                description,
            });
        }
    }

    // 2. Standalone entries in per-agent dirs: skills that live directly in an
    //    agent's own dir (installed by other means, or hand-placed). Dedup by
    //    resolved path across agents. Broken symlinks are reported as such.
    let mut seen: BTreeMap<PathBuf, SkillIndexEntry> = BTreeMap::new();
    for (a, dir) in dirs {
        if agents_table::is_universal(a) || !dir.is_dir() {
            continue;
        }
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        for name in names {
            if crate::scan::is_excluded_scan_name(&name, &[]) {
                continue;
            }
            let entry = dir.join(&name);
            let meta = match std::fs::symlink_metadata(&entry) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_dir() && !meta.file_type().is_symlink() {
                continue;
            }
            if meta.file_type().is_symlink() {
                // Link into canonical store → already covered by step 1.
                if entry.canonicalize().map(|r| r.parent() == Some(canonical_dir)).unwrap_or(false) {
                    continue;
                }
            }
            let resolved = std::fs::canonicalize(&entry).unwrap_or_else(|_| entry.clone());
            if !resolved.join("SKILL.md").is_file() {
                // Broken symlink (or a dir that lost its SKILL.md) — surface it.
                if meta.file_type().is_symlink() && !entry.exists() {
                    let e = SkillIndexEntry {
                        name: name.clone(),
                        scope: scope.into(),
                        managed_by: "unmanaged".into(),
                        path: entry.to_string_lossy().to_string(),
                        skill_md_path: entry.join("SKILL.md").to_string_lossy().to_string(),
                        source: None,
                        source_url: None,
                        source_type: None,
                        ref_spec: None,
                        hash: String::new(),
                        status: "broken".into(),
                        agents: vec![a.key.to_string()],
                        description: Some("失效链接（目标不存在）".into()),
                    };
                    seen.entry(resolved).or_insert(e);
                }
                continue;
            }
            if crate::scan::is_excluded_scan_name(resolved.file_name().map(|n| n.to_string_lossy().to_string()).as_deref().unwrap_or(""), &[]) {
                continue;
            }
            let (fm_name, description) = read_summary(&resolved.join("SKILL.md"));
            let e = SkillIndexEntry {
                name: fm_name.unwrap_or_else(|| name.clone()),
                scope: scope.into(),
                managed_by: "unmanaged".into(),
                path: resolved.to_string_lossy().to_string(),
                skill_md_path: resolved.join("SKILL.md").to_string_lossy().to_string(),
                source: None,
                source_url: None,
                source_type: None,
                ref_spec: None,
                hash: String::new(),
                status: "ok".into(),
                agents: vec![a.key.to_string()],
                description,
            };
            seen.entry(resolved).or_insert(e);
        }
    }
    for (_, mut e) in seen {
        e.agents.sort();
        e.agents.dedup();
        acc.push(e);
    }
}

/// Unified view over both lock entry shapes.
#[derive(Clone)]
pub struct LockView {
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub source_type: Option<String>,
    pub r#ref: Option<String>,
    pub hash: String,
}

fn global_lock_views(glock: &Option<GlobalLock>) -> BTreeMap<String, LockView> {
    glock
        .as_ref()
        .map(|l| {
            l.skills
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        LockView {
                            source: Some(v.source.clone()).filter(|s| !s.is_empty()),
                            source_url: v.source_url.clone(),
                            source_type: Some(v.source_type.clone()).filter(|s| !s.is_empty()),
                            r#ref: v.r#ref.clone(),
                            hash: v.skill_folder_hash.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn project_lock_views(plock: &Option<ProjectLock>) -> BTreeMap<String, LockView> {
    plock
        .as_ref()
        .map(|l| {
            l.skills
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        LockView {
                            source: Some(v.source.clone()).filter(|s| !s.is_empty()),
                            source_url: v.source_url.clone(),
                            source_type: Some(v.source_type.clone()).filter(|s| !s.is_empty()),
                            r#ref: v.r#ref.clone(),
                            hash: v.computed_hash.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scan_hub(acc: &mut RebuildAcc, hub: &Path, scope: &str) {
    if !hub.is_dir() {
        return;
    }
    let mut names: Vec<String> = std::fs::read_dir(hub)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for name in names {
        if crate::scan::is_excluded_scan_name(&name, &[]) {
            continue;
        }
        let path = hub.join(&name);
        if !crate::scan::is_skill_dir(&path) {
            continue;
        }
        let (fm_name, description) = read_summary(&path.join("SKILL.md"));
        acc.push(SkillIndexEntry {
            name: fm_name.unwrap_or_else(|| name.clone()),
            scope: scope.into(),
            managed_by: "hub".into(),
            path: path.to_string_lossy().to_string(),
            skill_md_path: path.join("SKILL.md").to_string_lossy().to_string(),
            source: None,
            source_url: None,
            source_type: None,
            ref_spec: None,
            hash: String::new(),
            status: "ok".into(),
            agents: Vec::new(),
            description,
        });
    }
}

/// Public single-hub scan used by the hub page (live view, bypasses the DB).
pub fn scan_hub_dir(hub: &Path, scope: &str) -> Vec<SkillIndexEntry> {
    let mut acc = RebuildAcc {
        entries: Vec::new(),
        npx_global: 0,
        npx_project: 0,
        unmanaged: 0,
        hub: 0,
    };
    scan_hub(&mut acc, hub, scope);
    acc.entries
}

/// Rebuild the whole index for the global scope plus (optionally) one project.
/// `project_key` is `"-"` for global-only, else the project root string.
pub fn rebuild(db: &crate::db::Db, project_root: Option<&Path>) -> Result<IndexSummary> {
    let project_key = project_root
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "-".into());

    let glock = skills_lock::load_global_lock()?;
    let gviews = global_lock_views(&glock);
    let gdirs = agent_dirs_global();

    let (pviews, pdirs) = match project_root {
        Some(root) => {
            let plock = skills_lock::load_project_lock(root)?;
            let views = project_lock_views(&plock);
            let dirs = agent_dirs_project(root);
            (views, dirs)
        }
        None => (BTreeMap::new(), Vec::new()),
    };

    let mut acc = RebuildAcc {
        entries: Vec::new(),
        npx_global: 0,
        npx_project: 0,
        unmanaged: 0,
        hub: 0,
    };
    scan_scope(&mut acc, SCOPE_GLOBAL, &skills_lock::global_canonical_path(), &gdirs, Some(&gviews));
    if let Some(root) = project_root {
        scan_scope(
            &mut acc,
            SCOPE_PROJECT,
            &root.join(skills_lock::PROJECT_CANONICAL_DIR),
            &pdirs,
            Some(&pviews),
        );
    }
    scan_hub(&mut acc, &crate::hub::global_hub_dir(), SCOPE_HUB_GLOBAL);
    if let Some(root) = project_root {
        scan_hub(&mut acc, &crate::hub::project_hub_dir(root), SCOPE_HUB_PROJECT);
    }

    let modified = db.replace_skill_index(&project_key, &acc.entries)?;
    let summary = IndexSummary {
        project_key,
        total: acc.entries.len(),
        npx_global: acc.npx_global,
        npx_project: acc.npx_project,
        unmanaged: acc.unmanaged,
        hub: acc.hub,
        modified,
        broken: acc.entries.iter().filter(|e| e.status == "broken").count(),
        fingerprint: fingerprint(project_root),
        rebuilt_at: chrono::Utc::now().timestamp_millis() as u64,
    };
    db.set_index_meta(&summary.project_key, &summary.fingerprint, summary.rebuilt_at)?;
    Ok(summary)
}

/// Rebuild only when the on-disk fingerprint changed since the last rebuild —
/// the auto-refresh tick's cheap path. `None` = nothing changed.
pub fn rebuild_if_stale(db: &crate::db::Db, project_root: Option<&Path>) -> Result<Option<IndexSummary>> {
    let project_key = project_root
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "-".into());
    let current = fingerprint(project_root);
    let stale = match db.get_index_meta(&project_key)? {
        Some((fp, _)) => fp != current,
        None => true,
    };
    if !stale {
        return Ok(None);
    }
    Ok(Some(rebuild(db, project_root)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_change_sensitive() {
        let td = tempfile::tempdir().unwrap();
        let skill = td.path().join("s");
        std::fs::create_dir_all(skill.join("sub")).unwrap();
        std::fs::write(skill.join("SKILL.md"), "a").unwrap();
        std::fs::write(skill.join("sub/x.txt"), "b").unwrap();
        let h1 = content_hash(&skill);
        assert_eq!(h1, content_hash(&skill), "stable across calls");
        std::fs::write(skill.join("sub/x.txt"), "changed").unwrap();
        assert_ne!(h1, content_hash(&skill), "content change alters hash");
        assert!(h1.starts_with("sha256-"));
    }

    #[test]
    fn summary_parses_frontmatter() {
        let td = tempfile::tempdir().unwrap();
        let md = td.path().join("SKILL.md");
        std::fs::write(&md, "---\nname: my-skill\ndescription: does things\n---\nbody").unwrap();
        let (name, desc) = read_summary(&md);
        assert_eq!(name.as_deref(), Some("my-skill"));
        assert_eq!(desc.as_deref(), Some("does things"));
        std::fs::write(&md, "no frontmatter").unwrap();
        assert_eq!(read_summary(&md), (None, None));
    }

    #[test]
    fn fingerprint_reflects_hub_changes() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().to_path_buf();
        let f1 = fingerprint(Some(&root));
        let hub = crate::hub::project_hub_dir(&root);
        crate::hub::create_skill(&hub, "project", "fp-skill", "").unwrap();
        let f2 = fingerprint(Some(&root));
        assert_ne!(f1, f2, "hub mutation must change the fingerprint");
    }
}
