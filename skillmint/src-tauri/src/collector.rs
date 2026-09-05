//! PRD-02: usage data collection.
//!
//! Each AI Coding Agent writes session/usage data to local on-disk files in its own
//! private format. A [`Collector`] knows how to read one agent's format and normalize
//! it into SkillMint's `collected_*` tables. Collection is **strictly read-only**:
//! we never write to agent directories, intercept network traffic, or alter agent
//! configuration (see PRD-02 hard constraints).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::params;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::db::Db;
use crate::fs::compute_hash;

/// SPEC-F3: open a file with 3 back-off retries for transient exclusive locks.
/// Delays are 100ms, 300ms, 900ms. Returns the final error if all retries fail.
pub fn open_file_with_retry(path: &Path) -> std::io::Result<File> {
    let delays = [100u64, 300, 900];
    let mut last_err = None;
    for (i, delay_ms) in delays.iter().enumerate() {
        match File::open(path) {
            Ok(f) => return Ok(f),
            Err(e) => {
                last_err = Some(e);
                if i < delays.len() - 1 {
                    std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "file open failed after retries")
    }))
}

use crate::models::{
    CollectedCodeContribution, CollectedFileState, CollectedPrompt, CollectedSession,
    CollectedTokenUsage, CollectionStats, SkillUsageAttribution,
};
use crate::settings::BillingMode;

/// The source identifier for Claude Code.
pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
/// The source identifier for Codex CLI.
pub const SOURCE_CODEX: &str = "codex";
/// PRD-05: source identifier for ZCode (CLI / IDE agent).
pub const SOURCE_ZCODE: &str = "zcode";
/// PRD-05: source identifier for Cursor (code-contribution only, no tokens).
pub const SOURCE_CURSOR: &str = "cursor";
/// PRD-08 P1: source identifier for OpenCode (SQLite; has tokens + prompts).
pub const SOURCE_OPENCODE: &str = "opencode";
/// PRD-08 P2: source identifiers for the session-only collectors (no tokens).
pub const SOURCE_CODEBUDDY: &str = "codebuddy";
pub const SOURCE_ANTIGRAVITY: &str = "antigravity";
pub const SOURCE_GEMINI_CLI: &str = "gemini-cli";
pub const SOURCE_TRAE: &str = "trae";
pub const SOURCE_TRAE_CN: &str = "trae-cn";
pub const SOURCE_TRAE_SOLO: &str = "trae-solo";
/// Kimi Code CLI (native wire.jsonl format; tokens + prompts persisted locally).
/// P3-10: value aligned with the agents_table matrix key `kimi-code-cli` —
/// attribution is `collected_sessions.source == agents.source` string equality,
/// so the tag must spell the product the same way the agent row does. (Note
/// SOURCE_ANTIGRAVITY stays "antigravity": that collector reads the Antigravity
/// *IDE* brain dirs, a different product from the `antigravity-cli` matrix key.)
pub const SOURCE_KIMI_CODE: &str = "kimi-code-cli";

/// PRD-08: per-source capability flags. Tells the analysis engine which signals
/// a source contributes honestly. Mirrors AI-Digest `TOOL_CAPABILITIES`. A flag
/// of 0 means "this source does not persist that signal locally" — the engine
/// must then report "数据不足" rather than fabricating a 0 (the fix for the
/// "zcode冒充全局" defect). `billing_mode` feeds the cost split.
#[derive(Debug, Clone, Copy)]
pub struct ToolCapabilities {
    pub has_token: bool,
    pub has_prompt: bool,
    pub has_tool_calls: bool,
    /// Whether the source persists trustworthy per-turn duration. Currently only
    /// ZCode (via its `turn_usage` table). Used by the agent-coefficient calc.
    pub has_turn_duration: bool,
    pub billing_mode: BillingMode,
}

/// PRD-08: the capability table for every source SkillMint currently collects.
/// Kept aligned with `discover_all_collectors()`. As new collectors land
/// (PRD-08 P2: opencode/codebuddy/trae/...), extend this table.
pub const ALL_TOOL_CAPABILITIES: &[(&str, ToolCapabilities)] = &[
    (
        SOURCE_CLAUDE_CODE,
        ToolCapabilities {
            has_token: true,
            has_prompt: true,
            has_tool_calls: true,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    (
        SOURCE_CODEX,
        ToolCapabilities {
            has_token: true,
            has_prompt: true,
            has_tool_calls: true,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    (
        SOURCE_ZCODE,
        ToolCapabilities {
            has_token: true,
            has_prompt: true,
            has_tool_calls: true,
            has_turn_duration: true,
            billing_mode: BillingMode::Subscription,
        },
    ),
    (
        SOURCE_CURSOR,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::Subscription,
        },
    ),
    // PRD-08 P1: OpenCode persists tokens (incl. cache read/write + reasoning)
    // and prompt text in its SQLite db — a full-fidelity source like ZCode.
    (
        SOURCE_OPENCODE,
        ToolCapabilities {
            has_token: true,
            has_prompt: true,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    // Kimi Code CLI: native wire.jsonl with usage.record + turn.prompt.
    (
        SOURCE_KIMI_CODE,
        ToolCapabilities {
            has_token: true,
            has_prompt: true,
            has_tool_calls: true,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    // PRD-08 P2: session-only sources (no local token/prompt persistence).
    (
        SOURCE_CODEBUDDY,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::Subscription,
        },
    ),
    (
        SOURCE_ANTIGRAVITY,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    (
        SOURCE_GEMINI_CLI,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::PayAsYouGo,
        },
    ),
    (
        SOURCE_TRAE,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::Subscription,
        },
    ),
    (
        SOURCE_TRAE_CN,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::Subscription,
        },
    ),
    (
        SOURCE_TRAE_SOLO,
        ToolCapabilities {
            has_token: false,
            has_prompt: false,
            has_tool_calls: false,
            has_turn_duration: false,
            billing_mode: BillingMode::Subscription,
        },
    ),
];

/// A collector reads one agent's local on-disk data and normalizes it into the DB.
pub trait Collector: Send + Sync {
    /// Stable source key, e.g. `claude-code`. Used as the `source` column everywhere.
    fn source(&self) -> &str;
    /// Human-readable kind for the `collected_sources` table, e.g. `direct_file`.
    fn collector_kind(&self) -> &str;
    /// Data root path this collector reads from (for display / diagnostics).
    fn data_path(&self) -> String;
    /// Whether the data source exists and is likely readable.
    fn is_available(&self) -> bool;
    /// Read everything and upsert into the DB. Must not panic on individual file/line
    /// errors — skip and keep going (hard constraint: a single source's failure must
    /// not block the rest).
    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats>;
}

/// Read file metadata and compare it with the stored `collector_file_states` row.
/// Returns true when the file is new or has changed.
/// Change detection uses mtime/size as a fast filter, then SHA-256 content hash
/// as the authoritative check. This avoids missing changes when a file is
/// rewritten with identical size/mtime (rare but possible) and avoids false
/// positives when only metadata changes.
pub fn is_file_changed(db: &Db, source: &str, path: &Path) -> Result<bool> {
    let metadata = std::fs::metadata(path)?;
    let mtime_ns = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;
    let size = metadata.len() as i64;

    match db.get_collected_file_state(source, &path.to_string_lossy())? {
        Some(state) => {
            let meta_changed = state.last_modified_ns != mtime_ns || state.last_size != size;
            if !meta_changed {
                // Fast path: metadata unchanged, assume content unchanged.
                return Ok(false);
            }
            // Metadata changed: confirm with content hash before re-collecting.
            let current_hash = compute_hash(path).unwrap_or_default();
            Ok(state.content_hash.as_deref() != Some(current_hash.as_str()))
        }
        None => Ok(true),
    }
}

/// Persist file metadata after it has been successfully collected.
pub fn mark_file_collected(db: &Db, source: &str, path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mtime_ns = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;
    let size = metadata.len() as i64;
    let content_hash = compute_hash(path).ok();
    db.upsert_collected_file_state(&CollectedFileState {
        source: source.to_string(),
        file_path: path.to_string_lossy().to_string(),
        last_modified_ns: mtime_ns,
        last_size: size,
        last_collected_at: now_secs(),
        content_hash,
    })
}

/// Build the list of all collectors to run. PRD-02 ships Claude Code + Codex;
/// PRD-05 adds ZCode (SQLite) and Cursor (ai-tracking code contribution).
pub fn discover_all_collectors() -> Vec<Box<dyn Collector>> {
    let mut collectors: Vec<Box<dyn Collector>> = vec![
        Box::new(ClaudeCodeCollector::default()),
        Box::new(CodexCollector::default()),
        Box::new(ZCodeCollector::default()),
        Box::new(CursorCollector::default()),
        Box::new(OpenCodeCollector::default()),
        Box::new(KimiCodeCollector::default()),
    ];
    // PRD-08 P2: the 6 session-only collectors (CodeBuddy, Antigravity, Gemini
    // CLI, Trae / Trae-CN / Trae-Solo).
    collectors.extend(crate::collectors_ext::all_session_only_collectors());
    collectors
}

// =============================================================================
// Claude Code
// =============================================================================

/// Reads `~/.claude/projects/<encoded-path>/<sessionUuid>.jsonl`.
pub struct ClaudeCodeCollector {
    root: PathBuf,
}

impl Default for ClaudeCodeCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join("projects");
        Self { root }
    }
}

impl ClaudeCodeCollector {
    /// Construct with a custom root (used by tests to point at a tempdir).
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Collector for ClaudeCodeCollector {
    fn source(&self) -> &str {
        SOURCE_CLAUDE_CODE
    }
    fn collector_kind(&self) -> &str {
        "direct_file"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats); // empty stats; caller marks source as not_found
        }

        // session_id (raw uuid filename) -> aggregated token accumulators per model
        let mut sessions = 0i64;
        let mut total_skipped = 0i64;

        // Walk all *.jsonl files under root.
        for entry in WalkDir::new(&self.root)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            // Incremental: skip files whose mtime/size have not changed.
            match is_file_changed(db, self.source(), path) {
                Ok(false) => {
                    total_skipped += 1;
                    continue;
                }
                Ok(true) => {}
                Err(e) => {
                    eprintln!("[collector] failed to stat {}: {}", path.display(), e);
                    total_skipped += 1;
                    continue;
                }
            }

            match self.collect_one_file(db, device_id, path) {
                Ok(FileStats {
                    session_upserted,
                    prompts,
                    skipped,
                }) => {
                    eprintln!("[codex-dbg] file done: upserted={} prompts={} skipped={}", session_upserted, prompts, skipped);
                    if session_upserted {
                        sessions += 1;
                    }
                    stats.prompts += prompts;
                    total_skipped += skipped;
                    if let Err(e) = mark_file_collected(db, self.source(), path) {
                        eprintln!("[collector] failed to mark {}: {}", path.display(), e);
                    }
                }
                Err(e) => {
                    // A single broken file must not abort the whole collection.
                    eprintln!("[collector] failed to read {}: {}", path.display(), e);
                    total_skipped += 1;
                }
            }
        }

        stats.sessions = sessions;
        stats.token_rows = -1; // not individually counted (folded into sessions); see note
        stats.skipped = total_skipped;
        Ok(stats)
    }
}

