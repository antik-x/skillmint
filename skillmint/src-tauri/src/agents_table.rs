//! P3-2: static mirror of the `npx skills` agent matrix (vercel-labs/skills
//! `src/agents.ts`, synced 2026-09, v1.5.x era — 77 agents).
//!
//! One row per agent key accepted by `skills add -a <key>`. `project_dir` is
//! relative to the project root, `global_dir` relative to $HOME (`None` = the
//! agent has no global scope). Agents whose project dir is the shared
//! `.agents/skills` directory are "universal": that directory is also the
//! canonical store for project-scope installs, so their entries are real
//! directories rather than symlinks.

/// The shared canonical directory (relative to project root / $HOME).
#[allow(dead_code)] // consumed by scan.rs / future callers
pub const CANONICAL_PROJECT_DIR: &str = ".agents/skills";
#[allow(dead_code)] // canonical global path comes from skills_lock; kept for symmetry
pub const CANONICAL_GLOBAL_DIR: &str = "~/.agents/skills";

pub struct AgentDef {
    /// `--agent` key used by the skills CLI (e.g. `claude-code`).
    pub key: &'static str,
    pub display_name: &'static str,
    /// Project-scope skills dir, relative to the project root.
    pub project_dir: &'static str,
    /// Global-scope skills dir, relative to $HOME; `None` = no global support.
    pub global_dir: Option<&'static str>,
}

