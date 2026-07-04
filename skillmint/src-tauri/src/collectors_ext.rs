//! PRD-08 P2: the remaining session-only collectors — CodeBuddy, Antigravity,
//! Gemini CLI, and the three Trae variants (international / CN / Solo).
//!
//! All six are **session-only**: their local formats do not persist token counts
//! or prompt text (the capability flags in `collector.rs` reflect this), so each
//! collector emits `collected_sessions` rows only — contributing to session
//! counts and the role-profile denominator but not to token sums.
//!
//! Ported from AI-Digest's `codebuddy.py`, `antigravity.py`, `gemini_cli.py`,
//! and `trae.py`. Read-only, like every collector.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;
use walkdir::WalkDir;

use crate::collector::{
    is_file_changed, mark_file_collected, open_ro, quality_score, now_secs, Collector,
    SOURCE_ANTIGRAVITY, SOURCE_CODEBUDDY, SOURCE_GEMINI_CLI, SOURCE_TRAE, SOURCE_TRAE_CN,
    SOURCE_TRAE_SOLO,
};
use crate::db::Db;
use crate::models::{CollectionStats, CollectedSession};

/// A normalized session pulled from one of these formats. Built by each
/// collector, then handed to [`insert_session`] for the upsert.
struct RawSession {
    id: String,
    start_secs: i64,
    end_secs: i64,
    project: Option<String>,
    title: Option<String>,
    message_count: i64,
}

/// Upsert one raw session into `collected_sessions` (session-only — no tokens,
/// no prompts). Returns nothing; the caller bumps its stat counters.
fn insert_session(db: &Db, device_id: &str, source: &str, raw: &RawSession) -> Result<()> {
    let pk = format!("{device_id}:{source}:{}", raw.id);
    let project_id = raw
        .project
        .as_ref()
        .and_then(|p| db.ensure_project_by_path(device_id, p).unwrap_or(None));
    db.upsert_collected_session(&CollectedSession {
        id: pk,
        device_id: device_id.to_string(),
        source: source.to_string(),
        project_id,
        agent_id: None,
        start_time: Some(raw.start_secs.max(0) as u64),
        end_time: Some(raw.end_secs.max(0) as u64),
        message_count: raw.message_count,
        title_or_prompt: raw.title.clone(),
        cached_at: now_secs(),
        project_path: raw.project.clone(),
        quality_score: Some(quality_score(raw.message_count, 0)),
    })?;
    Ok(())
}

/// Only keep sessions whose start is within the last 30 days (rolling window;
/// upserts are idempotent). Mirrors the OpenCode collector's approach.
fn in_window(start_secs: i64) -> bool {
    let cutoff = (now_secs().saturating_sub(30 * 86400)) as i64;
    start_secs >= cutoff
}

// Helper: parse an ISO-8601 timestamp (with optional Z) to epoch seconds.
fn parse_iso_to_secs(s: &str) -> Option<i64> {
    use chrono::{DateTime, NaiveDateTime};
    let s = s.trim().trim_end_matches('Z');
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(naive.and_utc().timestamp());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(naive.and_utc().timestamp());
    }
    None
}

/// Coerce a JSON timestamp value (ISO string, epoch-secs, or epoch-ms) to secs.
fn coerce_ts(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        // Heuristic: > 1e12 → milliseconds.
        return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
    }
    if let Some(s) = v.as_str() {
        return parse_iso_to_secs(s);
    }
    None
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

// =============================================================================
// CodeBuddy — CLI JSONL (`~/.codebuddy/projects/<enc>/*.jsonl`) + IDE state.vscdb
// =============================================================================

pub struct CodeBuddyCollector {
    cli_dir: PathBuf,
    ide_dir: PathBuf,
}

impl Default for CodeBuddyCollector {
    fn default() -> Self {
        Self {
            cli_dir: home().join(".codebuddy/projects"),
            ide_dir: home().join("Library/Application Support/CodeBuddy/User"),
        }
    }
}