struct FileStats {
    session_upserted: bool,
    prompts: i64,
    skipped: i64,
}

impl ClaudeCodeCollector {
    fn collect_one_file(
        &self,
        db: &Db,
        device_id: &str,
        path: &Path,
    ) -> Result<FileStats> {
        // session id = filename stem (a UUID)
        let session_raw = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let session_pk = format!("{device_id}:claude-code:{session_raw}");

        let file = open_file_with_retry(path).map_err(|e| {
            anyhow::anyhow!("无法打开文件 {}：{}（文件可能被其他程序占用）", path.display(), e)
        })?;
        let reader = BufReader::new(file);

        let mut prompts = 0i64;
        let mut skipped = 0i64;
        let mut message_count = 0i64;
        let mut first_prompt: Option<String> = None;
        let mut start_ts: Option<u64> = None;
        let mut end_ts: Option<u64> = None;
        let mut project_path: Option<String> = None; // first non-empty cwd seen
        // model -> accumulated usage for this session
        let mut per_model: HashMap<String, ModelAccum> = HashMap::new();
        let now = now_secs();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: JsonlLine = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1; // malformed line — skip, keep going
                    continue;
                }
            };

            message_count += 1;
            if project_path.is_none() {
                if let Some(cwd) = &parsed.cwd {
                    if !cwd.is_empty() {
                        project_path = Some(cwd.clone());
                    }
                }
            }
            if let Some(ts) = parsed.timestamp_secs() {
                start_ts = Some(start_ts.map_or(ts, |s| s.min(ts)));
                end_ts = Some(end_ts.map_or(ts, |e| e.max(ts)));
            }

            match parsed.line_type.as_str() {
                "user" => {
                    // Real user prompt (not a tool_result echo).
                    let text = parsed.user_prompt_text();
                    if let Some(t) = text {
                        if first_prompt.is_none() {
                            first_prompt = Some(t.chars().take(200).collect());
                        }
                        if let Some(uuid) = &parsed.uuid {
                            db.upsert_collected_prompt(&CollectedPrompt {
                                id: format!("{session_pk}:{uuid}"),
                                device_id: device_id.to_string(),
                                session_id: session_pk.clone(),
                                source: self.source().to_string(),
                                project_id: None, // linked by link_sessions_to_projects
                                prompt_text: Some(t.chars().take(2000).collect()),
                                started_at: parsed.timestamp_secs(),
                                duration_ms: None,
                                requested_action: None,
                                target_object: None,
                                interaction_state: None,
                                interaction_mode: None,
                                confidence: None,
                                tool_calls: None,
                                tool_errors: None,
                            })?;
                            prompts += 1;
                        }
                    }
                }
                "assistant" => {
                    if let Some(msg) = &parsed.message {
                        if let Some(usage) = &msg.usage {
                            // Skip streaming-placeholder lines with all-zero tokens.
                            if usage.input_tokens > 0 || usage.output_tokens > 0 {
                                let model = msg
                                    .model
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_string());
                                let acc = per_model.entry(model.clone()).or_default();
                                acc.input += usage.input_tokens;
                                acc.output += usage.output_tokens;
                                acc.cache_creation += usage.cache_creation_input_tokens;
                                acc.cache_read += usage.cache_read_input_tokens;
                                acc.calls += 1;
                            }
                        }
                        // PRD-02: skill usage attribution — detect `tool_use` of the Skill tool.
                        if let Some(uuid) = &parsed.uuid {
                            for skill_name in msg.skill_invocations() {
                                let normalized = strip_skill_prefix(&skill_name);
                                let skill_id = db.skill_id_by_name(&normalized).unwrap_or(None);
                                let attr_id = format!("{session_pk}:{uuid}:{normalized}");
                                db.upsert_skill_attribution(&SkillUsageAttribution {
                                    id: attr_id,
                                    device_id: device_id.to_string(),
                                    skill_id,
                                    skill_name: normalized,
                                    source: self.source().to_string(),
                                    session_id: session_pk.clone(),
                                    project_id: None,
                                    agent_id: None,
                                    attribution_type: "direct".to_string(),
                                    attributed_at: now,
                                })?;
                            }
                            // PRD-02 §4.1: indirect attribution — agent Read/Edited a SKILL.md path.
                            for skill_name in msg.skill_file_reads() {
                                let normalized = strip_skill_prefix(&skill_name);
                                let skill_id = db.skill_id_by_name(&normalized).unwrap_or(None);
                                let attr_id =
                                    format!("{session_pk}:{uuid}:indirect:{normalized}");
                                db.upsert_skill_attribution(&SkillUsageAttribution {
                                    id: attr_id,
                                    device_id: device_id.to_string(),
                                    skill_id,
                                    skill_name: normalized,
                                    source: self.source().to_string(),
                                    session_id: session_pk.clone(),
                                    project_id: None,
                                    agent_id: None,
                                    attribution_type: "indirect".to_string(),
                                    attributed_at: now,
                                })?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Resolve the session's project_id (and attribute project to skill rows) if we have a cwd.
        let project_id = match &project_path {
            Some(p) => db.ensure_project_by_path(device_id, p).unwrap_or(None),
            None => None,
        };

        // Upsert session + per-model token rows.
        let cached_at = now;
        db.upsert_collected_session(&CollectedSession {
            id: session_pk.clone(),
            device_id: device_id.to_string(),
            source: self.source().to_string(),
            project_id: project_id.clone(),
            agent_id: None,
            start_time: start_ts,
            end_time: end_ts,
            message_count,
            title_or_prompt: first_prompt,
            cached_at,
            project_path,
            quality_score: Some(quality_score(message_count, prompts)),
        })?;

        for (model, acc) in per_model {
            db.upsert_collected_token_usage(&CollectedTokenUsage {
                id: format!("{session_pk}:{}", model),
                device_id: device_id.to_string(),
                session_id: session_pk.clone(),
                source: self.source().to_string(),
                project_id: project_id.clone(),
                model_id: Some(model),
                input_tokens: acc.input,
                output_tokens: acc.output,
                reasoning_tokens: 0, // not exposed in Claude Code usage object
                cache_creation_input_tokens: acc.cache_creation,
                cache_read_input_tokens: acc.cache_read,
                total_tokens: acc.input + acc.output,
                model_calls: acc.calls,
                tool_calls: 0,
                duration_ms: None,
            })?;
        }

        Ok(FileStats {
            session_upserted: true,
            prompts,
            skipped,
        })
    }
}

#[derive(Default)]
struct ModelAccum {
    input: i64,
    output: i64,
    cache_creation: i64,
    cache_read: i64,
    calls: i64,
}

// -----------------------------------------------------------------------------
// Minimal deserialization of the Claude Code jsonl line format.
// We deserialize only what we need; unknown fields are ignored. `message.content`
// may be a string or an array of blocks.
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonlLine {
    #[serde(default, rename = "type")]
    line_type: String,
    uuid: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    message: Option<Message>,
}

impl JsonlLine {
    fn timestamp_secs(&self) -> Option<u64> {
        let s = self.timestamp.as_ref()?;
        // ISO-8601 e.g. "2026-06-23T16:23:25.358Z"
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp().max(0) as u64)
    }

    /// Extract user prompt text only for genuine user turns (string content, not a
    /// tool_result echo array).
    fn user_prompt_text(&self) -> Option<String> {
        let msg = self.message.as_ref()?;
        if msg.role.as_deref() != Some("user") {
            return None;
        }
        match &msg.content {
            Some(Content::String(s)) => Some(s.clone()),
            Some(Content::Array(blocks)) => {
                // Only treat a text block with no tool_result as a real prompt.
                blocks.iter().find_map(|b| match b {
                    Block::Text { text } => Some(text.clone()),
                    _ => None,
                })
            }
            None => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Message {
    role: Option<String>,
    content: Option<Content>,
    model: Option<String>,
    usage: Option<Usage>,
}

impl Message {
    /// Names of skills invoked via the `Skill` tool in this message
    /// (tool_use blocks with name=="Skill", input.skill==<name>).
    fn skill_invocations(&self) -> Vec<String> {
        let blocks = match &self.content {
            Some(Content::Array(b)) => b,
            _ => return vec![],
        };
        blocks
            .iter()
            .filter_map(|b| match b {
                Block::ToolUse { name, input } if name.as_deref() == Some("Skill") => {
                    input.skill.clone()
                }
                _ => None,
            })
            .collect()
    }

    /// PRD-02 §4.1 indirect attribution: skill names inferred from tool_use blocks
    /// (Read/Edit/Write) whose file_path contains `/skills/<name>/SKILL.md`.
    /// Excludes authoring of unrelated files; returns the <name> segment.
    fn skill_file_reads(&self) -> Vec<String> {
        let blocks = match &self.content {
            Some(Content::Array(b)) => b,
            _ => return vec![],
        };
        let mut found = Vec::new();
        for b in blocks.iter() {
            if let Block::ToolUse { name, input } = b {
                let is_file_tool = matches!(
                    name.as_deref(),
                    Some("Read") | Some("Edit") | Some("Write") | Some("MultiEdit")
                );
                if !is_file_tool {
                    continue;
                }
                if let Some(path) = &input.file_path {
                    // Match .../skills/<name>/SKILL.md (or .../skills/<name>)
                    if let Some(name) = extract_skill_name_from_path(path) {
                        found.push(name);
                    }
                }
            }
        }
        found
    }
}

/// Extract the skill directory name from a path like `.../skills/<name>/SKILL.md`.
fn extract_skill_name_from_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let idx = normalized.find("/skills/")?;
    let after = &normalized[idx + 8..]; // skip "/skills/"
    let name = after.split('/').next()?;
    let name = name.trim();
    if name.is_empty() || name == "SKILL.md" {
        None
    } else {
        Some(name.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Content {
    String(String),
    Array(Vec<Block>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text { text: String },
    // `input` is the tool input object; for the Skill tool it carries `{skill: "..."}`.
    ToolUse {
        name: Option<String>,
        #[serde(default)]
        input: ToolInput,
    },
    #[allow(dead_code)]
    ToolResult {
        content: Option<serde_json::Value>,
    },
    #[serde(other)]
    #[allow(dead_code)]
    Other,
}

/// Tool input payload; `skill` for the Skill tool, `file_path` for Read/Edit.
#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    skill: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
}

/// Strip a `plugin:` namespace prefix from a skill name
/// (e.g. `superpowers:systematic-debugging` -> `systematic-debugging`).
fn strip_skill_prefix(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((prefix, rest)) if !prefix.is_empty() && !rest.is_empty() => rest.to_string(),
        _ => name.to_string(),
    }
}


#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
}

// =============================================================================
// Codex CLI
// =============================================================================

/// Reads `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<ts>-<uuid>.jsonl`.
pub struct CodexCollector {
    root: PathBuf,
}

impl Default for CodexCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .join("sessions");
        Self { root }
    }
}

impl CodexCollector {
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Collector for CodexCollector {
    fn source(&self) -> &str {
        SOURCE_CODEX
    }
    fn collector_kind(&self) -> &str {
        "direct_file"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats);
        }

        let mut sessions = 0i64;
        let mut total_skipped = 0i64;
        for entry in WalkDir::new(&self.root)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file()
                || path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                || !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("rollout-"))
                    .unwrap_or(false)
            {
                continue;
            }

            match is_file_changed(db, self.source(), path) {
                Ok(false) => {
                    total_skipped += 1;
                    continue;
                }
                Ok(true) => {}
                Err(e) => {
                    eprintln!("[collector] failed to stat {}: {}", path.display(), e);
                    total_skipped += 1;
                    continue;
                }
            }

            match self.collect_one_file(db, device_id, path) {
                Ok(FileStats {
                    session_upserted,
                    prompts,
                    skipped,
                }) => {
                    if session_upserted {
                        sessions += 1;
                    }
                    stats.prompts += prompts;
                    total_skipped += skipped;
                    if let Err(e) = mark_file_collected(db, self.source(), path) {
                        eprintln!("[collector] failed to mark {}: {}", path.display(), e);
                    }
                }
                Err(e) => {
                    eprintln!("[collector] failed to read {}: {}", path.display(), e);
                    total_skipped += 1;
                }
            }
        }

        stats.sessions = sessions;
        stats.token_rows = -1;
        stats.skipped = total_skipped;
        Ok(stats)
    }
}