pub const AGENTS: &[AgentDef] = &[
    AgentDef { key: "adal", display_name: "AdaL", project_dir: ".adal/skills", global_dir: Some("~/.adal/skills") },
    AgentDef { key: "aider-desk", display_name: "AiderDesk", project_dir: ".aider-desk/skills", global_dir: Some("~/.aider-desk/skills") },
    AgentDef { key: "amp", display_name: "Amp", project_dir: ".agents/skills", global_dir: Some("~/.config/agents/skills") },
    AgentDef { key: "antigravity", display_name: "Antigravity", project_dir: ".agents/skills", global_dir: Some("~/.gemini/antigravity/skills") },
    AgentDef { key: "antigravity-cli", display_name: "Antigravity CLI", project_dir: ".agents/skills", global_dir: Some("~/.gemini/antigravity-cli/skills") },
    AgentDef { key: "astrbot", display_name: "AstrBot", project_dir: "data/skills", global_dir: Some("~/.astrbot/data/skills") },
    AgentDef { key: "augment", display_name: "Augment", project_dir: ".augment/skills", global_dir: Some("~/.augment/skills") },
    AgentDef { key: "autohand-code", display_name: "Autohand Code CLI", project_dir: ".autohand/skills", global_dir: Some("~/.autohand/skills") },
    AgentDef { key: "bob", display_name: "IBM Bob", project_dir: ".bob/skills", global_dir: Some("~/.bob/skills") },
    AgentDef { key: "claude-code", display_name: "Claude Code", project_dir: ".claude/skills", global_dir: Some("~/.claude/skills") },
    AgentDef { key: "cline", display_name: "Cline", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "codearts-agent", display_name: "CodeArts Agent", project_dir: ".codeartsdoer/skills", global_dir: Some("~/.codeartsdoer/skills") },
    AgentDef { key: "codebuddy", display_name: "CodeBuddy", project_dir: ".codebuddy/skills", global_dir: Some("~/.codebuddy/skills") },
    AgentDef { key: "codemaker", display_name: "Codemaker", project_dir: ".codemaker/skills", global_dir: Some("~/.codemaker/skills") },
    AgentDef { key: "codestudio", display_name: "Code Studio", project_dir: ".codestudio/skills", global_dir: Some("~/.codestudio/skills") },
    AgentDef { key: "codex", display_name: "Codex", project_dir: ".agents/skills", global_dir: Some("~/.codex/skills") },
    AgentDef { key: "command-code", display_name: "Command Code", project_dir: ".commandcode/skills", global_dir: Some("~/.commandcode/skills") },
    AgentDef { key: "continue", display_name: "Continue", project_dir: ".continue/skills", global_dir: Some("~/.continue/skills") },
    AgentDef { key: "cortex", display_name: "Cortex Code", project_dir: ".cortex/skills", global_dir: Some("~/.snowflake/cortex/skills") },
    AgentDef { key: "crush", display_name: "Crush", project_dir: ".crush/skills", global_dir: Some("~/.config/crush/skills") },
    AgentDef { key: "cursor", display_name: "Cursor", project_dir: ".agents/skills", global_dir: Some("~/.cursor/skills") },
    AgentDef { key: "deepagents", display_name: "Deep Agents", project_dir: ".agents/skills", global_dir: Some("~/.deepagents/agent/skills") },
    AgentDef { key: "devin", display_name: "Devin for Terminal", project_dir: ".devin/skills", global_dir: Some("~/.config/devin/skills") },
    AgentDef { key: "dexto", display_name: "Dexto", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "droid", display_name: "Droid", project_dir: ".factory/skills", global_dir: Some("~/.factory/skills") },
    AgentDef { key: "eve", display_name: "Eve", project_dir: "agent/skills", global_dir: None },
    AgentDef { key: "firebender", display_name: "Firebender", project_dir: ".agents/skills", global_dir: Some("~/.firebender/skills") },
    AgentDef { key: "forgecode", display_name: "ForgeCode", project_dir: ".forge/skills", global_dir: Some("~/.forge/skills") },
    AgentDef { key: "gemini-cli", display_name: "Gemini CLI", project_dir: ".agents/skills", global_dir: Some("~/.gemini/skills") },
    AgentDef { key: "github-copilot", display_name: "GitHub Copilot", project_dir: ".agents/skills", global_dir: Some("~/.copilot/skills") },
    AgentDef { key: "goose", display_name: "Goose", project_dir: ".goose/skills", global_dir: Some("~/.config/goose/skills") },
    AgentDef { key: "grok", display_name: "Grok Build", project_dir: ".grok/skills", global_dir: Some("~/.grok/skills") },
    AgentDef { key: "hermes-agent", display_name: "Hermes Agent", project_dir: ".hermes/skills", global_dir: Some("~/.hermes/skills") },
    AgentDef { key: "iflow-cli", display_name: "iFlow CLI", project_dir: ".iflow/skills", global_dir: Some("~/.iflow/skills") },
    AgentDef { key: "inference-sh", display_name: "inference.sh", project_dir: ".inferencesh/skills", global_dir: Some("~/.inferencesh/skills") },
    AgentDef { key: "jazz", display_name: "Jazz", project_dir: ".jazz/skills", global_dir: Some("~/.jazz/skills") },
    AgentDef { key: "junie", display_name: "Junie", project_dir: ".junie/skills", global_dir: Some("~/.junie/skills") },
    AgentDef { key: "kilo", display_name: "Kilo Code", project_dir: ".kilocode/skills", global_dir: Some("~/.kilocode/skills") },
    AgentDef { key: "kimchi", display_name: "Kimchi", project_dir: ".kimchi/skills", global_dir: Some("~/.config/kimchi/harness/skills") },
    AgentDef { key: "kimi-code-cli", display_name: "Kimi Code CLI", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "kiro-cli", display_name: "Kiro CLI", project_dir: ".kiro/skills", global_dir: Some("~/.kiro/skills") },
    AgentDef { key: "kode", display_name: "Kode", project_dir: ".kode/skills", global_dir: Some("~/.kode/skills") },
    AgentDef { key: "lingma", display_name: "Lingma", project_dir: ".lingma/skills", global_dir: Some("~/.lingma/skills") },
    AgentDef { key: "loaf", display_name: "Loaf", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "mcpjam", display_name: "MCPJam", project_dir: ".mcpjam/skills", global_dir: Some("~/.mcpjam/skills") },
    AgentDef { key: "minimax-code", display_name: "MiniMax Code", project_dir: ".minimax/skills", global_dir: Some("~/.minimax/skills") },
    AgentDef { key: "mistral-vibe", display_name: "Mistral Vibe", project_dir: ".vibe/skills", global_dir: Some("~/.vibe/skills") },
    AgentDef { key: "moxby", display_name: "Moxby", project_dir: ".moxby/skills", global_dir: Some("~/.moxby/skills") },
    AgentDef { key: "mux", display_name: "Mux", project_dir: ".mux/skills", global_dir: Some("~/.mux/skills") },
    AgentDef { key: "neovate", display_name: "Neovate", project_dir: ".neovate/skills", global_dir: Some("~/.neovate/skills") },
    AgentDef { key: "ona", display_name: "Ona", project_dir: ".ona/skills", global_dir: Some("~/.ona/skills") },
    AgentDef { key: "openclaw", display_name: "OpenClaw", project_dir: "skills", global_dir: Some("~/.openclaw/skills") },
    AgentDef { key: "opencode", display_name: "OpenCode", project_dir: ".agents/skills", global_dir: Some("~/.config/opencode/skills") },
    AgentDef { key: "openhands", display_name: "OpenHands", project_dir: ".openhands/skills", global_dir: Some("~/.openhands/skills") },
    AgentDef { key: "pi", display_name: "Pi", project_dir: ".pi/skills", global_dir: Some("~/.pi/agent/skills") },
    AgentDef { key: "pochi", display_name: "Pochi", project_dir: ".pochi/skills", global_dir: Some("~/.pochi/skills") },
    AgentDef { key: "posit-assistant", display_name: "Posit Assistant", project_dir: ".posit/assistant/skills", global_dir: Some("~/.posit/assistant/skills") },
    AgentDef { key: "promptscript", display_name: "PromptScript", project_dir: ".agents/skills", global_dir: None },
    AgentDef { key: "qoder", display_name: "Qoder", project_dir: ".qoder/skills", global_dir: Some("~/.qoder/skills") },
    AgentDef { key: "qoder-cn", display_name: "Qoder CN", project_dir: ".qoder/skills", global_dir: Some("~/.qoder-cn/skills") },
    AgentDef { key: "qwen-code", display_name: "Qwen Code", project_dir: ".qwen/skills", global_dir: Some("~/.qwen/skills") },
    AgentDef { key: "reasonix", display_name: "Reasonix", project_dir: ".reasonix/skills", global_dir: Some("~/.reasonix/skills") },
    AgentDef { key: "replit", display_name: "Replit", project_dir: ".agents/skills", global_dir: Some("~/.config/agents/skills") },
    AgentDef { key: "roo", display_name: "Roo Code", project_dir: ".roo/skills", global_dir: Some("~/.roo/skills") },
    AgentDef { key: "rovodev", display_name: "Rovo Dev", project_dir: ".rovodev/skills", global_dir: Some("~/.rovodev/skills") },
    AgentDef { key: "tabnine-cli", display_name: "Tabnine CLI", project_dir: ".tabnine/agent/skills", global_dir: Some("~/.tabnine/agent/skills") },
    AgentDef { key: "terramind", display_name: "Terramind", project_dir: ".terramind/skills", global_dir: Some("~/.terramind/skills") },
    AgentDef { key: "tinycloud", display_name: "Tinycloud", project_dir: ".tinycloud/skills", global_dir: Some("~/.tinycloud/skills") },
    AgentDef { key: "trae", display_name: "Trae", project_dir: ".trae/skills", global_dir: Some("~/.trae/skills") },
    AgentDef { key: "trae-cn", display_name: "Trae CN", project_dir: ".trae/skills", global_dir: Some("~/.trae-cn/skills") },
    AgentDef { key: "universal", display_name: "Universal", project_dir: ".agents/skills", global_dir: Some("~/.config/agents/skills") },
    AgentDef { key: "warp", display_name: "Warp", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "windsurf", display_name: "Windsurf", project_dir: ".windsurf/skills", global_dir: Some("~/.codeium/windsurf/skills") },
    AgentDef { key: "zcode", display_name: "ZCode", project_dir: ".zcode/skills", global_dir: Some("~/.zcode/skills") },
    AgentDef { key: "zed", display_name: "Zed", project_dir: ".agents/skills", global_dir: Some("~/.agents/skills") },
    AgentDef { key: "zencoder", display_name: "Zencoder", project_dir: ".zencoder/skills", global_dir: Some("~/.zencoder/skills") },
    AgentDef { key: "zenflow", display_name: "Zenflow", project_dir: ".zencoder/skills", global_dir: Some("~/.zencoder/skills") },
];