impl Collector for CodeBuddyCollector {
    fn source(&self) -> &str {
        SOURCE_CODEBUDDY
    }
    fn collector_kind(&self) -> &str {
        "jsonl+vscdb"
    }
    fn data_path(&self) -> String {
        self.cli_dir.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.cli_dir.is_dir() || self.ide_dir.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats { source: self.source().to_string(), ..Default::default() };
        // CLI JSONL.
        if self.cli_dir.is_dir() {
            for entry in WalkDir::new(&self.cli_dir).max_depth(2).into_iter().flatten() {
                let p = entry.path();
                if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.contains("subagent") || name.contains("tool-result") {
                    continue;
                }
                match is_file_changed(db, self.source(), p) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(_) => continue,
                }
                if let Some(raw) = parse_codebuddy_cli(p) {
                    if in_window(raw.start_secs) {
                        insert_session(db, device_id, self.source(), &raw)?;
                        stats.sessions += 1;
                    }
                }
                let _ = mark_file_collected(db, self.source(), p);
            }
        }
        // IDE state.vscdb.
        let ws_dir = self.ide_dir.join("workspaceStorage");
        if ws_dir.is_dir() {
            for ws in std::fs::read_dir(&ws_dir).into_iter().flatten().flatten() {
                let ws_path = ws.path();
                let db_path = ws_path.join("state.vscdb");
                if db_path.is_file() {
                    match is_file_changed(db, self.source(), &db_path) {
                        Ok(false) => continue,
                        Ok(true) => {}
                        Err(_) => continue,
                    }
                    let project = resolve_workspace_project(&ws_path);
                    for raw in parse_codebuddy_ide(&db_path, project.as_deref()) {
                        if in_window(raw.start_secs) {
                            insert_session(db, device_id, self.source(), &raw)?;
                            stats.sessions += 1;
                        }
                    }
                    let _ = mark_file_collected(db, self.source(), &db_path);
                }
            }
        }
        Ok(stats)
    }
}

/// Parse a CodeBuddy CLI `.jsonl` session (Claude-Code-like layout).
fn parse_codebuddy_cli(path: &Path) -> Option<RawSession> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let project = path
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| codebuddy_project_name(&n.to_string_lossy()));
    let file = std::fs::File::open(path).ok()?;
    let mut timestamps: Vec<i64> = Vec::new();
    let mut first_prompt: Option<String> = None;
    let mut msg_count = 0i64;
    for line in std::io::BufReader::new(file).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Timestamp: top-level, then snapshot, then message.
        for key in ["timestamp", "cacheBreaker"] {
            if let Some(ts) = obj.get(key).and_then(coerce_ts) {
                timestamps.push(ts);
            }
        }
        if let Some(snap) = obj.get("snapshot") {
            if let Some(ts) = snap.get("timestamp").and_then(coerce_ts) {
                timestamps.push(ts);
            }
        }
        if let Some(msg) = obj.get("message") {
            for key in ["timestamp", "cacheBreaker"] {
                if let Some(ts) = msg.get(key).and_then(coerce_ts) {
                    timestamps.push(ts);
                }
            }
        }
        let mtype = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if mtype == "human" || mtype == "assistant" {
            msg_count += 1;
            if mtype == "human" && first_prompt.is_none() {
                if let Some(content) = extract_message_content(obj.get("message")) {
                    first_prompt = Some(content);
                }
            }
        }
    }
    if timestamps.is_empty() {
        return None;
    }
    let start_secs = *timestamps.iter().min().unwrap();
    let end_secs = *timestamps.iter().max().unwrap();
    Some(RawSession {
        id,
        start_secs,
        end_secs,
        project,
        title: first_prompt.map(|s| s.chars().take(120).collect()),
        message_count: msg_count,
    })
}

fn codebuddy_project_name(dir_name: &str) -> String {
    let parts: Vec<&str> = dir_name.trim_matches('-').split('-').collect();
    let skip = ["Users", "home", "code", "projects"];
    let meaningful: Vec<&str> = parts.into_iter().filter(|p| !p.is_empty() && !skip.contains(p)).collect();
    meaningful.last().map(|s| s.to_string()).unwrap_or_else(|| dir_name.to_string())
}

fn extract_message_content(message: Option<&Value>) -> Option<String> {
    let msg = message?;
    let content = msg.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.trim().to_string());
    }
    if let Some(arr) = content.as_array() {
        let texts: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                    item.get("text").and_then(|v| v.as_str()).map(String::from)
                } else {
                    item.as_str().map(String::from)
                }
            })
            .collect();
        return Some(texts.join(" ").trim().to_string());
    }
    None
}