impl CodexCollector {
    fn collect_one_file(&self, db: &Db, device_id: &str, path: &Path) -> Result<FileStats> {
        let session_raw = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            // rollout-<ts>-<uuid> -> take the trailing uuid segment
            .rsplit('-')
            .next()
            .unwrap_or("unknown")
            .to_string();
        let session_pk = format!("{device_id}:codex:{session_raw}");

        let file = open_file_with_retry(path).map_err(|e| {
            anyhow::anyhow!("无法打开文件 {}：{}（文件可能被其他程序占用）", path.display(), e)
        })?;
        let reader = BufReader::new(file);

        let mut prompts = 0i64;
        let mut skipped = 0i64;
        let mut message_count = 0i64;
        let mut first_prompt: Option<String> = None;
        let mut start_ts: Option<u64> = None;
        let mut end_ts: Option<u64> = None;
        let mut project_path: Option<String> = None;
        let mut last_model = String::new();
        // Codex emits cumulative token_count events; keep the latest totals.
        let mut totals: Option<CodexUsage> = None;
        let now = now_secs();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: CodexLine = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            message_count += 1;
            if let Some(ts) = parsed.timestamp_secs() {
                start_ts = Some(start_ts.map_or(ts, |s| s.min(ts)));
                end_ts = Some(end_ts.map_or(ts, |e| e.max(ts)));
            }

            match parsed.kind.as_str() {
                "session_meta" => {
                    if project_path.is_none() {
                        if let Some(cwd) = jval_str(&parsed.payload, "cwd") {
                            if !cwd.is_empty() {
                                project_path = Some(cwd);
                            }
                        }
                    }
                }
                "turn_context" => {
                    if let Some(m) = jval_str(&parsed.payload, "model") {
                        if !m.is_empty() {
                            last_model = m;
                        }
                    }
                }
                "event_msg" => {
                    let sub = jval_str(&parsed.payload, "type").unwrap_or_default();
                    match sub.as_str() {
                        "user_message" => {
                            if let Some(msg) = jval_str(&parsed.payload, "message") {
                                if first_prompt.is_none() {
                                    first_prompt = Some(msg.chars().take(200).collect());
                                }
                                let prompt_id = format!(
                                    "{session_pk}:{}",
                                    parsed.timestamp.as_deref().unwrap_or("t")
                                );
                                db.upsert_collected_prompt(&CollectedPrompt {
                                    id: prompt_id,
                                    device_id: device_id.to_string(),
                                    session_id: session_pk.clone(),
                                    source: self.source().to_string(),
                                    project_id: None,
                                    prompt_text: Some(msg.chars().take(2000).collect()),
                                    started_at: parsed.timestamp_secs(),
                                    duration_ms: None,
                                    requested_action: None,
                                    target_object: None,
                                    interaction_state: None,
                                    interaction_mode: None,
                                    confidence: None,
                                    tool_calls: None,
                                    tool_errors: None,
                                })?;
                                prompts += 1;
                            }
                        }
                        "token_count" => {
                            if let Some(info) = parsed.payload.get("info") {
                                if let Some(tot) = info.get("total_token_usage") {
                                    if let Ok(u) =
                                        serde_json::from_value::<CodexUsage>(tot.clone())
                                    {
                                        totals = Some(u);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let project_id = match &project_path {
            Some(p) => db.ensure_project_by_path(device_id, p).unwrap_or(None),
            None => None,
        };

        let model_id = if last_model.is_empty() {
            "unknown".to_string()
        } else {
            last_model.clone()
        };

        db.upsert_collected_session(&CollectedSession {
            id: session_pk.clone(),
            device_id: device_id.to_string(),
            source: self.source().to_string(),
            project_id: project_id.clone(),
            agent_id: None,
            start_time: start_ts,
            end_time: end_ts,
            message_count,
            title_or_prompt: first_prompt,
            cached_at: now,
            project_path,
            quality_score: Some(quality_score(message_count, prompts)),
        })?;

        if let Some(t) = totals {
            db.upsert_collected_token_usage(&CollectedTokenUsage {
                id: format!("{session_pk}:{model_id}"),
                device_id: device_id.to_string(),
                session_id: session_pk.clone(),
                source: self.source().to_string(),
                project_id,
                model_id: Some(model_id.clone()),
                input_tokens: t.input_tokens,
                output_tokens: t.output_tokens,
                reasoning_tokens: t.reasoning_output_tokens,
                cache_creation_input_tokens: 0, // Codex has no cache-creation concept
                cache_read_input_tokens: t.cached_input_tokens,
                total_tokens: t.total_tokens,
                model_calls: 1,
                tool_calls: 0,
                duration_ms: None,
            })?;
        }

        Ok(FileStats {
            session_upserted: true,
            prompts,
            skipped,
        })
    }
}

// Codex rollout jsonl line deserialization. Envelope: {timestamp, type, payload}.
// The envelope `type` discriminates the payload shape (session_meta / turn_context /
// event_msg / response_item). For event_msg, the inner payload.type is the subtype
// (user_message / token_count / agent_message / ...).
#[derive(Debug, Deserialize)]
struct CodexLine {
    timestamp: Option<String>,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    payload: serde_json::Value,
}

impl CodexLine {
    fn timestamp_secs(&self) -> Option<u64> {
        chrono::DateTime::parse_from_rfc3339(self.timestamp.as_ref()?)
            .ok()
            .map(|dt| dt.timestamp().max(0) as u64)
    }
}

fn jval_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Codex token usage breakdown (from event_msg token_count.total_token_usage).
#[derive(Debug, Default, Clone, Deserialize)]
struct CodexUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    cached_input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    reasoning_output_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

/// Heuristic quality score 0..100 for a session (PRD-02 §3.2).
/// Combines conversational depth (message/prompt count) with diminishing returns.
pub(crate) fn quality_score(message_count: i64, prompts: i64) -> f64 {
    let depth = (prompts.max(0) as f64 * 3.0 + message_count.max(0) as f64).min(70.0);
    // Bonus for having real prompts (non-trivial session).
    let prompt_bonus = if prompts > 0 { 20.0 } else { 0.0 };
    // Small base so empty sessions aren't zero.
    let base = 10.0;
    (base + depth + prompt_bonus).min(100.0).round()
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// PRD-05 §6: open a SQLite database READ-ONLY via a URI (mode=ro), so the
/// collector never locks against a live agent process. `immutable` avoids WAL
/// contention at the cost of ignoring in-flight writes (acceptable for stats).
pub(crate) fn open_ro(uri: &str) -> Option<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

// =============================================================================
// PRD-05 §3.1: ZCode
// =============================================================================
//
// Reads `~/.zcode/cli/db/db.sqlite`. ZCode is the richest source: its `model_usage`
// table breaks tokens into input/output/reasoning/cache_creation/cache_read/computed_total
// and carries `query_source` (main_turn/subagent/compact/session_title/...) and `mode`
// (yolo/plan/edit/build) dimensions no other agent exposes. Schema verified 2026-06-26.
//
// Read path: session (directory/title/time) -> model_usage (tokens, aggregated per
// session+model) -> part (text blocks under user-role messages -> prompts).
//
// Hard constraint (PRD-05 §1.4): open the database READ-ONLY via a URI so we never
// lock against a live ZCode process. If the read-only open fails (active WAL), fall
// back to copying db+wal+shm to a tempdir and opening that.

/// Reads `~/.zcode/cli/db/db.sqlite` (tokens from `model_usage`, prompts from `part`).
pub struct ZCodeCollector {
    root: PathBuf,
}

impl Default for ZCodeCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zcode")
            .join("cli")
            .join("db")
            .join("db.sqlite");
        Self { root }
    }
}

impl ZCodeCollector {
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Open the ZCode SQLite read-only. Returns None if the open fails for any
    /// reason (missing file, locked, malformed); callers fall back or skip.
    fn open_readonly(&self) -> Option<rusqlite::Connection> {
        let uri = crate::db::sqlite_uri(&self.root, "?mode=ro&immutable=1");
        open_ro(&uri)
    }
}

impl Collector for ZCodeCollector {
    fn source(&self) -> &str {
        SOURCE_ZCODE
    }
    fn collector_kind(&self) -> &str {
        "sqlite"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_file()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats);
        }

        // Incremental: skip if the SQLite file itself has not changed since last collection.
        match is_file_changed(db, self.source(), &self.root) {
            Ok(false) => {
                stats.skipped = -1; // marker: skipped due to unchanged file
                return Ok(stats);
            }
            Ok(true) => {}
            Err(e) => eprintln!("[collector] failed to stat {}: {}", self.root.display(), e),
        }

        // PRD-05 §6: open read-only. If the live db is locked (active WAL), fall
        // back to a tempdir snapshot. We intentionally do NOT use a writable open.
        let conn = match self.open_readonly() {
            Some(c) => c,
            None => match self.open_snapshot() {
                Some(c) => c,
                None => return Ok(stats), // leave caller to mark source as unavailable
            },
        };

        let now = now_secs();
        let mut sessions = 0i64;
        let mut prompts = 0i64;
        let mut skipped = 0i64;

        // 1. Sessions: id, directory, title, time_created, time_updated.
        let session_rows = match conn.prepare(
            "SELECT id, directory, title, time_created, time_updated FROM session",
        ) {
            Ok(mut stmt) => stmt
                .query_map([], |row| {
                    Ok(ZcSession {
                        id: row.get(0)?,
                        directory: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        title: row.get::<_, Option<String>>(2)?,
                        time_created: row.get::<_, Option<i64>>(3)?,
                        time_updated: row.get::<_, Option<i64>>(4)?,
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                .unwrap_or_default(),
            Err(_) => {
                // Schema changed: table/columns missing. Mark incompatible downstream.
                return Ok(stats);
            }
        };

        for s in &session_rows {
            let session_pk = format!("{device_id}:zcode:{}", s.id);
            // Resolve project_id from the session's working directory.
            let project_id = if !s.directory.is_empty() {
                db.ensure_project_by_path(device_id, &s.directory)
                    .unwrap_or(None)
            } else {
                None
            };

            // 2. Token usage: aggregate model_usage per (session, model_id).
            //    PRD-05 §5.1: fold query_source into the row key so main_turn /
            //    subagent / compact counts stay distinct without a schema change.
            let token_rows: Vec<ZcTokenAgg> = conn
                .prepare(
                    "SELECT model_id, query_source,
                            SUM(input_tokens), SUM(output_tokens), SUM(reasoning_tokens),
                            SUM(cache_creation_input_tokens), SUM(cache_read_input_tokens),
                            SUM(computed_total_tokens), COUNT(*), SUM(tool_call_count), SUM(duration_ms)
                     FROM model_usage WHERE session_id = ?1
                     GROUP BY model_id, query_source",
                )
                .map_err(|e| anyhow::anyhow!("prepare model_usage: {e}"))
                .and_then(|mut stmt| {
                    stmt.query_map(params![s.id], |row| {
                        Ok(ZcTokenAgg {
                            model_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            query_source: row.get::<_, Option<String>>(1)?,
                            input: row.get(2)?,
                            output: row.get(3)?,
                            reasoning: row.get(4)?,
                            cache_creation: row.get(5)?,
                            cache_read: row.get(6)?,
                            total: row.get(7)?,
                            calls: row.get(8)?,
                            tool_calls: row.get(9)?,
                            duration_ms: row.get(10)?,
                        })
                    })
                    .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                    .map_err(|e| anyhow::anyhow!("query model_usage: {e}"))
                })
                .unwrap_or_default();

            // 3. Prompt count for this session (text parts under user-role messages).
            let prompt_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM part p
                     JOIN message m ON p.message_id = m.id
                     WHERE p.session_id = ?1
                       AND json_extract(p.data, '$.type') = 'text'
                       AND json_extract(m.data, '$.role') = 'user'",
                    params![s.id],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // First prompt text (for title/preview) — capped at 200 chars.
            let first_prompt: Option<String> = conn
                .query_row(
                    "SELECT json_extract(p.data, '$.text') FROM part p
                     JOIN message m ON p.message_id = m.id
                     WHERE p.session_id = ?1
                       AND json_extract(p.data, '$.type') = 'text'
                       AND json_extract(m.data, '$.role') = 'user'
                     ORDER BY p.time_created LIMIT 1",
                    params![s.id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten();

            // Upsert the session.
            db.upsert_collected_session(&CollectedSession {
                id: session_pk.clone(),
                device_id: device_id.to_string(),
                source: self.source().to_string(),
                project_id: project_id.clone(),
                agent_id: None,
                start_time: s.time_created.map(|t| (t / 1000).max(0) as u64),
                end_time: s.time_updated.map(|t| (t / 1000).max(0) as u64),
                message_count: prompt_count,
                title_or_prompt: s
                    .title
                    .clone()
                    .or_else(|| first_prompt.as_ref().map(|t| t.chars().take(200).collect())),
                cached_at: now,
                project_path: if s.directory.is_empty() {
                    None
                } else {
                    Some(s.directory.clone())
                },
                quality_score: Some(quality_score(prompt_count, prompt_count)),
            })?;
            sessions += 1;

            // Upsert per-(session, model, query_source) token rows.
            // PRD-05 §5.1: fold non-main_turn query_source into the model id so the
            // dimensions stay distinct without a schema change. main_turn keeps the
            // bare model name (the common case, keeps cross-agent sums clean).
            for agg in token_rows {
                let folded_model = match &agg.query_source {
                    Some(qs) if !qs.is_empty() && qs != "main_turn" => {
                        format!("{}@{}", agg.model_id, qs)
                    }
                    _ => agg.model_id.clone(),
                };
                db.upsert_collected_token_usage(&CollectedTokenUsage {
                    id: format!("{session_pk}:{folded_model}"),
                    device_id: device_id.to_string(),
                    session_id: session_pk.clone(),
                    source: self.source().to_string(),
                    project_id: project_id.clone(),
                    model_id: Some(folded_model.clone()),
                    input_tokens: agg.input,
                    output_tokens: agg.output,
                    reasoning_tokens: agg.reasoning,
                    cache_creation_input_tokens: agg.cache_creation,
                    cache_read_input_tokens: agg.cache_read,
                    total_tokens: agg.total,
                    model_calls: agg.calls,
                    tool_calls: agg.tool_calls,
                    duration_ms: Some(agg.duration_ms),
                })?;
            }

            // Upsert each prompt text (cap at 2000 chars).
            if prompt_count > 0 {
                let mut stmt = match conn.prepare(
                    "SELECT p.id, json_extract(p.data, '$.text'), (p.time_created/1000)
                     FROM part p
                     JOIN message m ON p.message_id = m.id
                     WHERE p.session_id = ?1
                       AND json_extract(p.data, '$.type') = 'text'
                       AND json_extract(m.data, '$.role') = 'user'
                     ORDER BY p.time_created",
                ) {
                    Ok(s) => s,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let rows: Vec<(String, Option<String>, Option<i64>)> = match stmt.query_map(
                    params![s.id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                        ))
                    },
                ) {
                    Ok(rs) => rs.filter_map(|r| r.ok()).collect(),
                    Err(_) => continue,
                };
                for (part_id, text, ts) in rows {
                    let Some(text) = text else { continue };
                    if text.trim().is_empty() {
                        continue;
                    }
                    db.upsert_collected_prompt(&CollectedPrompt {
                        id: format!("{session_pk}:{part_id}"),
                        device_id: device_id.to_string(),
                        session_id: session_pk.clone(),
                        source: self.source().to_string(),
                        project_id: project_id.clone(),
                        prompt_text: Some(text.chars().take(2000).collect()),
                        started_at: ts.map(|t| t.max(0) as u64),
                        duration_ms: None,
                        requested_action: None,
                        target_object: None,
                        interaction_state: None,
                        interaction_mode: None,
                        confidence: None,
                        tool_calls: None,
                        tool_errors: None,
                    })?;
                    prompts += 1;
                }
            }
        }

        stats.sessions = sessions;
        stats.prompts = prompts;
        stats.token_rows = -1; // folded into sessions; not individually counted
        stats.skipped = skipped;
        if let Err(e) = mark_file_collected(db, self.source(), &self.root) {
            eprintln!("[collector] failed to mark {}: {}", self.root.display(), e);
        }
        Ok(stats)
    }
}

impl ZCodeCollector {
    /// Fallback when the read-only open of the live db is blocked by an active WAL:
    /// copy db + wal + shm into a tempdir and open that read-only. Returns None if
    /// the copy or open fails (caller skips this source rather than aborting).
    fn open_snapshot(&self) -> Option<rusqlite::Connection> {
        let tmp = tempfile::tempdir().ok()?;
        let dest = tmp.path().join("zcode-snapshot.sqlite");
        std::fs::copy(&self.root, &dest).ok()?;
        // Best-effort: bring the WAL/SHM sidecars so the snapshot is current.
        let name = self
            .root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("db.sqlite");
        let parent = self.root.parent()?;
        for ext in ["wal", "shm"] {
            let src = parent.join(format!("{name}-{ext}"));
            if src.exists() {
                let _ = std::fs::copy(&src, dest.with_file_name(format!(
                    "{}-{ext}",
                    dest.file_name().and_then(|n| n.to_str()).unwrap_or("snapshot.sqlite")
                )));
            }
        }
        let uri = crate::db::sqlite_uri(&dest, "?mode=ro");
        open_ro(&uri)
    }
}

struct ZcSession {
    id: String,
    directory: String,
    title: Option<String>,
    time_created: Option<i64>,
    time_updated: Option<i64>,
}

struct ZcTokenAgg {
    model_id: String,
    query_source: Option<String>,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_creation: i64,
    cache_read: i64,
    total: i64,
    calls: i64,
    tool_calls: i64,
    duration_ms: i64,
}

// =============================================================================
// PRD-05 §3.2: Cursor
// =============================================================================
//
// Reads `~/.cursor/ai-tracking/ai-code-tracking.db`. Cursor has NO token data and NO
// prompt text locally — only AI-vs-human line attribution per git commit, plus per-file
// AI-code hashes. So this collector writes to the SEPARATE `collected_code_contributions`
// table and must NOT touch `collected_token_usage` (different semantics).
//
// Schema verified 2026-06-26: scored_commits (commitHash, branchName, commitDate,
// scoredAt, lines*, v2AiPercentage as string "100.00"), ai_code_hashes (model,
// conversationId). We do NOT parse state.vscdb composer JSON (private, version-fragile).

/// Reads `~/.cursor/ai-tracking/ai-code-tracking.db` (AI code contribution only).
pub struct CursorCollector {
    root: PathBuf,
}

impl Default for CursorCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cursor")
            .join("ai-tracking")
            .join("ai-code-tracking.db");
        Self { root }
    }
}

impl CursorCollector {
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn open_readonly(&self) -> Option<rusqlite::Connection> {
        let uri = crate::db::sqlite_uri(&self.root, "?mode=ro&immutable=1");
        open_ro(&uri)
    }
}

impl Collector for CursorCollector {
    fn source(&self) -> &str {
        SOURCE_CURSOR
    }
    fn collector_kind(&self) -> &str {
        "sqlite"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_file()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats);
        }

        // Incremental: skip if the SQLite file itself has not changed since last collection.
        match is_file_changed(db, self.source(), &self.root) {
            Ok(false) => {
                stats.skipped = -1;
                return Ok(stats);
            }
            Ok(true) => {}
            Err(e) => eprintln!("[collector] failed to stat {}: {}", self.root.display(), e),
        }

        let conn = match self.open_readonly() {
            Some(c) => c,
            None => return Ok(stats),
        };

        let now = now_secs();
        let mut commits = 0i64;
        let mut skipped = 0i64;

        // scored_commits; v2AiPercentage is a string like "100.00", fall back to v1.
        let mut stmt = match conn.prepare(
            "SELECT commitHash, branchName, commitDate, scoredAt,
                    linesAdded, linesDeleted, composerLinesAdded, composerLinesDeleted,
                    humanLinesAdded, humanLinesDeleted, tabLinesAdded, tabLinesDeleted,
                    v2AiPercentage, v1AiPercentage, commitMessage
             FROM scored_commits",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(stats), // table missing → schema incompatible
        };
        let rows = stmt.query_map([], |row| {
            Ok(CursorCommit {
                commit_hash: row.get::<_, String>(0)?,
                branch_name: row.get::<_, Option<String>>(1)?,
                commit_date: row.get::<_, Option<String>>(2)?,
                scored_at: row.get::<_, Option<i64>>(3)?,
                lines_added: row.get(4)?,
                lines_deleted: row.get(5)?,
                composer_added: row.get(6)?,
                composer_deleted: row.get(7)?,
                human_added: row.get(8)?,
                human_deleted: row.get(9)?,
                tab_added: row.get::<_, Option<i64>>(10)?,
                tab_deleted: row.get::<_, Option<i64>>(11)?,
                v2_pct: row.get::<_, Option<String>>(12)?,
                v1_pct: row.get::<_, Option<String>>(13)?,
                message: row.get::<_, Option<String>>(14)?,
            })
        });
        let rows = match rows {
            Ok(r) => r,
            Err(_) => return Ok(stats),
        };

        for row in rows {
            let c = match row {
                Ok(c) => c,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let ai_pct = c
                .v2_pct
                .as_deref()
                .or(c.v1_pct.as_deref())
                .and_then(parse_pct);
            let id = format!("{device_id}:cursor:{}", c.commit_hash);
            db.upsert_collected_code_contribution(&CollectedCodeContribution {
                id,
                device_id: device_id.to_string(),
                source: self.source().to_string(),
                project_id: None, // P0: scored_commits has no path (PRD-05 §3.2 待决)
                commit_hash: c.commit_hash,
                branch_name: c.branch_name,
                commit_date: c.commit_date,
                scored_at: c.scored_at.map(|t| (t / 1000).max(0) as u64),
                lines_added: c.lines_added,
                lines_deleted: c.lines_deleted,
                composer_lines_added: c.composer_added,
                composer_lines_deleted: c.composer_deleted,
                human_lines_added: c.human_added,
                human_lines_deleted: c.human_deleted,
                tab_lines_added: c.tab_added,
                ai_percentage: ai_pct,
                commit_message: c.message,
                cached_at: now,
            })?;
            commits += 1;
        }

        stats.sessions = commits; // repurpose: cursor's "rows" are commits, not sessions
        stats.skipped = skipped;
        if let Err(e) = mark_file_collected(db, self.source(), &self.root) {
            eprintln!("[collector] failed to mark {}: {}", self.root.display(), e);
        }
        Ok(stats)
    }
}

struct CursorCommit {
    commit_hash: String,
    branch_name: Option<String>,
    commit_date: Option<String>,
    scored_at: Option<i64>,
    lines_added: Option<i64>,
    lines_deleted: Option<i64>,
    composer_added: Option<i64>,
    composer_deleted: Option<i64>,
    human_added: Option<i64>,
    human_deleted: Option<i64>,
    tab_added: Option<i64>,
    tab_deleted: Option<i64>,
    v2_pct: Option<String>,
    v1_pct: Option<String>,
    message: Option<String>,
}

/// Parse a Cursor AI-percentage string ("100.00", "85.5%", "0.85") into a 0-100 float.
/// Returns None on any parse failure (caller leaves the field null).
fn parse_pct(s: &str) -> Option<f64> {
    let t = s.trim().trim_end_matches('%');
    let v: f64 = t.parse().ok()?;
    // If the value looks like a fraction (<=1.0 and the raw wasn't "100"), scale up.
    let scaled = if v <= 1.0 && !s.trim().contains('%') && t != "1" {
        v * 100.0
    } else {
        v
    };
    Some(scaled.clamp(0.0, 100.0))
}

// =============================================================================
// PRD-08 P1: OpenCode
// =============================================================================

/// Reads `~/.local/share/opencode/opencode.db` (SQLite). OpenCode persists
/// sessions, per-message tokens (input/output/reasoning + cache read/write), and
/// user prompt text — a full-fidelity source. Ported from AI-Digest's
/// `opencode.py`. Read-only, like the other collectors.
pub struct OpenCodeCollector {
    root: PathBuf,
}

impl Default for OpenCodeCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".local/share/opencode/opencode.db");
        Self { root }
    }
}

impl OpenCodeCollector {
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Open the OpenCode SQLite read-only. Returns None on any open failure.
    fn open_readonly(&self) -> Option<rusqlite::Connection> {
        let uri = crate::db::sqlite_uri(&self.root, "?mode=ro&immutable=1");
        open_ro(&uri)
    }
}

impl Collector for OpenCodeCollector {
    fn source(&self) -> &str {
        SOURCE_OPENCODE
    }
    fn collector_kind(&self) -> &str {
        "sqlite"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_file()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats);
        }

        // Incremental: skip if the SQLite file itself has not changed since last collection.
        match is_file_changed(db, self.source(), &self.root) {
            Ok(false) => {
                stats.skipped = -1;
                return Ok(stats);
            }
            Ok(true) => {}
            Err(e) => eprintln!("[collector] failed to stat {}: {}", self.root.display(), e),
        }

        let conn = match self.open_readonly() {
            Some(c) => c,
            None => return Ok(stats),
        };
        let now = now_secs();

        // Sessions in the last 30 days (OpenCode uses ms-epoch timestamps).
        // We pull a rolling window rather than a single day so re-collect
        // backfills; upserts are idempotent on the session PK.
        let cutoff_ms = (now.saturating_sub(30 * 86400) as i64) * 1000;
        let mut stmt = match conn.prepare(
            "SELECT id, title, directory, time_created, time_updated FROM session
             WHERE time_updated >= ?1
             ORDER BY time_created ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(stats), // schema mismatch → skip gracefully
        };
        let rows = match stmt.query_map(params![cutoff_ms], |row| {
            Ok(OpenCodeSession {
                id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                title: row.get::<_, Option<String>>(1)?,
                directory: row.get::<_, Option<String>>(2)?,
                time_created: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                time_updated: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        }) {
            Ok(r) => r,
            Err(_) => return Ok(stats),
        };
        let sessions: Vec<OpenCodeSession> = rows.filter_map(|r| r.ok()).collect();
        drop(stmt);

        for s in &sessions {
            let session_pk = format!("{device_id}:{}:{}", self.source(), s.id);
            let project_path = s.directory.as_ref().and_then(|d| {
                std::path::Path::new(d).file_name().map(|n| n.to_string_lossy().to_string())
            });
            let project_id = project_path
                .as_ref()
                .and_then(|p| db.ensure_project_by_path(device_id, p).unwrap_or(None));

            // First user prompt + message count (from the part/message tables).
            let (first_prompt, message_count) = opencode_first_prompt_and_count(&conn, &s.id);
            let title = first_prompt
                .clone()
                .or_else(|| s.title.clone())
                .unwrap_or_else(|| "OpenCode session".to_string());

            // Per-model token aggregation from message.data.tokens.
            let per_model = opencode_token_agg(&conn, &s.id);

            db.upsert_collected_session(&CollectedSession {
                id: session_pk.clone(),
                device_id: device_id.to_string(),
                source: self.source().to_string(),
                project_id,
                agent_id: None,
                start_time: Some((s.time_created / 1000).max(0) as u64),
                end_time: Some((s.time_updated / 1000).max(0) as u64),
                message_count,
                title_or_prompt: Some(title.chars().take(120).collect()),
                cached_at: now,
                project_path,
                quality_score: Some(quality_score(message_count, 0)),
            })?;
            stats.sessions += 1;

            for (model, acc) in per_model {
                db.upsert_collected_token_usage(&CollectedTokenUsage {
                    id: format!("{session_pk}:{model}"),
                    device_id: device_id.to_string(),
                    session_id: session_pk.clone(),
                    source: self.source().to_string(),
                    project_id: None,
                    model_id: Some(model),
                    input_tokens: acc.input,
                    output_tokens: acc.output,
                    reasoning_tokens: acc.reasoning,
                    cache_creation_input_tokens: acc.cache_write,
                    cache_read_input_tokens: acc.cache_read,
                    total_tokens: acc.input + acc.output + acc.reasoning + acc.cache_read + acc.cache_write,
                    model_calls: acc.calls,
                    tool_calls: 0,
                    duration_ms: None,
                })?;
                stats.token_rows += 1;
            }

            // User prompts (for the collaboration dimension + classification).
            for (text, started_ms) in opencode_user_prompts(&conn, &s.id) {
                let prompt_id = format!("{session_pk}:{}", short_hash(&text));
                db.upsert_collected_prompt(&CollectedPrompt {
                    id: prompt_id,
                    device_id: device_id.to_string(),
                    session_id: session_pk.clone(),
                    source: self.source().to_string(),
                    project_id: None,
                    prompt_text: Some(text.chars().take(300).collect()),
                    started_at: Some((started_ms / 1000).max(0) as u64),
                    duration_ms: None,
                    requested_action: None,
                    target_object: None,
                    interaction_state: None,
                    interaction_mode: None,
                    confidence: None,
                    tool_calls: None,
                    tool_errors: None,
                })?;
                stats.prompts += 1;
            }
        }

        if let Err(e) = mark_file_collected(db, self.source(), &self.root) {
            eprintln!("[collector] failed to mark {}: {}", self.root.display(), e);
        }
        Ok(stats)
    }
}

struct OpenCodeSession {
    id: String,
    title: Option<String>,
    directory: Option<String>,
    time_created: i64,
    time_updated: i64,
}

#[derive(Default)]
struct TokenAcc {
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    calls: i64,
}

/// Extract the first user prompt text and the user+assistant message count.
fn opencode_first_prompt_and_count(conn: &rusqlite::Connection, session_id: &str) -> (Option<String>, i64) {
    let mut first: Option<String> = None;
    let mut count = 0i64;
    let sql = "SELECT json_extract(m.data,'$.role') AS role,
                      json_extract(p.data,'$.text') AS text
               FROM part p JOIN message m ON m.id = p.message_id
               WHERE p.session_id = ?1
                 AND json_extract(p.data,'$.type')='text'
                 AND json_extract(m.data,'$.role') IN ('user','assistant')
               ORDER BY p.time_created ASC";
    if let Ok(mut stmt) = conn.prepare(sql) {
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        });
        if let Ok(rows) = rows {
            for r in rows.flatten() {
                let (role, text) = r;
                if let Some(t) = text {
                    let t = t.trim();
                    if t.is_empty() {
                        continue;
                    }
                    count += 1;
                    if first.is_none() && role.as_deref() == Some("user") {
                        first = Some(t.to_string());
                    }
                }
            }
        }
    }
    (first, count)
}

/// Aggregate per-model token usage from message.data.tokens (JSON).
fn opencode_token_agg(conn: &rusqlite::Connection, session_id: &str) -> HashMap<String, TokenAcc> {
    let mut agg: HashMap<String, TokenAcc> = HashMap::new();
    let sql = "SELECT data FROM message WHERE session_id = ?1 AND json_extract(data,'$.tokens') IS NOT NULL";
    if let Ok(mut stmt) = conn.prepare(sql) {
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0));
        if let Ok(rows) = rows {
            for r in rows.flatten() {
                let data: serde_json::Value = match serde_json::from_str(&r) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let model = data
                    .get("modelID")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let tokens = match data.get("tokens") {
                    Some(t) => t,
                    None => continue,
                };
                let cache = tokens.get("cache");
                let acc = agg.entry(model).or_default();
                acc.input += tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
                acc.output += tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                acc.reasoning += tokens.get("reasoning").and_then(|v| v.as_i64()).unwrap_or(0);
                acc.cache_read += cache.and_then(|c| c.get("read")).and_then(|v| v.as_i64()).unwrap_or(0);
                acc.cache_write += cache.and_then(|c| c.get("write")).and_then(|v| v.as_i64()).unwrap_or(0);
                acc.calls += 1;
            }
        }
    }
    agg
}

