//! P3-5: private skills hubs — the authoring/hosting layer that replaces the
//! retired center repo. A hub is just a directory of skill folders (each with
//! a SKILL.md) under git; the `skills` CLI installs from it directly
//! (`skills add <hub-path>` locally, or `<git-url>` after a manual push).
//!
//! Two scopes:
//! - global: `~/.skillmint/hub` (own .git, optional private remote — personal
//!   skills shared across machines/projects)
//! - project: `<project>/.skillmint/hub` (committed with the project repo —
//!   team-shared; no nested .git is created)
//!
//! Rule of the house (design lock-in): collecting INTO the hub only ever
//! copies; it never touches the source directory, npx locks, or agent links.

use std::path::{Path, PathBuf};

use anyhow::Result;

pub const HUB_DIR_NAME: &str = ".skillmint/hub";
pub const TRASH_DIR_NAME: &str = ".skillmint/trash";
/// How long a trash snapshot survives before the startup purge removes it
/// (mirrors the SPEC-C3 trash retention used by the DB rows).
pub const TRASH_RETENTION_DAYS: u64 = 30;

pub fn global_hub_dir() -> PathBuf {
    crate::scan::expand_path("~/.skillmint/hub")
}

pub fn project_hub_dir(project_root: &Path) -> PathBuf {
    project_root.join(HUB_DIR_NAME)
}

pub fn resolve_hub(scope: &str, project_root: Option<&Path>) -> Result<PathBuf> {
    match scope {
        "global" => Ok(global_hub_dir()),
        "project" => {
            let root = project_root
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| anyhow::anyhow!("未指定项目根目录"))?;
            Ok(project_hub_dir(&root))
        }
        other => Err(anyhow::anyhow!("未知 hub 作用域：{other}")),
    }
}

fn run_git(hub: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .current_dir(hub)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("无法调用 git：{e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} 失败：{}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_git_inited(hub: &Path) -> bool {
    hub.join(".git").exists()
}

/// Create the hub dir if missing and make it a git repo (global scope only —
/// a project hub relies on the project's own repo and must never nest .git).
pub fn ensure_hub(hub: &Path, scope: &str) -> Result<()> {
    std::fs::create_dir_all(hub)?;
    let gitignore = hub.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(&gitignore, ".DS_Store\nnode_modules/\n__pycache__/\n");
    }
    if scope == "global" && !is_git_inited(hub) {
        run_git(hub, &["init"])?;
    }
    Ok(())
}

/// Commit pending changes if any. Message should describe the mutation.
pub fn auto_commit(hub: &Path, message: &str) -> Result<Option<String>> {
    if !is_git_inited(hub) {
        return Ok(None);
    }
    if run_git(hub, &["status", "--porcelain"])?.is_empty() {
        return Ok(None);
    }
    run_git(hub, &["add", "-A"])?;
    run_git(hub, &["commit", "-m", message])?;
    Ok(run_git(hub, &["rev-parse", "--short", "HEAD"]).ok())
}

const SKILL_TEMPLATE: &str = r#"---
name: {name}
description: {description}
---

# {name}

## What this skill does

TODO: describe the capability in one or two sentences.

## How to use it

TODO: concrete usage steps / example invocations.
"#;

/// Validate a skill folder name: kebab-case, non-empty, no path separators.
pub fn validate_skill_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(anyhow::anyhow!("skill 名称需为小写字母/数字/短横线的 kebab-case（≤64 字符）：{name}"))
    }
}

/// Create a new skill folder in the hub with a SKILL.md template and commit.
pub fn create_skill(hub: &Path, scope: &str, name: &str, description: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;
    ensure_hub(hub, scope)?;
    let dir = hub.join(name);
    if dir.exists() {
        anyhow::bail!("hub 中已存在同名 skill：{name}");
    }
    std::fs::create_dir_all(&dir)?;
    let desc = if description.trim().is_empty() { "TODO: one-line description" } else { description.trim() };
    std::fs::write(dir.join("SKILL.md"), SKILL_TEMPLATE.replace("{name}", name).replace("{description}", desc))?;
    auto_commit(hub, &format!("skill: create {name}"))?;
    Ok(dir)
}