/// Parse CodeBuddy IDE conversations from `state.vscdb`'s chat memento.
fn parse_codebuddy_ide(db_path: &Path, project: Option<&str>) -> Vec<RawSession> {
    let mut out = Vec::new();
    let uri = crate::db::sqlite_uri(db_path, "?mode=ro&immutable=1");
    let conn = match open_ro(&uri) {
        Some(c) => c,
        None => return out,
    };
    let value: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            params!["memento/webviewView.coding-copilot.webviews.chat"],
            |row| row.get(0),
        )
        .ok();
    let value = match value {
        Some(v) => v,
        None => return out,
    };
    let text = String::from_utf8_lossy(&value);
    let data: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let conversations: Option<Value> = data.get("conversations").cloned().or_else(|| {
        // Fall back to alternate keys.
        for key in ["chatSessions", "sessions", "history"] {
            if let Some(c) = data.get(key) {
                return Some(c.clone());
            }
        }
        None
    });
    let list: Vec<Value> = match conversations {
        Some(Value::Array(a)) => a,
        Some(Value::Object(o)) => o.into_iter().map(|(_, v)| v).collect(),
        _ => return out,
    };
    for conv in list {
        let obj = match conv.as_object() {
            Some(o) => o,
            None => continue,
        };
        let id = obj
            .get("id")
            .or_else(|| obj.get("sessionId"))
            .or_else(|| obj.get("uuid"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let start = obj
            .get("createdAt")
            .or_else(|| obj.get("created_at"))
            .or_else(|| obj.get("timestamp"))
            .and_then(coerce_ts);
        let start_secs = match start {
            Some(s) => s,
            None => continue,
        };
        let end_secs = obj
            .get("updatedAt")
            .or_else(|| obj.get("updated_at"))
            .or_else(|| obj.get("lastUpdatedAt"))
            .and_then(coerce_ts)
            .unwrap_or(start_secs);
        let title = obj
            .get("title")
            .or_else(|| obj.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(120).collect::<String>());
        let msg_count = obj
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter(|m| {
                        let role = m.get("role").or_else(|| m.get("type")).and_then(|v| v.as_str()).unwrap_or("");
                        matches!(role.to_lowercase().as_str(), "user" | "human" | "assistant" | "ai")
                    })
                    .count() as i64
            })
            .unwrap_or(0);
        out.push(RawSession {
            id: format!("codebuddy-ide-{id}"),
            start_secs,
            end_secs,
            project: project.map(String::from),
            title,
            message_count: msg_count,
        });
    }
    out
}

/// Read `workspace.json` → folder name for project attribution.
fn resolve_workspace_project(ws_path: &Path) -> Option<String> {
    let ws_json = ws_path.join("workspace.json");
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&ws_json).ok()?).ok()?;
    let folder = data.get("folder")?.as_str()?;
    let cleaned = folder.strip_prefix("file://").unwrap_or(folder);
    Path::new(cleaned)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

// =============================================================================
// Antigravity — `~/.gemini/antigravity/brain/*/.metadata.json` + `.md` (+ .pb)
// =============================================================================

pub struct AntigravityCollector {
    brain_dir: PathBuf,
    conv_dir: PathBuf,
}

impl Default for AntigravityCollector {
    fn default() -> Self {
        let base = home().join(".gemini/antigravity");
        Self {
            brain_dir: base.join("brain"),
            conv_dir: base.join("conversations"),
        }
    }
}