/// Collect (text, time_created_ms) for user prompts in a session.
fn opencode_user_prompts(conn: &rusqlite::Connection, session_id: &str) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    let sql = "SELECT p.time_created, json_extract(p.data,'$.text') AS text
               FROM part p JOIN message m ON m.id = p.message_id
               WHERE p.session_id = ?1
                 AND json_extract(p.data,'$.type')='text'
                 AND json_extract(m.data,'$.role')='user'
               ORDER BY p.time_created ASC";
    if let Ok(mut stmt) = conn.prepare(sql) {
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get::<_, Option<i64>>(0)?.unwrap_or(0), row.get::<_, Option<String>>(1)?))
        });
        if let Ok(rows) = rows {
            for r in rows.flatten() {
                if let Some(t) = r.1 {
                    let t = t.trim().to_string();
                    if !t.is_empty() {
                        out.push((t, r.0));
                    }
                }
            }
        }
    }
    out
}

/// Stable short hash for a dedup-friendly prompt id component.
/// Uses SHA-256 (already in Cargo.toml) and truncates to 16 hex chars so the
/// id stays short while remaining deterministic across Rust versions/platforms.
fn short_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}


// =============================================================================
// Kimi Code CLI
// =============================================================================

/// Reads `~/.kimi-code/session_index.jsonl` and the per-session
/// `sessions/<wd>/<session>/agents/main/wire.jsonl` files.
pub struct KimiCodeCollector {
    root: PathBuf,
}