/// Directories never copied when collecting a skill into the hub (aligns with
/// scan::DEFAULT_SCAN_EXCLUSIONS plus VCS/OS noise).
const COLLECT_SKIP: &[&str] = &[".git", "node_modules", "__pycache__", ".DS_Store", ".trash", "cache", "data", "marketplaces"];

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if COLLECT_SKIP.contains(&name.as_str()) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        // Symlinks inside a skill are resolved to their target content: the
        // hub must be a self-contained copy (it gets pushed to remotes).
        let resolved = std::fs::canonicalize(&from).unwrap_or(from.clone());
        if resolved.is_dir() {
            copy_tree(&resolved, &to)?;
        } else if resolved.is_file() {
            std::fs::copy(&resolved, &to)?;
        }
    }
    Ok(())
}

/// Collect (copy) a skill directory into the hub and commit. The source —
/// whether an agent dir entry, a canonical npx install, or an unmanaged
/// folder — is never modified.
pub fn collect_skill(hub: &Path, scope: &str, source: &Path, name: Option<&str>) -> Result<PathBuf> {
    let source = std::fs::canonicalize(source)
        .map_err(|e| anyhow::anyhow!("源目录不存在：{}（{e}）", source.display()))?;
    if !source.join("SKILL.md").is_file() {
        anyhow::bail!("源目录没有 SKILL.md，不是有效 skill：{}", source.display());
    }
    let name = match name {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .ok_or_else(|| anyhow::anyhow!("无法从路径推导 skill 名称"))?,
    };
    validate_skill_name(&name)?;
    ensure_hub(hub, scope)?;
    let dir = hub.join(&name);
    if dir.exists() {
        anyhow::bail!("hub 中已存在同名 skill：{name}（请先重命名或删除）");
    }
    copy_tree(&source, &dir)?;
    auto_commit(hub, &format!("skill: collect {name} from {}", source.display()))?;
    Ok(dir)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HubStatus {
    pub scope: String,
    pub path: String,
    pub exists: bool,
    pub git_inited: bool,
    pub skill_count: usize,
    pub remote: Option<String>,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub dirty_files: usize,
    pub last_commit: Option<HubCommit>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HubCommit {
    pub hash: String,
    pub message: String,
    pub time: String,
}

/// Read-only hub state for the hub page. `fetch` runs best-effort so
/// ahead/behind is meaningful; network failure never fails the status call.
pub fn hub_status(hub: &Path, scope: &str) -> Result<HubStatus> {
    let base = HubStatus {
        scope: scope.to_string(),
        path: hub.to_string_lossy().to_string(),
        exists: hub.is_dir(),
        git_inited: is_git_inited(hub),
        skill_count: 0,
        remote: None,
        branch: "main".into(),
        ahead: 0,
        behind: 0,
        dirty_files: 0,
        last_commit: None,
    };
    if !base.exists {
        return Ok(base);
    }
    let skill_count = std::fs::read_dir(hub)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir() && e.path().join("SKILL.md").is_file())
                .count()
        })
        .unwrap_or(0);

    let mut st = HubStatus { skill_count, ..base };
    if !st.git_inited {
        return Ok(st);
    }
    st.branch = run_git(hub, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "main".into());
    st.remote = run_git(hub, &["remote", "get-url", "origin"]).ok();
    let _ = run_git(hub, &["fetch", "origin", "--quiet"]);
    let sb = run_git(hub, &["status", "--porcelain=v1", "--branch"]).unwrap_or_default();
    let mut has_upstream = false;
    for line in sb.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if rest.contains("...") {
                has_upstream = true;
            }
            for tok in rest.split(['[', ']', ',', ' ']) {
                if let Some(n) = tok.strip_prefix("ahead ") {
                    st.ahead = n.parse().unwrap_or(0);
                }
                if let Some(n) = tok.strip_prefix("behind ") {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if !line.is_empty() {
            st.dirty_files += 1;
        }
    }
    if !has_upstream {
        // No upstream yet: every commit is waiting to be pushed once a remote
        // is configured — much more useful to the UI than a bare 0.
        st.ahead = run_git(hub, &["rev-list", "--count", "HEAD"])
            .ok()
            .and_then(|c| c.trim().parse().ok())
            .unwrap_or(0);
    }
    let log = run_git(hub, &["log", "-1", "--pretty=%h%x1f%s%x1f%ci"]).ok();
    st.last_commit = log.map(|l| {
        let mut it = l.splitn(3, '\u{1f}');
        HubCommit {
            hash: it.next().unwrap_or("").into(),
            message: it.next().unwrap_or("").into(),
            time: it.next().unwrap_or("").into(),
        }
    });
    Ok(st)
}