impl Collector for AntigravityCollector {
    fn source(&self) -> &str {
        SOURCE_ANTIGRAVITY
    }
    fn collector_kind(&self) -> &str {
        "artifacts"
    }
    fn data_path(&self) -> String {
        self.brain_dir.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.brain_dir.is_dir() || self.conv_dir.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats { source: self.source().to_string(), ..Default::default() };
        // brain/<session>/*.
        if self.brain_dir.is_dir() {
            for entry in std::fs::read_dir(&self.brain_dir).into_iter().flatten().flatten() {
                let session_dir = entry.path();
                if !session_dir.is_dir() {
                    continue;
                }
                // Treat each session directory as a single incremental unit.
                match is_file_changed(db, self.source(), &session_dir) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(_) => continue,
                }
                if let Some(raw) = parse_antigravity_brain(&session_dir) {
                    if in_window(raw.start_secs) {
                        insert_session(db, device_id, self.source(), &raw)?;
                        stats.sessions += 1;
                    }
                }
                let _ = mark_file_collected(db, self.source(), &session_dir);
            }
        }
        // conversations/*.pb (fallback by mtime/birthtime).
        if self.conv_dir.is_dir() {
            for entry in std::fs::read_dir(&self.conv_dir).into_iter().flatten().flatten() {
                let pb = entry.path();
                if pb.extension().and_then(|e| e.to_str()) != Some("pb") {
                    continue;
                }
                let id = pb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                match is_file_changed(db, self.source(), &pb) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(_) => continue,
                }
                let meta = match std::fs::metadata(&pb) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
                let ctime = meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
                let (start_secs, end_secs) = match (ctime, mtime) {
                    (Some(c), Some(m)) => (c, m),
                    _ => continue,
                };
                if in_window(start_secs) {
                    insert_session(
                        db,
                        device_id,
                        self.source(),
                        &RawSession { id, start_secs, end_secs, project: None, title: Some("Antigravity session".into()), message_count: 0 },
                    )?;
                    stats.sessions += 1;
                }
                let _ = mark_file_collected(db, self.source(), &pb);
            }
        }
        Ok(stats)
    }
}

fn parse_antigravity_brain(session_dir: &Path) -> Option<RawSession> {
    let id = session_dir.file_name()?.to_string_lossy().to_string();
    let mut timestamps: Vec<i64> = Vec::new();
    let mut title = String::new();
    let mut summaries: Vec<String> = Vec::new();
    let mut md_count = 0i64;

    // *.metadata.json
    if let Ok(entries) = std::fs::read_dir(session_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".metadata.json") {
                if let Ok(txt) = std::fs::read_to_string(&p) {
                    if let Ok(meta) = serde_json::from_str::<Value>(&txt) {
                        for key in ["createdAt", "updatedAt", "lastModified"] {
                            if let Some(ts) = meta.get(key).and_then(coerce_ts) {
                                timestamps.push(ts);
                            }
                        }
                        if let Some(s) = meta.get("summary").and_then(|v| v.as_str()) {
                            if !s.is_empty() {
                                summaries.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(best) = summaries.into_iter().max_by_key(|s| s.len()) {
        title = best.chars().take(120).collect();
    }
    // *.md (non-metadata)
    if let Ok(entries) = std::fs::read_dir(session_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".metadata.json") {
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            md_count += 1;
            if let Ok(meta) = std::fs::metadata(&p) {
                if let Ok(mtime) = meta.modified() {
                    if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                        timestamps.push(d.as_secs() as i64);
                    }
                }
            }
            if title.is_empty() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    for line in content.lines().take(5) {
                        let line = line.trim();
                        if let Some(h) = line.strip_prefix("# ") {
                            title = h.trim().chars().take(120).collect();
                            break;
                        }
                    }
                }
            }
        }
    }
    if timestamps.is_empty() {
        if let Ok(meta) = std::fs::metadata(session_dir) {
            if let Ok(mtime) = meta.modified() {
                if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    timestamps.push(d.as_secs() as i64);
                }
            }
        }
    }
    let start_secs = *timestamps.iter().min()?;
    let end_secs = *timestamps.iter().max().unwrap_or(&start_secs);
    Some(RawSession {
        id,
        start_secs,
        end_secs,
        project: None,
        title: if title.is_empty() { Some("Antigravity session".into()) } else { Some(title) },
        message_count: md_count,
    })
}

// =============================================================================
// Gemini CLI — `~/.gemini/history/*` (directory mtime only)
// =============================================================================

pub struct GeminiCliCollector {
    base_dir: PathBuf,
}

impl Default for GeminiCliCollector {
    fn default() -> Self {
        Self { base_dir: home().join(".gemini/history") }
    }
}

