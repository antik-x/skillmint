//! P3-3: parsers for the two `npx skills` lock files — the on-disk source of
//! truth SkillMint indexes (Mole model: the app never owns install state).
//!
//! - Project scope: `<project-root>/skills-lock.json` (schema v1, committed to
//!   git; entries keyed by skill name, hash = sha256 over the skill folder's
//!   files as computed by the CLI).
//! - Global scope: `~/.agents/.skill-lock.json` (schema v3; hash = GitHub tree
//!   SHA of the skill folder; `$XDG_STATE_HOME/skills/` overrides the dir).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

pub const PROJECT_LOCK_FILE: &str = "skills-lock.json";
pub const GLOBAL_LOCK_DIR: &str = "~/.agents";
pub const GLOBAL_LOCK_FILE: &str = ".skill-lock.json";
pub const GLOBAL_CANONICAL_DIR: &str = "~/.agents/skills";

/// The shared canonical store for project-scope installs (relative name only).
pub const PROJECT_CANONICAL_DIR: &str = ".agents/skills";

/// One entry of the project lock. Timestamp-free by the CLI's design (clean
/// git merges); `sourceType` distinguishes github / node_modules / local / ….
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // lock files are parsed in full; some fields are informational only
pub struct ProjectLockEntry {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub source_type: String,
    #[serde(default)]
    pub skill_path: Option<String>,
    #[serde(default)]
    pub computed_hash: String,
    #[serde(default)]
    pub subagents: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectLock {
    #[serde(deserialize_with = "serde_version", default)]
    pub version: u32,
    #[serde(default)]
    pub skills: BTreeMap<String, ProjectLockEntry>,
}

fn serde_version<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64().unwrap_or(0) as u32)
}

/// One entry of the global lock.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // lock files are parsed in full; some fields are informational only
pub struct GlobalLockEntry {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_type: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub skill_path: Option<String>,
    #[serde(default)]
    pub skill_folder_hash: String,
    #[serde(default)]
    pub installed_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub well_known_digest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GlobalLock {
    #[serde(deserialize_with = "serde_version", default)]
    pub version: u32,
    #[serde(default)]
    pub skills: BTreeMap<String, GlobalLockEntry>,
    #[serde(default, rename = "lastSelectedAgents")]
    pub last_selected_agents: Vec<String>,
}

/// `~/.agents/.skill-lock.json` (honoring `$XDG_STATE_HOME/skills/` like the CLI).
pub fn global_lock_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.trim().is_empty() {
            return Path::new(&xdg).join("skills").join(GLOBAL_LOCK_FILE);
        }
    }
    crate::scan::expand_path(&format!("{GLOBAL_LOCK_DIR}/{GLOBAL_LOCK_FILE}"))
}

/// The canonical global install dir (`~/.agents/skills`).
pub fn global_canonical_path() -> PathBuf {
    crate::scan::expand_path(GLOBAL_CANONICAL_DIR)
}

pub fn load_project_lock(project_root: &Path) -> Result<Option<ProjectLock>> {
    let path = project_root.join(PROJECT_LOCK_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let lock: ProjectLock = serde_json::from_str(&content)?;
    Ok(Some(lock))
}

pub fn load_global_lock() -> Result<Option<GlobalLock>> {
    let path = global_lock_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let lock: GlobalLock = serde_json::from_str(&content)?;
    Ok(Some(lock))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_project_lock_v1() {
        let raw = r#"{
          "version": 1,
          "skills": {
            "pdf": {
              "source": "vercel-labs/agent-skills",
              "sourceUrl": "https://github.com/vercel-labs/agent-skills",
              "sourceType": "github",
              "skillPath": "skills/pdf/SKILL.md",
              "computedHash": "sha256-abc123",
              "subagents": []
            },
            "my-tool": {
              "source": "./skills-hub/my-tool",
              "sourceType": "local",
              "computedHash": "sha256-def456"
            }
          }
        }"#;
        let lock: ProjectLock = serde_json::from_str(raw).unwrap();
        assert_eq!(lock.version, 1);
        assert_eq!(lock.skills.len(), 2);
        let pdf = lock.skills.get("pdf").unwrap();
        assert_eq!(pdf.source_type, "github");
        assert_eq!(pdf.skill_path.as_deref(), Some("skills/pdf/SKILL.md"));
        let mine = lock.skills.get("my-tool").unwrap();
        assert_eq!(mine.source_type, "local");
        assert!(mine.skill_path.is_none());
    }

    #[test]
    fn parse_global_lock_v3() {
        let raw = r#"{
          "version": 3,
          "skills": {
            "docx": {
              "source": "vercel-labs/agent-skills",
              "sourceType": "github",
              "sourceUrl": "https://github.com/vercel-labs/agent-skills",
              "skillPath": "skills/docx",
              "skillFolderHash": "abc123tree",
              "installedAt": "2026-08-01T10:00:00.000Z",
              "updatedAt": "2026-08-19T10:00:00.000Z"
            }
          },
          "lastSelectedAgents": ["claude-code", "zcode"]
        }"#;
        let lock: GlobalLock = serde_json::from_str(raw).unwrap();
        assert_eq!(lock.version, 3);
        let e = lock.skills.get("docx").unwrap();
        assert_eq!(e.skill_folder_hash, "abc123tree");
        assert_eq!(lock.last_selected_agents, vec!["claude-code", "zcode"]);
    }

    #[test]
    fn missing_fields_tolerated() {
        let lock: ProjectLock =
            serde_json::from_str(r#"{"version": 1, "skills": {"x": {"source": "s", "sourceType": "local"}}}"#).unwrap();
        assert_eq!(lock.skills["x"].computed_hash, "");
        let g: GlobalLock = serde_json::from_str(r#"{"version": 3, "skills": {}}"#).unwrap();
        assert!(g.skills.is_empty());
    }

    #[test]
    fn paths_follow_cli_conventions() {
        assert!(global_lock_path().ends_with(".agents/.skill-lock.json"));
        assert!(global_canonical_path().ends_with(".agents/skills"));
    }

    #[test]
    fn version_field_accepts_number_only() {
        let lock: ProjectLock = serde_json::from_str(r#"{"version": 9, "skills": {}}"#).unwrap();
        assert_eq!(lock.version, 9);
    }
}