impl Default for KimiCodeCollector {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".kimi-code");
        Self { root }
    }
}

impl KimiCodeCollector {
    /// Construct with a custom root (used by tests).
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Collector for KimiCodeCollector {
    fn source(&self) -> &str {
        SOURCE_KIMI_CODE
    }
    fn collector_kind(&self) -> &str {
        "kimi_wire_jsonl"
    }
    fn data_path(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.root.is_dir() && self.root.join("session_index.jsonl").is_file()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats {
            source: self.source().to_string(),
            ..Default::default()
        };
        if !self.is_available() {
            return Ok(stats);
        }

        let index_path = self.root.join("session_index.jsonl");
        let file = open_file_with_retry(&index_path).map_err(|e| {
            anyhow::anyhow!("无法打开文件 {}：{}（文件可能被其他程序占用）", index_path.display(), e)
        })?;
        let reader = BufReader::new(file);
        let mut sessions = 0i64;
        let mut total_skipped = 0i64;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    total_skipped += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let entry: SessionIndexEntry = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    total_skipped += 1;
                    continue;
                }
            };
            let wire_path = PathBuf::from(&entry.session_dir)
                .join("agents")
                .join("main")
                .join("wire.jsonl");
            if !wire_path.is_file() {
                total_skipped += 1;
                continue;
            }