impl Collector for GeminiCliCollector {
    fn source(&self) -> &str {
        SOURCE_GEMINI_CLI
    }
    fn collector_kind(&self) -> &str {
        "mtime"
    }
    fn data_path(&self) -> String {
        self.base_dir.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.base_dir.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats { source: self.source().to_string(), ..Default::default() };
        if !self.base_dir.is_dir() {
            return Ok(stats);
        }
        for entry in std::fs::read_dir(&self.base_dir)?.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            match is_file_changed(db, self.source(), &project_dir) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(_) => continue,
            }
            let project = project_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            // Use the dir's mtime; fall back to the newest child mtime.
            let mut best: Option<i64> = dir_mtime_secs(&project_dir);
            if best.filter(|&t| in_window(t)).is_none() {
                best = None;
                if let Ok(children) = std::fs::read_dir(&project_dir) {
                    for child in children.flatten() {
                        if let Some(ts) = dir_mtime_secs(&child.path()) {
                            if in_window(ts) {
                                best = Some(ts);
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(ts) = best.filter(|&t| in_window(t)) {
                let id = format!("gemini-{project}-{}", ts);
                insert_session(
                    db,
                    device_id,
                    self.source(),
                    &RawSession {
                        id,
                        start_secs: ts,
                        end_secs: ts,
                        project: Some(project.clone()),
                        title: Some(format!("Gemini CLI session in {project}")),
                        message_count: 0,
                    },
                )?;
                stats.sessions += 1;
            }
            let _ = mark_file_collected(db, self.source(), &project_dir);
        }
        Ok(stats)
    }
}

fn dir_mtime_secs(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let d = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(d.as_secs() as i64)
}

// =============================================================================
// Trae (international / CN / Solo) — VSCode-fork state.vscdb mementos
// =============================================================================

pub struct TraeCollector {
    variant: TraeVariant,
    base_dir: PathBuf,
}

#[derive(Clone, Copy)]
enum TraeVariant {
    International,
    Cn,
    Solo,
}

impl TraeCollector {
    fn new(variant: TraeVariant) -> Self {
        let base_dir = match variant {
            TraeVariant::International => home().join("Library/Application Support/Trae/User"),
            TraeVariant::Cn => home().join("Library/Application Support/Trae CN/User"),
            TraeVariant::Solo => home().join("Library/Application Support/TRAE SOLO/User"),
        };
        Self { variant, base_dir }
    }

    fn source_key(&self) -> &'static str {
        match self.variant {
            TraeVariant::International => SOURCE_TRAE,
            TraeVariant::Cn => SOURCE_TRAE_CN,
            TraeVariant::Solo => SOURCE_TRAE_SOLO,
        }
    }
}

impl Collector for TraeCollector {
    fn source(&self) -> &str {
        self.source_key()
    }
    fn collector_kind(&self) -> &str {
        "vscdb"
    }
    fn data_path(&self) -> String {
        self.base_dir.to_string_lossy().to_string()
    }
    fn is_available(&self) -> bool {
        self.base_dir.is_dir()
    }

    fn collect(&self, db: &Db, device_id: &str) -> Result<CollectionStats> {
        let mut stats = CollectionStats { source: self.source().to_string(), ..Default::default() };
        // Treat the whole base_dir as the incremental unit for Trae.
        match is_file_changed(db, self.source(), &self.base_dir) {
            Ok(false) => return Ok(stats),
            Ok(true) => {}
            Err(_) => {}
        }
        let raws = match self.variant {
            TraeVariant::Solo => collect_trae_solo(&self.base_dir),
            _ => collect_trae_vsc(&self.base_dir),
        };
        for raw in raws {
            if in_window(raw.start_secs) {
                insert_session(db, device_id, self.source(), &raw)?;
                stats.sessions += 1;
            }
        }
        let _ = mark_file_collected(db, self.source(), &self.base_dir);
        Ok(stats)
    }
}

/// International / CN: read `memento/icube-ai-agent-storage*` from each
/// workspace's `state.vscdb`.
fn collect_trae_vsc(base_dir: &Path) -> Vec<RawSession> {
    let mut out = Vec::new();
    let ws_dir = base_dir.join("workspaceStorage");
    if !ws_dir.is_dir() {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(&ws_dir) else { return out };
    for ws in entries.flatten() {
        let ws_path = ws.path();
        let db_path = ws_path.join("state.vscdb");
        if !db_path.is_file() {
            continue;
        }
        let project = resolve_workspace_project(&ws_path);
        let uri = crate::db::sqlite_uri(&db_path, "?mode=ro&immutable=1");
        let conn = match open_ro(&uri) {
            Some(c) => c,
            None => continue,
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT value FROM ItemTable WHERE key LIKE 'memento/icube-ai-agent-storage%'",
        ) else {
            continue;
        };
        let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)).ok();
        if let Some(rows) = rows {
            for r in rows.flatten() {
                let text = String::from_utf8_lossy(&r);
                let data: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let Some(list) = data.get("list").and_then(|v| v.as_array()) else { continue };
                for sess in list {
                    let Some(obj) = sess.as_object() else { continue };
                    let id = obj.get("sessionId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let Some(created) = obj.get("createdAt").and_then(coerce_ts) else { continue };
                    let updated = obj.get("updatedAt").and_then(coerce_ts).unwrap_or(created);
                    let title = obj
                        .get("title")
                        .or_else(|| obj.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.chars().take(120).collect::<String>());
                    let msg_count = obj
                        .get("messages")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter(|m| {
                                    matches!(m.get("role").and_then(|v| v.as_str()), Some("user") | Some("assistant"))
                                })
                                .count() as i64
                        })
                        .unwrap_or(0);
                    out.push(RawSession {
                        id: format!("trae-{id}"),
                        start_secs: created,
                        end_secs: updated,
                        project: project.clone(),
                        title,
                        message_count: msg_count,
                    });
                }
            }
        }
    }
    out
}

