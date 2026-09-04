use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::db::Db;
use crate::models::Agent;

/// P3-2: built-in agent presets, derived from the static `npx skills` agent
/// matrix plus the resource directories the app has always scanned. PRD-06:
/// the entity is the Agent (name + source), not the directory; `scan_and_persist_agents`
/// groups rows by `(name, source)` so one tool produces one Agent row with its dirs.
///
/// Skills dirs come straight from the matrix (77 agents; each contributes its
/// global dir when it has one). On top of those:
/// - `~/.agents/skills` is the npx CLI's GLOBAL CANONICAL store (project scope
///   uses `./.agents/skills`, which the per-project index scans) — attributed
///   to the synthetic "Universal Agents" row rather than any single tool;
/// - non-skills resource dirs (rules/commands/instructions/plugins) keep being
///   scanned for the Agents page and collection;
/// - `~/.skills` stays the generic fallback.
fn built_in_presets() -> Vec<(String, String, String, String, Option<String>)> {
    let mut rows: Vec<(String, String, String, String, Option<String>)> = Vec::new();

    // Canonical store first, so it wins the directory-dedup against any agent
    // whose own global dir happens to be `~/.agents/skills` (e.g. Cline).
    rows.push((
        "Universal Agents".into(),
        "universal".into(),
        "~/.agents/skills".into(),
        "skills".into(),
        Some("npx skills 的全局 canonical 目录（`skills add -g` 的实际落盘处，其他 agent 目录是指向这里的链接）。".into()),
    ));

    for a in crate::agents_table::AGENTS {
        if let Some(g) = a.global_dir {
            rows.push((
                a.display_name.to_string(),
                a.key.to_string(),
                g.to_string(),
                "skills".into(),
                Some(format!("{} 的全局 Skills 目录（npx skills 支持的 agent）。", a.display_name)),
            ));
        }
    }

    // Non-skills resource directories (scanned as before; never install targets).
    rows.push(("Cursor".into(), "cursor".into(), "~/.cursor/rules".into(), "rules".into(),
        Some("Cursor 编辑器的 Rules 目录，用于存放全局或项目级规则文件。".into())));
    rows.push(("Claude Code".into(), "claude-code".into(), "~/.claude/commands".into(), "commands".into(),
        Some("Anthropic Claude Code 的 Commands 目录，用于存放自定义斜杠命令。".into())));
    rows.push(("Codex".into(), "codex".into(), "~/.codex/instructions".into(), "instructions".into(),
        Some("OpenAI Codex CLI 的 Instructions 目录，用于存放系统级指令。".into())));
    rows.push(("ZCode".into(), "zcode".into(), "~/.zcode/cli/plugins".into(), "skills".into(),
        Some("ZCode 的插件目录，用于存放 ZCode Agent 可识别的技能。".into())));
    // Generic fallback.
    rows.push(("Generic Skills".into(), "".into(), "~/.skills".into(), "skills".into(),
        Some("用户全局 Skills 目录，作为跨 Agent 共享的兜底技能仓库。".into())));

    rows
}

/// Expand `~` or `~/` to home directory.
pub fn expand_path(path: &str) -> PathBuf {
    if path == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else if path.starts_with("~/") {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(&path[2..])
    } else {
        PathBuf::from(path)
    }
}

/// P0-3: directory names that are never skills, regardless of content
/// (e.g. `~/.agents/skills/{cache,data,marketplaces}` are symlinked support
/// dirs, and `marketplaces` carries an embedded .git mirror — importing it as
/// a skill was the incident behind this exclusion list).
pub const DEFAULT_SCAN_EXCLUSIONS: &[&str] = &[
    "cache",
    "data",
    "marketplaces",
    "node_modules",
    ".git",
    ".trash",
];

/// P0-3: a directory entry only counts as a skill when it — or, for a
/// symlink, its resolved target — contains a `SKILL.md`. `Path::is_dir` and
/// the joined `is_file` both follow symlinks, so broken links and links to
/// non-skill dirs correctly fail the check.
pub fn is_skill_dir(path: &std::path::Path) -> bool {
    path.is_dir() && path.join("SKILL.md").is_file()
}

/// P0-3: name-based scan/import exclusion. `extra` carries user-configured
/// additions from `Settings::scan_exclude_names`.
pub fn is_excluded_scan_name(name: &str, extra: &[String]) -> bool {
    DEFAULT_SCAN_EXCLUSIONS.contains(&name) || extra.iter().any(|n| n == name)
}