            // Incremental: skip sessions whose wire file has not changed.
            match is_file_changed(db, self.source(), &wire_path) {
                Ok(false) => {
                    total_skipped += 1;
                    continue;
                }
                Ok(true) => {}
                Err(e) => {
                    eprintln!("[collector] failed to stat {}: {}", wire_path.display(), e);
                    total_skipped += 1;
                    continue;
                }
            }

            match self.collect_one_session(db, device_id, &entry, &wire_path) {
                Ok(FileStats {
                    session_upserted,
                    prompts,
                    skipped,
                }) => {
                    if session_upserted {
                        sessions += 1;
                    }
                    stats.prompts += prompts;
                    total_skipped += skipped;
                    if let Err(e) = mark_file_collected(db, self.source(), &wire_path) {
                        eprintln!("[collector] failed to mark {}: {}", wire_path.display(), e);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[collector] failed to read {}: {}",
                        wire_path.display(),
                        e
                    );
                    total_skipped += 1;
                }
            }
        }

        stats.sessions = sessions;
        stats.token_rows = -1;
        stats.skipped = total_skipped;
        Ok(stats)
    }
}

impl KimiCodeCollector {
    fn collect_one_session(
        &self,
        db: &Db,
        device_id: &str,
        entry: &SessionIndexEntry,
        wire_path: &Path,
    ) -> Result<FileStats> {
        let session_pk = format!("{device_id}:kimi-code:{}", entry.session_id);
        let file = open_file_with_retry(wire_path).map_err(|e| {
            anyhow::anyhow!("无法打开文件 {}：{}（文件可能被其他程序占用）", wire_path.display(), e)
        })?;
        let reader = BufReader::new(file);

        let mut prompts = 0i64;
        let mut skipped = 0i64;
        let mut message_count = 0i64;
        let mut first_prompt: Option<String> = None;
        let mut start_ts: Option<u64> = None;
        let mut end_ts: Option<u64> = None;
        let mut tool_calls = 0i64;
        let mut per_model: HashMap<String, ModelAccum> = HashMap::new();
        let now = now_secs();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: KimiWireLine = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            message_count += 1;
            if let Some(ts) = parsed.timestamp() {
                start_ts = Some(start_ts.map_or(ts, |s| s.min(ts)));
                end_ts = Some(end_ts.map_or(ts, |e| e.max(ts)));
            }

            match parsed.line_type.as_str() {
                "metadata" => {
                    if let Some(ts) = parsed.created_at {
                        start_ts = Some(start_ts.map_or(ts, |s| s.min(ts)));
                    }
                }
                "turn.prompt" => {
                    if let Some(text) = parsed.user_prompt_text() {
                        if first_prompt.is_none() {
                            first_prompt = Some(text.chars().take(200).collect());
                        }
                        let prompt_id = format!("{session_pk}:turn:{}", parsed.time.unwrap_or(0));
                        db.upsert_collected_prompt(&CollectedPrompt {
                            id: prompt_id,
                            device_id: device_id.to_string(),
                            session_id: session_pk.clone(),
                            source: self.source().to_string(),
                            project_id: None,
                            prompt_text: Some(text.chars().take(2000).collect()),
                            started_at: parsed.timestamp(),
                            duration_ms: None,
                            requested_action: None,
                            target_object: None,
                            interaction_state: None,
                            interaction_mode: None,
                            confidence: None,
                            tool_calls: None,
                            tool_errors: None,
                        })?;
                        prompts += 1;
                    }
                }
                "tool.call" => {
                    tool_calls += 1;
                }
                "usage.record" => {
                    // Only aggregate per-turn usage; session-scope rows are summaries.
                    if parsed.usage_scope.as_deref() != Some("turn") {
                        continue;
                    }
                    if let Some(usage) = parsed.usage {
                        if usage.input_other > 0
                            || usage.output > 0
                            || usage.input_cache_read > 0
                            || usage.input_cache_creation > 0
                        {
                            let model = parsed
                                .model
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string());
                            let acc = per_model.entry(model.clone()).or_default();
                            acc.input += usage.input_other;
                            acc.output += usage.output;
                            acc.cache_read += usage.input_cache_read;
                            acc.cache_creation += usage.input_cache_creation;
                            acc.calls += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        let project_id = db
            .ensure_project_by_path(device_id, &entry.work_dir)
            .unwrap_or(None);

        db.upsert_collected_session(&CollectedSession {
            id: session_pk.clone(),
            device_id: device_id.to_string(),
            source: self.source().to_string(),
            project_id: project_id.clone(),
            agent_id: None,
            start_time: start_ts,
            end_time: end_ts,
            message_count,
            title_or_prompt: first_prompt,
            cached_at: now,
            project_path: Some(entry.work_dir.clone()),
            quality_score: Some(quality_score(message_count, prompts)),
        })?;

        for (model, acc) in per_model {
            db.upsert_collected_token_usage(&CollectedTokenUsage {
                id: format!("{session_pk}:{}", model),
                device_id: device_id.to_string(),
                session_id: session_pk.clone(),
                source: self.source().to_string(),
                project_id: project_id.clone(),
                model_id: Some(model),
                input_tokens: acc.input,
                output_tokens: acc.output,
                reasoning_tokens: 0,
                cache_creation_input_tokens: acc.cache_creation,
                cache_read_input_tokens: acc.cache_read,
                total_tokens: acc.input + acc.output,
                model_calls: acc.calls,
                tool_calls,
                duration_ms: None,
            })?;
        }

        Ok(FileStats {
            session_upserted: true,
            prompts,
            skipped,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SessionIndexEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "sessionDir")]
    session_dir: String,
    #[serde(rename = "workDir")]
    work_dir: String,
}

#[derive(Debug, Deserialize)]
struct KimiWireLine {
    #[serde(default, rename = "type")]
    line_type: String,
    time: Option<u64>,
    #[serde(rename = "createdAt")]
    created_at: Option<u64>,
    model: Option<String>,
    #[serde(rename = "usageScope")]
    usage_scope: Option<String>,
    usage: Option<KimiUsage>,
    input: Option<Vec<KimiInputPart>>,
}

impl KimiWireLine {
    fn timestamp(&self) -> Option<u64> {
        self.time
    }

    fn user_prompt_text(&self) -> Option<String> {
        if self.line_type != "turn.prompt" {
            return None;
        }
        let input = self.input.as_ref()?;
        let mut texts = Vec::new();
        for part in input {
            if part.part_type == "text" {
                if let Some(t) = &part.text {
                    texts.push(t.as_str());
                }
            }
        }
        let joined = texts.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }
}

#[derive(Debug, Deserialize)]
struct KimiUsage {
    #[serde(rename = "inputOther", default)]
    input_other: i64,
    #[serde(default)]
    output: i64,
    #[serde(rename = "inputCacheRead", default)]
    input_cache_read: i64,
    #[serde(rename = "inputCacheCreation", default)]
    input_cache_creation: i64,
}

#[derive(Debug, Deserialize)]
struct KimiInputPart {
    #[serde(default, rename = "type")]
    part_type: String,
    text: Option<String>,
}

/// Remove `collector_file_states` rows for `source` whose files no longer exist.
/// This keeps the incremental state accurate when a user deletes or rotates
/// agent session files. The collected_* session rows are intentionally left
/// untouched: they remain as historical usage data.
pub fn prune_missing_file_states(db: &Db, source: &str) -> Result<usize> {
    let states = db.list_collected_file_states(source)?;
    let mut removed = 0usize;
    for state in states {
        if !std::path::Path::new(&state.file_path).exists() {
            db.delete_collected_file_state(source, &state.file_path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