/// Solo: read `solo-lite:content-map*` + `solo-lite-mode-state-map*` from the
/// global `state.vscdb`. Session ids are MongoDB ObjectIds (first 8 hex = ts).
fn collect_trae_solo(base_dir: &Path) -> Vec<RawSession> {
    let mut out = Vec::new();
    let db_path = base_dir.join("globalStorage/state.vscdb");
    if !db_path.is_file() {
        return out;
    }
    let uri = crate::db::sqlite_uri(&db_path, "?mode=ro&immutable=1");
    let conn = match open_ro(&uri) {
        Some(c) => c,
        None => return out,
    };
    // content-map → { sessionId: { uri, ... } }
    let value: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key LIKE 'solo-lite:content-map%'",
            [],
            |row| row.get(0),
        )
        .ok();
    let content_map: Value = match value {
        Some(v) => serde_json::from_str(&String::from_utf8_lossy(&v)).unwrap_or(Value::Null),
        None => return out,
    };
    let Some(map) = content_map.as_object() else { return out };
    for (sid, info) in map {
        // First 8 hex chars of the ObjectId = Unix timestamp.
        let created = match i64::from_str_radix(sid.get(..8).unwrap_or(""), 16) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let title = info
            .get("uri")
            .and_then(|v| v.as_str())
            .map(|u| {
                let decoded = urlencoding_decode(u);
                Path::new(&decoded)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Trae Solo session".to_string())
            })
            .unwrap_or_else(|| "Trae Solo session".to_string());
        out.push(RawSession {
            id: format!("trae-solo-{sid}"),
            start_secs: created,
            end_secs: created,
            project: None,
            title: Some(title.chars().take(120).collect()),
            message_count: 0,
        });
    }
    out
}

/// Minimal URL-decode for the `file://`-prefixed URIs Trae Solo stores.
fn urlencoding_decode(s: &str) -> String {
    let stripped = s.strip_prefix("file://").unwrap_or(s);
    let mut out = String::with_capacity(stripped.len());
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(b as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Public constructors for `discover_all_collectors` to wire up the 6 collectors.
pub fn all_session_only_collectors() -> Vec<Box<dyn Collector>> {
    vec![
        Box::new(CodeBuddyCollector::default()),
        Box::new(AntigravityCollector::default()),
        Box::new(GeminiCliCollector::default()),
        Box::new(TraeCollector::new(TraeVariant::International)),
        Box::new(TraeCollector::new(TraeVariant::Cn)),
        Box::new(TraeCollector::new(TraeVariant::Solo)),
    ]
}