/// Look up an agent definition by its CLI key.
#[allow(dead_code)] // exercised by unit tests; lookup helper for callers
pub fn get(key: &str) -> Option<&'static AgentDef> {
    AGENTS.iter().find(|a| a.key == key)
}

/// True when the agent's project skills dir is the shared canonical directory.
pub fn is_universal(a: &AgentDef) -> bool {
    a.project_dir == CANONICAL_PROJECT_DIR
}

/// Resolve a global dir like `~/.claude/skills` to an absolute path.
pub fn expand_global(global_dir: &str) -> std::path::PathBuf {
    crate::scan::expand_path(global_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_core_agents() {
        assert!(AGENTS.len() >= 70, "expected the full npx skills matrix, got {}", AGENTS.len());
        for key in ["claude-code", "codex", "cursor", "zcode", "opencode", "windsurf", "gemini-cli"] {
            assert!(get(key).is_some(), "missing agent {key}");
        }
    }

    #[test]
    fn canonical_dirs_match_npx_convention() {
        let claude = get("claude-code").unwrap();
        assert_eq!(claude.project_dir, ".claude/skills");
        assert_eq!(claude.global_dir, Some("~/.claude/skills"));
        let zcode = get("zcode").unwrap();
        assert_eq!(zcode.project_dir, ".zcode/skills");
        assert_eq!(zcode.global_dir, Some("~/.zcode/skills"));
        // Codex and Cursor install project-scope into the shared canonical dir.
        assert!(is_universal(get("codex").unwrap()));
        assert!(is_universal(get("cursor").unwrap()));
        // eve / promptscript have no global scope.
        assert_eq!(get("eve").unwrap().global_dir, None);
    }

    #[test]
    fn global_dirs_are_home_relative() {
        for a in AGENTS {
            if let Some(g) = a.global_dir {
                assert!(g.starts_with("~/") || g.starts_with('/'), "{} global dir {g}", a.key);
            }
        }
    }
}