/// Canonical agent id for a tool name (PRD-06: stable, directory-independent slug).
fn agent_id_for(name: &str) -> String {
    format!("agent-{}", name.to_lowercase().replace(' ', "-"))
}

/// Discover agents whose directories exist on this machine. PRD-06: groups preset
/// rows by `(name, source)` into a single Agent owning all of its existing directories.
///
/// Directory-level dedup: when multiple presets resolve to the same physical
/// directory (e.g. both "Kimi Code" and "Generic Agents" point at
/// `~/.agents/skills`), only one Agent is emitted for that directory. We prefer
/// the preset with a non-empty `source` (so collection attribution still works)
/// and otherwise fall back to the first one encountered.
pub fn discover_agents() -> Vec<Agent> {
    use std::collections::BTreeMap;

    // dir -> chosen preset entry. Using the canonical path as the dedup key
    // guarantees that the same directory is never listed twice.
    let mut seen: std::collections::HashMap<PathBuf, (String, String, Option<String>)> =
        std::collections::HashMap::new();
    // (name, source) -> (dir, description) — one Agent per tool.
    let mut tools: BTreeMap<(String, String), (PathBuf, Option<String>)> = BTreeMap::new();
    for (name, source, rule, _role, description) in built_in_presets() {
        let path = expand_path(&rule);
        if !(path.exists() && path.is_dir()) {
            continue;
        }
        // Decide whether this directory is already claimed. We clone the prior
        // claim out of the map before any mutation to avoid borrow conflicts.
        let prior = seen.get(&path).cloned();
        match prior {
            // This directory was already claimed by another preset.
            Some((prev_name, prev_source, _)) => {
                let prev_has_source = !prev_source.is_empty();
                let cur_has_source = !source.is_empty();
                // Replace only if current is attributable (has source) and the
                // previously kept one is not — i.e. upgrade from generic to
                // specific tool attribution.
                if cur_has_source && !prev_has_source {
                    seen.insert(
                        path.clone(),
                        (
                            name.to_string(),
                            source.to_string(),
                            description.clone(),
                        ),
                    );
                    // Drop the previously inserted tool entry that pointed here.
                    tools.remove(&(prev_name.clone(), prev_source.clone()));
                } else {
                    // Keep the earlier entry; skip this duplicate directory.
                    continue;
                }
            }
            None => {
                seen.insert(
                    path.clone(),
                    (
                        name.to_string(),
                        source.to_string(),
                        description.clone(),
                    ),
                );
            }
        }
        let key = (name.to_string(), source.to_string());
        tools
            .entry(key)
            .or_insert_with(|| (path.clone(), description.clone()));
    }

    tools
        .into_iter()
        .map(|((name, source), (dir, description))| Agent {
            id: agent_id_for(&name),
            name,
            skill_directory: dir,
            is_enabled: true,
            discovery_rule: None,
            description,
            source: if source.is_empty() {
                None
            } else {
                Some(source)
            },
        })
        .collect()
}

/// Scan discovered agents and persist them to DB if new. PRD-06: an Agent is
/// keyed by id (tool slug); re-scans update source but don't duplicate the row.
///
/// Issue #1 cleanup: rows persisted before directory-level dedup may still
/// occupy a directory that now belongs to a different agent (e.g. a stale
/// "Generic Agents" row next to "Kimi Code", both at `~/.agents/skills`). We
/// delete such stale duplicates — but only when BOTH rows point at a directory
/// the current scan still discovers, so user-created agents pointing at
/// hand-added directories are never touched.
pub fn scan_and_persist_agents(db: &Db) -> Result<Vec<Agent>> {
    let discovered = discover_agents();
    let existing: HashSet<String> = db.get_agents()?.into_iter().map(|a| a.id).collect();

    for agent in &discovered {
        if !existing.contains(&agent.id) {
            db.insert_agent(agent)?;
        } else {
            // Touch source on an already-known row (e.g. first run after PRD-06 upgrade).
            db.insert_agent(agent)?;
        }
    }

    // Clean up legacy duplicates: for each discovered directory, any persisted
    // agent that is NOT in the current discovered set but still points at that
    // directory is a leftover from before dedup and should be removed.
    let discovered_ids: HashSet<&String> = discovered.iter().map(|a| &a.id).collect();
    let discovered_dirs: HashSet<&PathBuf> = discovered.iter().map(|a| &a.skill_directory).collect();
    for row in db.get_agents()? {
        if discovered_ids.contains(&row.id) {
            continue;
        }
        if discovered_dirs.contains(&row.skill_directory) {
            db.delete_agent(&row.id)?;
        }
    }

    db.get_agents()
}