/// Set (or replace) the `origin` remote of the global hub.
pub fn set_remote(hub: &Path, url: &str) -> Result<()> {
    if !is_git_inited(hub) {
        ensure_hub(hub, "global")?;
    }
    if run_git(hub, &["remote", "get-url", "origin"]).is_ok() {
        run_git(hub, &["remote", "set-url", "origin", url])?;
    } else {
        run_git(hub, &["remote", "add", "origin", url])?;
    }
    Ok(())
}

/// Manual push (design: never automatic).
pub fn push(hub: &Path) -> Result<String> {
    if !is_git_inited(hub) {
        anyhow::bail!("hub 还不是 git 仓库");
    }
    if run_git(hub, &["remote", "get-url", "origin"]).is_err() {
        anyhow::bail!("hub 未配置远端（origin）。先在设置里添加私有 git 远端。");
    }
    let branch = run_git(hub, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "main".into());
    run_git(hub, &["push", "-u", "origin", &branch])
}

/// Snapshot a directory into `~/.skillmint/trash/` before a destructive op and
/// register it in the SPEC-C3 trash table (startup purge handles expiry).
/// Returns the snapshot path so a later failure can clean it up.
pub fn snapshot_to_trash(db: &crate::db::Db, name: &str, source_dir: &Path) -> Result<PathBuf> {
    let trash_root = crate::scan::expand_path(&format!("~/{TRASH_DIR_NAME}"));
    std::fs::create_dir_all(&trash_root)?;
    let ts = chrono::Utc::now().timestamp_millis();
    let snapshot = trash_root.join(format!("{name}-{ts}"));
    copy_tree(&std::fs::canonicalize(source_dir)?, &snapshot)?;
    let expires = chrono::Utc::now().timestamp_millis()
        + (TRASH_RETENTION_DAYS * 24 * 3600 * 1000) as i64;
    db.insert_trash_item(
        "skill",
        name,
        name,
        &snapshot.to_string_lossy(),
        &serde_json::json!({ "source": source_dir.to_string_lossy() }),
        ts as u64,
        expires as u64,
    )?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_hub() -> (tempfile::TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let hub = td.path().join("hub");
        (td, hub)
    }

    #[test]
    fn validate_names() {
        assert!(validate_skill_name("pdf-export").is_ok());
        assert!(validate_skill_name("a1-b2").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("PDF").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("-x").is_err());
        assert!(validate_skill_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn create_skill_writes_template_and_commits() {
        let (_td, hub) = tmp_hub();
        let dir = create_skill(&hub, "global", "demo-skill", "does demo things").unwrap();
        let md = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(md.contains("name: demo-skill"));
        assert!(md.contains("description: does demo things"));
        assert!(is_git_inited(&hub));
        let st = hub_status(&hub, "global").unwrap();
        assert_eq!(st.skill_count, 1);
        assert_eq!(st.ahead, 1, "one commit expected");
        assert!(st.last_commit.as_ref().unwrap().message.contains("create demo-skill"));
    }

    #[test]
    fn create_skill_rejects_duplicates() {
        let (_td, hub) = tmp_hub();
        create_skill(&hub, "global", "dup", "").unwrap();
        assert!(create_skill(&hub, "global", "dup", "").is_err());
    }

    #[test]
    fn collect_copies_without_touching_source() {
        let td = tempfile::tempdir().unwrap();
        let src = td.path().join("src-skill");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: src-skill\ndescription: x\n---\nbody").unwrap();
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("nested/keep.txt"), "1").unwrap();
        std::fs::create_dir_all(src.join("node_modules")).unwrap();
        std::fs::write(src.join("node_modules/junk.js"), "2").unwrap();
        let before = std::fs::read_dir(&src).unwrap().count();

        let (_h, hub) = tmp_hub();
        let dir = collect_skill(&hub, "project", &src, None).unwrap();
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("nested/keep.txt").is_file());
        assert!(!dir.join("node_modules").exists(), "excluded dirs must not be copied");
        assert_eq!(std::fs::read_dir(&src).unwrap().count(), before, "source untouched");
    }

    #[test]
    fn collect_follows_symlinked_skill_entry() {
        // The realistic case: collecting an npx-installed skill whose agent-dir
        // entry is a symlink into the canonical store.
        let td = tempfile::tempdir().unwrap();
        let canon = td.path().join("canon/real-skill");
        std::fs::create_dir_all(&canon).unwrap();
        std::fs::write(canon.join("SKILL.md"), "---\nname: real-skill\ndescription: x\n---\n").unwrap();
        let agent_dir = td.path().join("agentdir");
        std::fs::create_dir_all(&agent_dir).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&canon, agent_dir.join("real-skill")).unwrap();

        let (_h, hub) = tmp_hub();
        let dir = collect_skill(&hub, "global", &agent_dir.join("real-skill"), None).unwrap();
        assert!(dir.join("SKILL.md").is_file());
        // And the canonical source is untouched.
        assert!(canon.join("SKILL.md").is_file());
    }

    #[test]
    fn collect_requires_skill_md() {
        let td = tempfile::tempdir().unwrap();
        let not_a_skill = td.path().join("junk");
        std::fs::create_dir_all(&not_a_skill).unwrap();
        let (_h, hub) = tmp_hub();
        assert!(collect_skill(&hub, "global", &not_a_skill, None).is_err());
    }

    #[test]
    fn status_without_remote_reports_counts() {
        let (_td, hub) = tmp_hub();
        create_skill(&hub, "global", "s1", "").unwrap();
        let st = hub_status(&hub, "global").unwrap();
        assert!(st.git_inited);
        assert!(st.remote.is_none());
        assert_eq!(st.skill_count, 1);
        assert_eq!(st.dirty_files, 0, "auto-commit leaves the tree clean");
    }

    #[test]
    fn project_hub_has_no_nested_git() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let hub = project_hub_dir(&root);
        create_skill(&hub, "project", "team-skill", "").unwrap();
        assert!(!is_git_inited(&hub), "project hub must not nest .git");
        let st = hub_status(&hub, "project").unwrap();
        assert_eq!(st.skill_count, 1);
        assert_eq!(st.ahead, 0, "no git → no ahead/behind");
    }

    #[test]
    fn push_requires_remote() {
        let (_td, hub) = tmp_hub();
        create_skill(&hub, "global", "x", "").unwrap();
        assert!(push(&hub).is_err());
    }

    #[test]
    fn set_remote_is_idempotent() {
        let (_td, hub) = tmp_hub();
        create_skill(&hub, "global", "x", "").unwrap();
        set_remote(&hub, "git@github.com:me/skills.git").unwrap();
        set_remote(&hub, "https://example.com/me/skills.git").unwrap();
        assert_eq!(hub_status(&hub, "global").unwrap().remote.as_deref(), Some("https://example.com/me/skills.git"));
    }
}
