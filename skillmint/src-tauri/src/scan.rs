use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::db::Db;
use crate::models::Agent;

/// One built-in agent preset. PRD-06: the entity is the Agent (name + source),
/// not the directory; a tool may own multiple directories. Here each preset row is
/// one (tool, source, directory, role) — `scan_and_persist_agents` groups rows by
/// `(name, source)` so that one tool produces one Agent row with N directories.
const PRESETS: &[(&str, &str, &str, &str, Option<&str>)] = &[
    // (name, source, directory_rule, role, description)
    ("Cursor", "cursor", "~/.cursor/skills", "skills", Some("Cursor 编辑器的 Skills 目录，用于存放可被 Cursor Agent 调用的技能包。")),
    ("Cursor", "cursor", "~/.cursor/rules", "rules", Some("Cursor 编辑器的 Rules 目录，用于存放全局或项目级规则文件。")),
    ("Claude Code", "claude-code", "~/.claude/skills", "skills", Some("Anthropic Claude Code 的 Skills 目录，供 Claude 在对话中引用。")),
    ("Claude Code", "claude-code", "~/.claude/commands", "commands", Some("Anthropic Claude Code 的 Commands 目录，用于存放自定义斜杠命令。")),
    ("Codex", "codex", "~/.codex/skills", "skills", Some("OpenAI Codex CLI 的 Skills 目录，供 Codex Agent 使用。")),
    ("Codex", "codex", "~/.codex/instructions", "instructions", Some("OpenAI Codex CLI 的 Instructions 目录，用于存放系统级指令。")),
    ("ZCode", "zcode", "~/.zcode/cli/plugins", "skills", Some("ZCode 的插件目录，用于存放 ZCode Agent 可识别的技能。")),
    // PRD-08 P1: OpenCode usage data is collected, so it gets a source-keyed
    // Agent for attribution (matches the collector's SOURCE_OPENCODE).
    ("OpenCode", "opencode", "~/.local/share/opencode/skills", "skills", Some("OpenCode 的 Skills 目录，供 OpenCode Agent 使用。")),
    ("Kiro", "kiro", "~/.kiro/skills", "skills", Some("Kiro 的 Skills 目录，用于存放 Kiro Agent 可识别的技能。")),
    ("Lingma", "lingma", "~/.lingma/skills", "skills", Some("通义灵码的 Skills 目录，用于存放灵码 Agent 可使用的技能。")),
    ("CoPaw", "copaw", "~/.copaw/skills", "skills", Some("CoPaw 的 Skills 目录，用于存放 CoPaw Agent 可识别的技能。")),
    ("OpenClaw", "openclaw", "~/.openclaw/skills", "skills", Some("OpenClaw 的 Skills 目录，用于存放 OpenClaw Agent 可使用的技能。")),
    ("Kimi Code", "kimi-code", "~/.agents/skills", "skills", Some("Kimi Code CLI 的 Skills 目录，对应其默认用户级 skills 路径 ~/.agents/skills。")),
    ("Generic Agents", "", "~/.agents/skills", "skills", Some("通用 Agent Skills 目录，覆盖 Cursor/Gemini/Trae/OpenCode 等本机没有独立 skills 目录的 Agent 工具。")),
    ("Generic Skills", "", "~/.skills", "skills", Some("用户全局 Skills 目录，作为跨 Agent 共享的兜底技能仓库。")),
];

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
pub fn discover_agents() -> Vec<Agent> {
    use std::collections::BTreeMap;

    // (name, source) -> (first_dir, description) — one Agent per tool.
    let mut tools: BTreeMap<(String, String), (PathBuf, Option<String>)> = BTreeMap::new();
    for (name, source, rule, _role, description) in PRESETS {
        let path = expand_path(rule);
        if path.exists() && path.is_dir() {
            let key = (name.to_string(), source.to_string());
            tools
                .entry(key)
                .or_insert_with(|| (path.clone(), description.map(|s| s.to_string())));
        }
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

    db.get_agents()
}
