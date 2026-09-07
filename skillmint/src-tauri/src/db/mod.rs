use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use std::str::FromStr;
use uuid::Uuid;

use crate::models::{
    Agent, AgentDirectory, AgentSkillItem, AgentUsageSummary, CollectedCodeContribution,
    CollectedFileState, CollectedPrompt, CollectedSession, CollectedSource, CollectedTokenUsage,
    CollectionJob, HighValuePrompt, IntervalUnit, KgEdge, KgGraph, KgNode, ProjectAgentEntry,
    DataDictionary, ProjectUsageSummary, RawDataExport, RunStatus, ScheduledTask, ScheduleStrategy,
    SkillBundle, SkillBundleItem, SkillProjectBinding, SkillStatus, SkillUsageAttribution,
    SkillUsageSummary, Skill, Source, SourceType, SyncMode, SyncStatus, SyncTarget, TableInfo, TaskKind,
    TaskRun, TriggerSource,
};

mod agents;
mod analytics;
mod bundles;
mod collection;
mod data_management;
mod discovery;
pub mod index_store;
mod kg;
mod llm;
mod projects;
mod reports;
mod scheduler;
mod skills;
mod sources;
mod sync_targets;
mod trash;
mod usage;

/// PRD-08 §3.4 (P3): one prompt row backing a heatmap cell drill-down.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CellPrompt {
    pub prompt_text: String,
    pub started_at: Option<i64>,
    pub requested_action: Option<String>,
    pub target_object: Option<String>,
    pub interaction_state: Option<String>,
    pub confidence: Option<f64>,
}

/// PRD-08 §3.8 (P2): row-count preview of an AI-Digest digest.db, for the
/// import UI.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct DigestDbStats {
    pub sessions: i64,
    pub prompts: i64,
    pub token_usage: i64,
}

/// PRD-08 §3.8 (P2): outcome of an import run.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct DigestImportSummary {
    pub sessions: i64,
    pub prompts: i64,
    pub token_rows: i64,
}

/// PRD-08 §3.8 (P2): in-memory representation of an AI-Digest digest.db so
/// the heavy read of the external database can happen WITHOUT holding the
/// local DB mutex, and the local write can happen inside a single transaction.
#[derive(Debug, Default)]
pub struct DigestData {
    pub sessions: Vec<DigestSessionRow>,
    pub token_usage: Vec<DigestTokenRow>,
    pub prompts: Vec<DigestPromptRow>,
}

#[derive(Debug)]
pub struct DigestSessionRow {
    pub id: String,
    pub source: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub project_path: Option<String>,
    pub title: Option<String>,
    pub message_count: Option<i64>,
}

#[derive(Debug)]
pub struct DigestTokenRow {
    pub session_id: String,
    pub source: String,
    pub project_path: Option<String>,
    pub model_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub total_tokens: i64,
    pub model_calls: i64,
    pub tool_calls: i64,
    pub duration_ms: Option<i64>,
}

#[derive(Debug)]
pub struct DigestPromptRow {
    pub turn_id: String,
    pub session_id: String,
    pub source: String,
    pub project_path: Option<String>,
    pub prompt_text: Option<String>,
    pub started_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub requested_action: Option<String>,
    pub target_object: Option<String>,
    pub interaction_state: Option<String>,
    pub interaction_mode: Option<String>,
}

/// PRD-08 §3.8: parse an AI-Digest ISO timestamp to epoch seconds.
/// Handles:
///   - RFC 3339 / ISO 8601 with offset and fractional seconds
///     (e.g. "2026-06-25T12:08:58.993000+08:00")
///   - "YYYY-MM-DD HH:MM:SS" and "YYYY-MM-DDTHH:MM:SS"
///   - date-only "YYYY-MM-DD"
/// Returns None on any parse failure.
fn iso_to_secs(s: &str) -> Option<i64> {
    use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
    let s = s.trim();
    // AI-Digest stores timestamps with offset + fractional seconds.
    if let Ok(dt) = DateTime::<FixedOffset>::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%:z") {
        return Some(dt.timestamp());
    }
    if let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(naive.and_utc().timestamp());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(naive.and_utc().timestamp());
    }
    // Date only → midnight (00:00:00) of that day.
    if let Ok(day) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }
    None
}

/// PRD-08 §3.8: map AI-Digest's human-readable `source_name` values to the
/// slugged collector keys SkillMint uses (`claude-code`, `zcode`, ...). This is
/// critical for analysis: without it, imported rows keep "Claude Code" while
/// locally-collected rows are "claude-code", and the Role Profile / per-source
/// distribution would split each tool into two buckets. Mapping mirrors
/// AI-Digest's `PLATFORM_MAP`. Unknown names pass through unchanged.
fn normalize_digest_source(raw: &str) -> String {
    match raw.trim() {
        "Claude Code" => "claude-code",
        "Codex" => "codex",
        "ZCode" => "zcode",
        "Cursor" => "cursor",
        "OpenCode" => "opencode",
        "CodeBuddy" => "codebuddy",
        "Antigravity" => "antigravity",
        "Gemini CLI" => "gemini-cli",
        "Trae" => "trae",
        "Trae CN" => "trae-cn",
        "Trae Solo" => "trae-solo",
        "Kimi Code" => "kimi-code",
        other => return other.to_lowercase().replace(' ', "-"),
    }
    .to_string()
}

/// Build a SQLite URI from a local path, percent-encoding characters that are
/// invalid in a URI (spaces, non-ASCII, etc.) while keeping `/` path separators
/// intact. Without this, `~/.digest/digest.db` under a username containing
/// spaces or CJK characters fails to open.
pub(crate) fn sqlite_uri(path: &std::path::Path, query: &str) -> String {
    use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};
    const PATH_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'/');
    let encoded = percent_encode(
        path.to_str().unwrap_or_default().as_bytes(),
        PATH_SET,
    );
    format!("file:{encoded}{query}")
}


pub struct Db {
    conn: Connection,
    /// PRD-06 §6: on-disk path of the database, retained so the entity-refactor
    /// migration can take a physical backup before any structural change.
    path: PathBuf,
}

/// PRD-06 §3.1: one row of the directory-grain → tool-grain merge map.
/// `canonical_id` is the surviving Agent; every alias id is merged into it with
/// its `skill_directory` recorded as an `agent_directories` row of the given role.
struct MergeTarget {
    canonical_id: &'static str,
    source: &'static str,
    aliases: &'static [(&'static str, &'static str)], // (alias agent id, role)
}

impl Db {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn,
            path: path.clone(),
        })
    }

    /// P2-2: expose the on-disk database path for data-management UI.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// PRD-06: test/diagnostic accessor for the underlying connection. Used by
    /// migration acceptance tests to set up legacy rows and assert post-conditions
    /// on columns not exposed by a dedicated helper.
    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn init(&mut self, device_id: &str) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                repo_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                skill_directory TEXT NOT NULL,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                discovery_rule TEXT
            );

            -- PRD-06 §5.2: agent_directories (1:N sub-records). An Agent owns multiple
            -- directories; the directory is a sub-record, not the entity.
            CREATE TABLE IF NOT EXISTS agent_directories (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                role TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                UNIQUE(agent_id, path)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_directories_agent ON agent_directories(agent_id);

            -- PRD-08 (P1): cached per-directory skill scan results so the UI
            -- does not re-read the filesystem on every page open.
            CREATE TABLE IF NOT EXISTS agent_directory_skills (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                exists_in_center INTEGER NOT NULL DEFAULT 0,
                content_match INTEGER,
                agent_hash TEXT,
                center_hash TEXT,
                scanned_at INTEGER NOT NULL,
                PRIMARY KEY (agent_id, path, name)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_directory_skills_lookup
                ON agent_directory_skills(agent_id, path);

            CREATE TABLE IF NOT EXISTS sync_targets (
                id TEXT PRIMARY KEY,
                skill_id TEXT NOT NULL REFERENCES skills(id),
                agent_id TEXT NOT NULL REFERENCES agents(id),
                mode TEXT NOT NULL CHECK(mode IN ('symlink', 'copy')),
                last_sync_at INTEGER,
                status TEXT NOT NULL CHECK(status IN ('synced','local_changed','center_changed','conflict','broken')),
                UNIQUE(skill_id, agent_id)
            );

            -- PRD-01: project-level skill management scaffolding (empty, not wired to UI yet).
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                first_seen_at INTEGER,
                last_active_at INTEGER,
                agent_sources TEXT,
                is_stale INTEGER DEFAULT 0,
                created_at INTEGER,
                updated_at INTEGER,
                UNIQUE(device_id, path)
            );
            CREATE INDEX IF NOT EXISTS idx_projects_device ON projects(device_id);

            CREATE TABLE IF NOT EXISTS agent_instances (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                agent_id TEXT NOT NULL REFERENCES agents(id),
                project_id TEXT NOT NULL REFERENCES projects(id),
                last_session_at INTEGER,
                session_count INTEGER DEFAULT 0,
                total_tokens INTEGER DEFAULT 0,
                total_prompts INTEGER DEFAULT 0,
                created_at INTEGER,
                updated_at INTEGER,
                UNIQUE(device_id, agent_id, project_id)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_instances_device ON agent_instances(device_id);

            CREATE TABLE IF NOT EXISTS skill_project_bindings (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                skill_id TEXT NOT NULL REFERENCES skills(id),
                project_id TEXT REFERENCES projects(id),
                agent_id TEXT REFERENCES agents(id),
                mode TEXT NOT NULL CHECK(mode IN ('local_copy', 'symlink', 'reference')),
                local_path TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER,
                updated_at INTEGER,
                UNIQUE(device_id, skill_id, project_id, agent_id)
            );
            CREATE INDEX IF NOT EXISTS idx_skill_project_bindings_device ON skill_project_bindings(device_id);

            -- PRD-02: usage data collection (normalized, source-agnostic).
            CREATE TABLE IF NOT EXISTS collected_sources (
                source TEXT PRIMARY KEY,
                collector_kind TEXT NOT NULL,
                data_path TEXT,
                status TEXT NOT NULL,
                last_collected_at INTEGER,
                record_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS collected_sessions (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                source TEXT NOT NULL,
                project_id TEXT,
                agent_id TEXT,
                start_time INTEGER,
                end_time INTEGER,
                message_count INTEGER,
                title_or_prompt TEXT,
                cached_at INTEGER,
                project_path TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cs_device ON collected_sessions(device_id);

            CREATE TABLE IF NOT EXISTS collected_prompts (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                project_id TEXT,
                prompt_text TEXT,
                started_at INTEGER,
                duration_ms INTEGER,
                requested_action TEXT,
                target_object TEXT,
                interaction_state TEXT,
                interaction_mode TEXT,
                confidence REAL,
                tool_calls INTEGER,
                tool_errors INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_cp_device ON collected_prompts(device_id);

            CREATE TABLE IF NOT EXISTS collected_token_usage (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                project_id TEXT,
                model_id TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                reasoning_tokens INTEGER,
                cache_creation_input_tokens INTEGER,
                cache_read_input_tokens INTEGER,
                total_tokens INTEGER,
                model_calls INTEGER,
                tool_calls INTEGER,
                duration_ms INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_ctu_device ON collected_token_usage(device_id);

            -- PRD-05 §5.2: Cursor AI code contribution (per scored git commit).
            -- Separate from collected_token_usage: Cursor has no token data, only
            -- AI-vs-human line attribution, so folding it in would pollute sums.
            CREATE TABLE IF NOT EXISTS collected_code_contributions (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                source TEXT NOT NULL,
                project_id TEXT,
                commit_hash TEXT NOT NULL,
                branch_name TEXT,
                commit_date TEXT,
                scored_at INTEGER,
                lines_added INTEGER,
                lines_deleted INTEGER,
                composer_lines_added INTEGER,
                composer_lines_deleted INTEGER,
                human_lines_added INTEGER,
                human_lines_deleted INTEGER,
                tab_lines_added INTEGER,
                ai_percentage REAL,
                commit_message TEXT,
                cached_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_ccc_device ON collected_code_contributions(device_id);
            CREATE INDEX IF NOT EXISTS idx_ccc_scored ON collected_code_contributions(scored_at);

            -- PRD-02 §5.2: skill usage attribution.
            CREATE TABLE IF NOT EXISTS skill_usage_attributions (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                skill_id TEXT,
                skill_name TEXT NOT NULL,
                source TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project_id TEXT,
                agent_id TEXT,
                attribution_type TEXT NOT NULL,
                attributed_at INTEGER,
                UNIQUE(device_id, skill_name, session_id)
            );
            CREATE INDEX IF NOT EXISTS idx_sua_device ON skill_usage_attributions(device_id);

            -- Incremental collection: per-file state so collectors can skip unchanged files.
            CREATE TABLE IF NOT EXISTS collector_file_states (
                source TEXT NOT NULL,
                file_path TEXT NOT NULL,
                last_modified_ns INTEGER NOT NULL DEFAULT 0,
                last_size INTEGER NOT NULL DEFAULT 0,
                last_collected_at INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT,
                PRIMARY KEY (source, file_path)
            );
            CREATE INDEX IF NOT EXISTS idx_cfs_source ON collector_file_states(source);

            -- Background collection jobs for async manual collection.
            CREATE TABLE IF NOT EXISTS collection_jobs (
                id TEXT PRIMARY KEY,
                started_at INTEGER NOT NULL,
                completed_at INTEGER,
                status TEXT NOT NULL CHECK(status IN ('pending','running','completed','failed','cancelled')),
                progress_json TEXT NOT NULL DEFAULT '{}',
                result_json TEXT,
                error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cj_status ON collection_jobs(status);
            CREATE INDEX IF NOT EXISTS idx_cj_started ON collection_jobs(started_at);

            -- P0: audit log for LLM/ACP routing so users can verify data locality.
            CREATE TABLE IF NOT EXISTS llm_request_logs (
                id TEXT PRIMARY KEY,
                requested_at INTEGER NOT NULL,
                provider TEXT NOT NULL,
                fallback INTEGER NOT NULL DEFAULT 0,
                has_raw_text INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_llm_logs_time ON llm_request_logs(requested_at DESC);

            -- PRD-03 §5: knowledge graph (concept network over SKILL.md).
            CREATE TABLE IF NOT EXISTS kg_nodes (
                id TEXT PRIMARY KEY,            -- hash(label+type), globally stable for dedup
                label TEXT NOT NULL,
                type TEXT NOT NULL CHECK(type IN ('concept','scenario','practice','action','object')),
                source TEXT,
                description TEXT,
                embedding BLOB,
                created_at INTEGER,
                updated_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS kg_edges (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                source_id TEXT NOT NULL REFERENCES kg_nodes(id),
                target_id TEXT NOT NULL REFERENCES kg_nodes(id),
                relation TEXT NOT NULL,         -- similar/depends_on/composes/conflicts_with/generalizes
                weight REAL,
                reason TEXT,
                is_manual INTEGER DEFAULT 0,
                is_rejected INTEGER DEFAULT 0,
                created_at INTEGER,
                updated_at INTEGER,
                UNIQUE(device_id, source_id, target_id, relation)
            );
            CREATE INDEX IF NOT EXISTS idx_kge_device ON kg_edges(device_id);

            CREATE TABLE IF NOT EXISTS kg_skill_nodes (
                device_id TEXT NOT NULL,
                skill_id TEXT NOT NULL REFERENCES skills(id),
                node_id TEXT NOT NULL REFERENCES kg_nodes(id),
                relevance REAL,
                created_at INTEGER,
                PRIMARY KEY (device_id, skill_id, node_id)
            );
            CREATE INDEX IF NOT EXISTS idx_kgsn_device ON kg_skill_nodes(device_id);

            -- PRD-07: remote sources (connected Git/GitHub repos / local dirs).
            CREATE TABLE IF NOT EXISTS sources (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_type TEXT NOT NULL,        -- github | git | local
                url TEXT NOT NULL,
                ref_spec TEXT NOT NULL DEFAULT 'main',
                subpath TEXT NOT NULL DEFAULT '',
                cache_path TEXT NOT NULL,
                commit_sha TEXT NOT NULL DEFAULT '',
                added_at INTEGER NOT NULL,
                last_fetched_at INTEGER,
                pull_policy TEXT NOT NULL DEFAULT 'manual',
                remote_revision TEXT NOT NULL DEFAULT ''
            );

            -- PRD-08: analysis engine dimension tables. `digest_model` holds the
            -- editable reference pricing (CNY per 1M tokens) so pricing is data,
            -- not code. `digest_tool` holds per-source capability flags so the
            -- engine can tell which sources contribute tokens vs prompts vs
            -- trusted turn duration. Both are seeded once on first migration.
            CREATE TABLE IF NOT EXISTS digest_model (
                model_id TEXT PRIMARY KEY,        -- normalized id, matched case-insensitively
                display_name TEXT,
                input_price REAL NOT NULL,        -- CNY / 1M tokens
                cache_price REAL NOT NULL DEFAULT 0,
                output_price REAL NOT NULL,       -- CNY / 1M tokens (incl. reasoning)
                billing_mode TEXT NOT NULL CHECK(billing_mode IN ('pay_as_you_go','subscription')),
                source TEXT NOT NULL DEFAULT 'builtin',  -- builtin | custom
                updated_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS digest_tool (
                source TEXT PRIMARY KEY,          -- == Agent.source
                has_token INTEGER NOT NULL,
                has_prompt INTEGER NOT NULL,
                has_tool_calls INTEGER NOT NULL,
                has_turn_duration INTEGER NOT NULL,
                billing_mode TEXT NOT NULL
            );

            -- PRD-08 §3.6: cached LLM daily summary (one row per date).
            CREATE TABLE IF NOT EXISTS digest_summary (
                date TEXT PRIMARY KEY,            -- YYYY-MM-DD
                highlights TEXT NOT NULL,
                activities TEXT NOT NULL,         -- JSON array
                model TEXT,
                provider TEXT,
                created_at INTEGER NOT NULL
            );

            -- PRD-08 §3.6c (P2): analysis result cache. window_metrics is somewhat
            -- expensive (3 windows × token+prompt metrics); this memoizes it. A row
            -- is valid while `data_through` >= the newest collected_* row it covers,
            -- so any fresh collection invalidates by bumping the window.
            CREATE TABLE IF NOT EXISTS analysis_window_cache (
                cache_key TEXT PRIMARY KEY,       -- hash(kind, ref, source)
                payload TEXT NOT NULL,            -- serialized WindowMetrics
                computed_at INTEGER NOT NULL,
                data_through INTEGER              -- newest cached_at covered, for invalidation
            );

            -- SPEC-I2: discovery inbox + weekly reports.
            CREATE TABLE IF NOT EXISTS discoveries (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK(kind IN ('repeat_pattern','high_value_prompt','skill_feedback','capability_gap')),
                title TEXT NOT NULL,
                payload TEXT NOT NULL DEFAULT '{}',
                confidence REAL NOT NULL,
                dedup_key TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending','accepted','dismissed','expired')),
                created_at INTEGER NOT NULL,
                decided_at INTEGER,
                resulting_skill_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_discoveries_status ON discoveries(status);
            CREATE INDEX IF NOT EXISTS idx_discoveries_dedup ON discoveries(dedup_key);
            CREATE INDEX IF NOT EXISTS idx_discoveries_created ON discoveries(created_at DESC);

            CREATE TABLE IF NOT EXISTS weekly_reports (
                week_start TEXT PRIMARY KEY,      -- YYYY-MM-DD (user-local Monday)
                content TEXT NOT NULL DEFAULT '{}',
                generated_at INTEGER NOT NULL,
                model TEXT,
                provider TEXT
            );

            -- PRD-10: scheduled tasks and execution history.
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                task_kind TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 0,
                strategy_kind TEXT NOT NULL CHECK(strategy_kind IN ('manual','interval','cron')),
                strategy_value INTEGER,
                strategy_unit TEXT CHECK(strategy_unit IN ('minutes','hours','days')),
                strategy_expression TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_run_at INTEGER,
                last_status TEXT CHECK(last_status IN ('pending','running','success','failed','skipped')),
                next_run_at INTEGER,
                run_count INTEGER NOT NULL DEFAULT 0,
                error_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_next_run ON scheduled_tasks(next_run_at);

            CREATE TABLE IF NOT EXISTS task_runs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
                status TEXT NOT NULL CHECK(status IN ('pending','running','success','failed','skipped')),
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                duration_ms INTEGER,
                result_summary TEXT NOT NULL DEFAULT '',
                error_message TEXT,
                triggered_by TEXT NOT NULL CHECK(triggered_by IN ('schedule','manual','startup'))
            );
            CREATE INDEX IF NOT EXISTS idx_task_runs_task_id ON task_runs(task_id);
            CREATE INDEX IF NOT EXISTS idx_task_runs_started_at ON task_runs(started_at);

            -- SPEC-F3: audit log for remote skill installations with safety findings.
            CREATE TABLE IF NOT EXISTS install_audit (
                id TEXT PRIMARY KEY,
                skill_name TEXT NOT NULL,
                source TEXT NOT NULL,
                findings_json TEXT NOT NULL DEFAULT '[]',
                confirmed_risks_json TEXT NOT NULL DEFAULT '[]',
                confirmed INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_install_audit_skill ON install_audit(skill_name);
            CREATE INDEX IF NOT EXISTS idx_install_audit_created ON install_audit(created_at DESC);

            -- PRD-02 Phase 2: skill bundles (named collections of skills).
            CREATE TABLE IF NOT EXISTS skill_bundles (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(device_id, name)
            );
            CREATE INDEX IF NOT EXISTS idx_skill_bundles_device ON skill_bundles(device_id);

            CREATE TABLE IF NOT EXISTS skill_bundle_items (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                bundle_id TEXT NOT NULL REFERENCES skill_bundles(id) ON DELETE CASCADE,
                skill_id TEXT NOT NULL REFERENCES skills(id),
                sort_order INTEGER NOT NULL DEFAULT 0,
                added_at INTEGER NOT NULL,
                UNIQUE(device_id, bundle_id, skill_id)
            );
            CREATE INDEX IF NOT EXISTS idx_skill_bundle_items_bundle ON skill_bundle_items(bundle_id);

            -- SPEC-C1: adoption ledger for the three-way discovery decision.
            -- Every decision is recorded atomically with the discoveries status
            -- update so the inbox can be replayed and the cooling tier computed.
            CREATE TABLE IF NOT EXISTS adoption_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                discovery_id TEXT NOT NULL,
                decision TEXT NOT NULL CHECK(decision IN ('adopted_as_is','adopted_edited','rejected')),
                reject_reason TEXT CHECK(reject_reason IN ('wrong','trivial','duplicate')),
                resulting_skill_id TEXT,
                decided_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_adoption_events_discovery ON adoption_events(discovery_id);
            CREATE INDEX IF NOT EXISTS idx_adoption_events_decided ON adoption_events(decided_at DESC);

            -- SPEC-C1: gate rejection bookkeeping. Low-confidence / daily-limit /
            -- cooling / rule-gate candidates are recorded here so the inbox can
            -- surface the silent "low_confidence" band (PRD-12 §3.1).
            CREATE TABLE IF NOT EXISTS gate_rejections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                reason TEXT NOT NULL CHECK(reason IN ('below_threshold','daily_limit','cooling','rule_gate')),
                kind TEXT NOT NULL,
                confidence REAL NOT NULL,
                payload TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_gate_rejections_reason ON gate_rejections(reason);
            CREATE INDEX IF NOT EXISTS idx_gate_rejections_created ON gate_rejections(created_at DESC);

            -- SPEC-C3: recycle bin ledger. A trashed skill is snapshotted to
            -- disk before the live rows are removed; this row points at the
            -- snapshot and carries enough metadata to rebuild the skill on restore.
            CREATE TABLE IF NOT EXISTS trash_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_type TEXT NOT NULL CHECK(item_type IN ('skill')),
                original_id TEXT NOT NULL,
                original_name TEXT NOT NULL,
                snapshot_path TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                deleted_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_trash_items_expires ON trash_items(expires_at);
            CREATE INDEX IF NOT EXISTS idx_trash_items_deleted ON trash_items(deleted_at DESC);

            -- P3-3: rebuildable skill index — the app's read model over on-disk
            -- truth (npx locks + agent dirs + private hubs). Pure cache: safe to
            -- drop and rebuild at any time. `project_key` = "-" for the global
            -- scope, else the project root; status is computed in
            -- `index_store::replace_skill_index` by diffing content hashes
            -- against the previous scan.
            CREATE TABLE IF NOT EXISTS skill_index (
                project_key TEXT NOT NULL,
                scope TEXT NOT NULL,
                name TEXT NOT NULL,
                managed_by TEXT NOT NULL,
                path TEXT NOT NULL,
                skill_md_path TEXT,
                source TEXT,
                source_url TEXT,
                source_type TEXT,
                ref_spec TEXT,
                hash TEXT,
                content_hash TEXT,
                prev_content_hash TEXT,
                status TEXT NOT NULL DEFAULT 'ok',
                agents_json TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (project_key, scope, name, path)
            );
            CREATE INDEX IF NOT EXISTS idx_skill_index_scope ON skill_index(project_key, scope);

            -- P3-3: refresh bookkeeping — the auto tick rebuilds only when this
            -- fingerprint (lock/canonical/hub mtimes) actually changed.
            CREATE TABLE IF NOT EXISTS skill_index_meta (
                project_key TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                rebuilt_at INTEGER NOT NULL
            );
            "#,
        )?;

        // Migrate: add `description` column to agents if it doesn't exist.
        let has_description: bool = self.conn.prepare("PRAGMA table_info(agents)")?.query_map([], |row| {
            let name: String = row.get(1)?;
            Ok(name)
        })?.any(|n| n.as_deref() == Ok("description"));
        if !has_description {
            let _ = self.conn.execute("ALTER TABLE agents ADD COLUMN description TEXT", []);
        }

        // PRD-01: add `device_id` column to existing tables, then backfill with the local device id.
        // Idempotent: only touches rows whose device_id is still NULL/empty.
        for table in ["skills", "agents", "sync_targets"] {
            self.ensure_device_id_column(table)?;
            self.conn.execute(
                &format!("UPDATE {table} SET device_id = ?1 WHERE device_id IS NULL OR device_id = ''"),
                params![device_id],
            )?;
        }

        // PRD-02 migration: add `project_path` to collected_sessions if the table was
        // created by an older build (CREATE TABLE IF NOT EXISTS won't add new columns).
        self.ensure_column("collected_sessions", "project_path", "TEXT")?;

        // E2-S2.1.4 Prompt 净化：打标用户输入 vs 系统/Skill 注入（保留原文可回溯；
        // 老数据为 NULL，检测器按规则兜底过滤）。见 prompt_kind.rs。
        self.ensure_column("collected_prompts", "prompt_kind", "TEXT")?;
        self.retag_prompt_kinds()?;

        // PRD-01: extend agents with usage-derived columns.
        self.ensure_column("agents", "last_used_at", "INTEGER")?;
        self.ensure_column("agents", "project_count", "INTEGER DEFAULT 0")?;
        self.ensure_column("agents", "created_at", "INTEGER")?;
        self.ensure_column("agents", "updated_at", "INTEGER")?;
        // PRD-02: quality score on sessions.
        self.ensure_column("collected_sessions", "quality_score", "REAL")?;

        // Incremental collection: content hash for accurate change detection.
        self.ensure_column("collector_file_states", "content_hash", "TEXT")?;

        // PRD-01 patch FR-F: pinned version on project bindings (multi-version skills).
        self.ensure_column("skill_project_bindings", "pinned_version", "TEXT")?;

        // PRD-06 §3.1/§5.1/§5.3: Agent entity refactor.
        // - `agents.source`: collection normalization key (replaces name-LIKE hack).
        // - `sync_targets.agent_directory_id`: which directory a sync lands in (1 Agent : N dirs).
        self.ensure_column("agents", "source", "TEXT")?;
        self.ensure_column("sync_targets", "agent_directory_id", "TEXT")?;
        // P1-3: skill lifecycle status.
        self.ensure_column("skills", "status", "TEXT DEFAULT 'draft'")?;
        // Run the one-shot merge migration (idempotent — guarded by a sentinel).
        self.migrate_agent_entity_v2()?;
        // PRD-08: seed the analysis dimension tables (pricing + tool caps) once.
        self.migrate_digest_dimensions()?;
        // P3-10: one-time cleanup of phantom projects + Kimi tag alignment.
        self.migrate_project_link_hygiene(device_id)?;

        Ok(())
    }

    // -------------------------------------------------------------------------
    // P3-10: project-link hygiene migration.
    //
    // One-time repair of the agent↔project association data (the detail page's
    // 「关联项目」), which had accumulated three classes of garbage:
    //   1. Phantom projects: collectors recorded bare relative cwds
    //      (`claude-chrome`) or paths inside a tool's own data dir
    //      (`~/.codex/sessions/...`) as project_path → projects rows that are
    //      not projects.
    //   2. Duplicate spellings of the same physical path, inflating
    //      agents.project_count (Claude Code showed 177 vs ~102 real ones).
    //   3. Stale counters on agents whose instances had been wiped.
    // Also aligns the Kimi collector tag with its agents_table key
    // (`kimi-code` → `kimi-code-cli`) in every stored row, so the
    // source-equality attribution matches without aliases. SOURCE_ANTIGRAVITY
    // is intentionally untouched: it collects the Antigravity *IDE*, a
    // different product from the `antigravity-cli` matrix key.
    // Idempotent via the `project_link_hygiene_done` sentinel; takes a
    // pre-migration backup because it deletes rows.
    // -------------------------------------------------------------------------
    fn migrate_project_link_hygiene(&mut self, device_id: &str) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )?;
        let done: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM schema_meta WHERE key = 'project_link_hygiene_done'",
                [],
                |_| Ok(()),
            )
            .is_ok();
        if done {
            return Ok(());
        }

        self.backup_before_migration();

        // 1. Rename the Kimi collector tag to the matrix key in every table
        //    that stores a source column.
        for table in [
            "collected_sessions",
            "collected_prompts",
            "collected_token_usage",
            "skill_usage_attributions",
            "collector_file_states",
            "collected_sources",
        ] {
            let sql = format!(
                "UPDATE {table} SET source = 'kimi-code-cli' WHERE source = 'kimi-code'"
            );
            if let Err(e) = self.conn.execute(&sql, []) {
                // Defensive: every listed table exists in this schema, but a
                // partial schema must not block boot.
                eprintln!("[migration] kimi tag rename skipped for {table}: {e}");
            }
        }

        // 2. Rebuild the association data inside one transaction.
        let tx = self.conn.transaction()?;
        match Self::rebuild_project_links_tx(&tx, device_id) {
            Ok((phantom, merged, linked)) => {
                tx.commit()?;
                eprintln!(
                    "[migration] project link hygiene: {phantom} phantom projects removed, \
                     {merged} duplicates merged, {linked} sessions relinked"
                );
            }
            Err(e) => {
                let _ = tx.rollback();
                anyhow::bail!("project link hygiene migration failed (rolled back): {e}");
            }
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('project_link_hygiene_done', '1')",
            [],
        )?;
        Ok(())
    }

    /// E2-S2.1.4：一次性重打标 collected_prompts.prompt_kind。规则修正后，老数据里
    /// 被误标为 user 的系统注入行（如 TodoWrite 提醒）需按最新规则重算；只跑一次。
    fn retag_prompt_kinds(&self) -> Result<()> {
        // schema_meta 在部分迁移路径中晚于本调用创建，先确保存在。
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )?;
        let done: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'prompt_kind_retag_v1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if done {
            return Ok(());
        }

        let rows: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, IFNULL(prompt_text, '') FROM collected_prompts")?;
            let mapped = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        };

        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let mut changed = 0usize;
        for (id, text) in &rows {
            let kind = if crate::prompt_kind::is_user_prompt(text) {
                crate::prompt_kind::PROMPT_KIND_USER
            } else {
                crate::prompt_kind::PROMPT_KIND_NON_USER
            };
            changed += self.conn.execute(
                "UPDATE collected_prompts SET prompt_kind = ?1 WHERE id = ?2 AND (prompt_kind IS NULL OR prompt_kind != ?1)",
                rusqlite::params![kind, id],
            )?;
        }
        self.conn.execute("COMMIT", [])?;
        self.conn.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('prompt_kind_retag_v1', '1')",
            [],
        )?;
        eprintln!("[migrate] prompt_kind retag: {changed} row(s) updated of {}", rows.len());
        Ok(())
    }

    /// Add a column to a table if absent. Used for additive migrations on tables
    /// that pre-date the column (CREATE TABLE IF NOT EXISTS does not evolve schema).
    fn ensure_column(&self, table: &str, column: &str, decl: &str) -> Result<()> {
        let has_col: bool = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })?
            .any(|n| n.as_deref() == Ok(column));
        if !has_col {
            self.conn
                .execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
        }
        Ok(())
    }

    /// Add a `device_id TEXT` column to the given table if absent.
    fn ensure_device_id_column(&self, table: &str) -> Result<()> {
        let has_col: bool = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })?
            .any(|n| n.as_deref() == Ok("device_id"));
        if !has_col {
            self.conn
                .execute(&format!("ALTER TABLE {table} ADD COLUMN device_id TEXT"), [])?;
        }
        Ok(())
    }

    /// PRD-06 §6 / §11b: copy the database file (plus any WAL/SHM sidecars) to a
    /// timestamped `migration-backup-<ts>.db` next to the live db, before the
    /// agent-entity migration runs. Best-effort: failures are logged to stderr and
    /// swallowed — the in-transaction rollback remains the primary safety net, and
    /// blocking boot on a backup-copy error would be worse than migrating without one.
    /// Keeps at most the 3 newest backups; older ones are pruned.
    fn backup_before_migration(&self) {
        let parent = match self.path.parent() {
            Some(p) => p,
            None => {
                eprintln!("[migration] cannot resolve db parent dir; skipping backup");
                return;
            }
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup = parent.join(format!("migration-backup-{ts}.db"));

        // Copy the main db file, then the -wal and -shm sidecars if present, so a
        // hot backup captures the latest committed state.
        let copy_result = (|| -> std::io::Result<()> {
            std::fs::copy(&self.path, &backup)?;
            for ext in ["wal", "shm"] {
                let src = parent.join(format!(
                    "{}-{ext}",
                    self.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("skillmint.db")
                ));
                if src.exists() {
                    let dst = parent.join(format!(
                        "{}-{ext}",
                        backup
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("backup.db")
                    ));
                    let _ = std::fs::copy(&src, &dst);
                }
            }
            Ok(())
        })();
        if let Err(e) = copy_result {
            eprintln!(
                "[migration] db backup failed (continuing anyway): {}: {}",
                backup.display(),
                e
            );
            return;
        }

        // Prune old backups: keep the 3 newest by embedded timestamp.
        let mut backups: Vec<(u64, std::path::PathBuf)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if let Some(rest) = name.strip_prefix("migration-backup-") {
                    if let Some(stem) = rest.strip_suffix(".db") {
                        if let Ok(t) = stem.parse::<u64>() {
                            backups.push((t, entry.path()));
                        }
                    }
                }
            }
        }
        backups.sort_unstable_by(|a, b| b.0.cmp(&a.0)); // newest first
        for (_, path) in backups.into_iter().skip(3) {
            let _ = std::fs::remove_file(path);
        }
    }

    // -------------------------------------------------------------------------
    // PRD-06 §3.1: Agent entity refactor migration.
    //
    // Merges directory-grained agent rows (e.g. `agent-claude-code` +
    // `agent-claude-commands`) into a single tool-grained Agent, demoting the
    // extra directories to `agent_directories` sub-records. Rewrites
    // `sync_targets.agent_id` to point at the canonical Agent and fills
    // `agent_directory_id`. Backfills `agents.source`. Idempotent via a sentinel
    // row in a metadata table; wrapped in a transaction with a pre-migration
    // backup so a failure rolls back and never leaves orphaned sync_targets.
    // -------------------------------------------------------------------------

    /// The merge whitelist. Derived from `scan.rs` presets: tools that historically
    /// produced more than one agent row (one per directory) collapse to one.
    /// Order matters only for readability; matching is by exact id.
    fn agent_merge_whitelist() -> &'static [MergeTarget] {
        &[
            MergeTarget {
                canonical_id: "agent-claude-code",
                source: "claude-code",
                aliases: &[
                    ("agent-claude-code", "skills"),
                    ("agent-claude-commands", "commands"),
                ],
            },
            MergeTarget {
                canonical_id: "agent-cursor",
                source: "cursor",
                aliases: &[
                    ("agent-cursor", "skills"),
                    ("agent-cursor-rules", "rules"),
                ],
            },
            MergeTarget {
                canonical_id: "agent-codex",
                source: "codex",
                aliases: &[
                    ("agent-codex", "skills"),
                    ("agent-codex-instructions", "instructions"),
                ],
            },
        ]
    }

    /// Idempotent: runs once, guarded by `pragma_agent_entity_v2` sentinel in
    /// `collected_sources`-independent metadata. Safe to call every boot.
    fn migrate_agent_entity_v2(&mut self) -> Result<()> {
        // Sentinel: a dedicated metadata table. Cheaper than a column check each boot.
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )?;
        let done: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM schema_meta WHERE key = 'agent_entity_v2_done'",
                [],
                |_| Ok(()),
            )
            .is_ok();
        if done {
            return Ok(());
        }

        // PRD-06 §6 / §11b: take a physical backup of the database file before
        // any structural change, so a migration failure (or a bad merge) is
        // always recoverable by the user. Best-effort: a copy failure is logged
        // but does NOT block the migration (the in-transaction rollback is the
        // primary safety net; the backup is defense-in-depth).
        self.backup_before_migration();

        // Always backfill `agents.source` for any agent we can identify, even on
        // a repeat run (cheap, idempotent UPDATE — only touches NULL sources).
        for t in Self::agent_merge_whitelist() {
            let mut ids: Vec<&dyn rusqlite::ToSql> = vec![&t.source];
            for (alias_id, _) in t.aliases {
                ids.push(alias_id);
            }
            let placeholders = t.aliases.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "UPDATE agents SET source = ? WHERE id IN ({placeholders}) AND source IS NULL"
            );
            self.conn.execute(&sql, ids.as_slice())?;
        }

        // Begin the structural merge inside a transaction.
        let tx = self.conn.transaction()?;
        let result = (|| -> Result<()> {
            for t in Self::agent_merge_whitelist() {
                // 1. Ensure the canonical agent row exists with the right source.
                //    (If only an alias row exists — e.g. user only ever had commands —
                //    promote the first existing alias to canonical by id rewrite.)
                let canonical_exists: bool = tx
                    .query_row(
                        "SELECT 1 FROM agents WHERE id = ?1",
                        params![t.canonical_id],
                        |_| Ok(()),
                    )
                    .is_ok();

                if !canonical_exists {
                    // Promote the first existing alias to the canonical id, if any.
                    let alias_ids: Vec<&&str> = t.aliases.iter().map(|(a, _)| a).collect();
                    let alias_params: Vec<&dyn rusqlite::ToSql> =
                        alias_ids.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
                    let promo_id: Option<String> = tx
                        .query_row(
                            &format!(
                                "SELECT id FROM agents WHERE id IN ({}) ORDER BY id LIMIT 1",
                                t.aliases.iter().map(|_| "?").collect::<Vec<_>>().join(",")
                            ),
                            alias_params.as_slice(),
                            |row| row.get(0),
                        )
                        .ok();
                    if let Some(alias_existing_id) = promo_id {
                        // Rewrite this alias row's id to the canonical id and set source.
                        tx.execute(
                            "UPDATE agents SET id = ?1, source = ?2 WHERE id = ?3",
                            params![t.canonical_id, t.source, &alias_existing_id],
                        )?;
                    } else {
                        // Neither canonical nor any alias exists — nothing to merge.
                        continue;
                    }
                } else {
                    tx.execute(
                        "UPDATE agents SET source = ?1 WHERE id = ?2 AND source IS NULL",
                        params![t.source, t.canonical_id],
                    )?;
                }

                // 2. For each alias row (including the canonical's own directory),
                //    record it as an agent_directories sub-record of the canonical,
                //    then fold its sync_targets into the canonical agent.
                let now = now_secs();
                for (alias_id, role) in t.aliases {
                    // Read the alias row's skill_directory (if the row still exists).
                    let dir_row: Option<(String, Option<String>)> = tx
                        .query_row(
                            "SELECT skill_directory, device_id FROM agents WHERE id = ?1",
                            params![alias_id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .ok();
                    let Some((dir_path, _device_id)) = dir_row else {
                        // Alias row already gone (prior migration) — skip.
                        continue;
                    };

                    // 2a. Upsert agent_directories row (path uniquely identifies within agent).
                    let adir_id = format!("adir-{}-{}", t.canonical_id, role);
                    tx.execute(
                        "INSERT OR IGNORE INTO agent_directories (id, agent_id, path, role, is_enabled, created_at)
                         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                        params![adir_id, t.canonical_id, &dir_path, role, now],
                    )?;

                    // 2b. Rewire this alias's sync_targets onto the canonical agent,
                    //     stamping agent_directory_id. Resolve (skill, dir) collisions:
                    //     if a sync_target already exists for the same skill under the
                    //     canonical agent, keep the existing one and delete the alias's
                    //     duplicate rather than violating UNIQUE(skill_id, agent_id).
                    if *alias_id != t.canonical_id {
                        // Stamp the directory on the alias's targets first.
                        tx.execute(
                            "UPDATE sync_targets SET agent_directory_id = ?1 WHERE agent_id = ?2 AND agent_directory_id IS NULL",
                            params![adir_id, alias_id],
                        )?;
                        // Delete alias targets whose (skill_id, agent_id=canonical) already exists.
                        tx.execute(
                            "DELETE FROM sync_targets
                             WHERE agent_id = ?1
                               AND skill_id IN (
                                   SELECT skill_id FROM sync_targets WHERE agent_id = ?2
                               )",
                            params![alias_id, t.canonical_id],
                        )?;
                        // Move the remaining alias targets to the canonical agent.
                        tx.execute(
                            "UPDATE sync_targets SET agent_id = ?1 WHERE agent_id = ?2",
                            params![t.canonical_id, alias_id],
                        )?;
                        // 2c. Drop the folded alias agent row.
                        tx.execute(
                            "DELETE FROM agents WHERE id = ?1",
                            params![alias_id],
                        )?;
                    } else {
                        // Canonical row's own targets: stamp its primary directory id.
                        tx.execute(
                            "UPDATE sync_targets SET agent_directory_id = ?1 WHERE agent_id = ?2 AND agent_directory_id IS NULL",
                            params![adir_id, t.canonical_id],
                        )?;
                    }
                }
            }

            // 3. Backfill agent_directories for any agent NOT in the whitelist
            //    (user-created agents, or single-dir presets like Kiro/Lingma).
            //    Give each a single sub-record from its skill_directory if it has none yet.
            {
                let mut stmt = tx.prepare(
                    "SELECT a.id, a.skill_directory FROM agents a
                     WHERE NOT EXISTS (SELECT 1 FROM agent_directories d WHERE d.agent_id = a.id)",
                )?;
                let rows: Vec<(String, String)> = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                let now = now_secs();
                for (agent_id, dir_path) in rows {
                    let adir_id = format!("adir-{agent_id}-primary");
                    tx.execute(
                        "INSERT OR IGNORE INTO agent_directories (id, agent_id, path, role, is_enabled, created_at)
                         VALUES (?1, ?2, ?3, 'skills', 1, ?4)",
                        params![adir_id, agent_id, dir_path, now],
                    )?;
                }
            }

            // 4. Consistency check (PRD-06 §11b decision 1): no orphan sync_targets.
            let orphan_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM sync_targets
                 WHERE agent_id NOT IN (SELECT id FROM agents)",
                [],
                |row| row.get(0),
            )?;
            anyhow::ensure!(orphan_count == 0, "migration produced {orphan_count} orphan sync_targets; rolling back");

            // 5. Stamp the sentinel.
            tx.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('agent_entity_v2_done', '1')",
                [],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                tx.commit()?;
                Ok(())
            }
            Err(e) => {
                // rollback is implicit on drop, but be explicit for clarity.
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    // -------------------------------------------------------------------------
    // PRD-08 §5: seed the analysis dimension tables (pricing + tool caps).
    //
    // Idempotent via `INSERT OR IGNORE`: every builtin row is inserted only if
    // absent, so user edits are never overwritten and **new** sources/models
    // added in a later build are back-filled on the next boot. There is deliberately
    // NO permanent sentinel — a one-shot guard would prevent back-filling when
    // `ALL_TOOL_CAPABILITIES` / `MODEL_PRICING` grow (which is exactly how the 6
    // session-only collectors were lost on upgraded installs).
    // -------------------------------------------------------------------------

    fn migrate_digest_dimensions(&self) -> Result<()> {
        let now = now_secs();
        // Seed pricing: INSERT OR IGNORE preserves any user edits on re-seed.
        for (model_id, mode, in_p, cache_p, out_p) in crate::pricing::MODEL_PRICING {
            self.conn.execute(
                "INSERT OR IGNORE INTO digest_model
                 (model_id, display_name, input_price, cache_price, output_price, billing_mode, source, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'builtin', ?7)",
                params![model_id, model_id, in_p, cache_p, out_p, mode.to_string(), now],
            )?;
        }
        // Seed tool capability flags (one row per known source).
        for (source, caps) in crate::collector::ALL_TOOL_CAPABILITIES {
            self.conn.execute(
                "INSERT OR IGNORE INTO digest_tool
                 (source, has_token, has_prompt, has_tool_calls, has_turn_duration, billing_mode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    source,
                    caps.has_token as i64,
                    caps.has_prompt as i64,
                    caps.has_tool_calls as i64,
                    caps.has_turn_duration as i64,
                    caps.billing_mode.to_string(),
                ],
            )?;
        }
        Ok(())
    }

    /// Transaction-aware core of `ensure_project_by_path`. Lets import/write
    /// paths run the project lookup/insert on the same connection (including a
    /// transaction) to avoid split-brain inserts and partial rollbacks.
    fn ensure_project_by_path_conn(
        conn: &rusqlite::Connection,
        device_id: &str,
        raw_path: &str,
    ) -> Result<Option<String>> {
        let normalized = {
            // P3-10: only plausible working directories become projects — see
            // `is_linkable_project_path` for the garbage this keeps out.
            if !is_linkable_project_path(raw_path) {
                return Ok(None);
            }
            normalize_path(raw_path)
        };
        if normalized.is_empty() {
            return Ok(None);
        }
        // Look up by (device_id, path).
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM projects WHERE device_id = ?1 AND path = ?2",
                params![device_id, normalized],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Ok(Some(id));
        }
        let id = new_id();
        let name = normalized
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("project")
            .to_string();
        let now = now_secs();
        conn.execute(
            r#"INSERT OR IGNORE INTO projects
               (id, device_id, name, path, first_seen_at, last_active_at, is_stale, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)"#,
            params![id, device_id, name, normalized, now, now, now, now],
        )?;
        // Re-query: INSERT OR IGNORE may have been a no-op if a concurrent
        // insert won the race, in which case our generated `id` is not the row
        // that actually exists. Return the true row id.
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM projects WHERE device_id = ?1 AND path = ?2",
                params![device_id, normalized],
                |row| row.get(0),
            )
            .ok();
        Ok(id)
    }

    // ----- PRD-03: knowledge graph storage -----------------------------------

    /// Execute a raw SQL statement (for maintenance like clearing auto-edges).
    pub fn conn_execute(&self, sql: &str) -> Result<()> {
        self.conn.execute(sql, [])?;
        Ok(())
    }

    /// Borrow the connection for read-only queries (used by kg coverage report).
    pub fn conn_ref(&self) -> &Connection {
        &self.conn
    }
}

fn default_task(
    task_kind: TaskKind,
    name: &str,
    description: &str,
    strategy: ScheduleStrategy,
    enabled: bool,
) -> ScheduledTask {
    let now = now_secs();
    ScheduledTask {
        id: new_id(),
        task_kind,
        name: name.to_string(),
        description: description.to_string(),
        enabled,
        strategy,
        created_at: now,
        updated_at: now,
        last_run_at: None,
        last_status: None,
        next_run_at: None,
        run_count: 0,
        error_count: 0,
    }
}

fn status_to_db(status: &RunStatus) -> String {
    match status {
        RunStatus::Pending => "pending".to_string(),
        RunStatus::Running => "running".to_string(),
        RunStatus::Success => "success".to_string(),
        RunStatus::Failed => "failed".to_string(),
        RunStatus::Skipped => "skipped".to_string(),
    }
}

fn status_from_db(s: &str) -> RunStatus {
    match s {
        "running" => RunStatus::Running,
        "success" => RunStatus::Success,
        "failed" => RunStatus::Failed,
        "skipped" => RunStatus::Skipped,
        _ => RunStatus::Pending,
    }
}

fn trigger_to_db(trigger: &TriggerSource) -> String {
    match trigger {
        TriggerSource::Schedule => "schedule".to_string(),
        TriggerSource::Manual => "manual".to_string(),
        TriggerSource::Startup => "startup".to_string(),
    }
}

fn trigger_from_db(s: &str) -> TriggerSource {
    match s {
        "manual" => TriggerSource::Manual,
        "startup" => TriggerSource::Startup,
        _ => TriggerSource::Schedule,
    }
}

fn strategy_to_db(strategy: &ScheduleStrategy) -> (String, Option<i64>, Option<String>, Option<String>) {
    match strategy {
        ScheduleStrategy::Manual => ("manual".to_string(), None, None, None),
        ScheduleStrategy::Interval { value, unit } => {
            let unit_str = match unit {
                IntervalUnit::Minutes => "minutes",
                IntervalUnit::Hours => "hours",
                IntervalUnit::Days => "days",
            };
            ("interval".to_string(), Some(*value as i64), Some(unit_str.to_string()), None)
        }
        ScheduleStrategy::Cron { expression } => {
            ("cron".to_string(), None, None, Some(expression.clone()))
        }
    }
}

fn strategy_from_db(
    kind: &str,
    value: Option<i64>,
    unit: Option<String>,
    expr: Option<String>,
) -> ScheduleStrategy {
    match kind {
        "interval" => {
            let unit = unit.as_deref().map(|u| match u {
                "hours" => IntervalUnit::Hours,
                "days" => IntervalUnit::Days,
                _ => IntervalUnit::Minutes,
            }).unwrap_or(IntervalUnit::Minutes);
            ScheduleStrategy::Interval {
                value: value.unwrap_or(30) as u32,
                unit,
            }
        }
        "cron" => ScheduleStrategy::Cron {
            expression: expr.unwrap_or_default(),
        },
        _ => ScheduleStrategy::Manual,
    }
}

fn map_scheduled_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTask> {
    let kind_str: String = row.get(1)?;
    let task_kind = TaskKind::from_str(&kind_str).unwrap_or(TaskKind::CollectUsageData);
    let strategy_kind: String = row.get(5)?;
    let strategy = strategy_from_db(
        &strategy_kind,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    );
    Ok(ScheduledTask {
        id: row.get(0)?,
        task_kind,
        name: row.get(2)?,
        description: row.get(3)?,
        enabled: row.get::<_, i32>(4)? != 0,
        strategy,
        created_at: row.get::<_, u64>(9)?,
        updated_at: row.get::<_, u64>(10)?,
        last_run_at: row.get::<_, Option<u64>>(11)?,
        last_status: row.get::<_, Option<String>>(12)?.as_deref().map(status_from_db),
        next_run_at: row.get::<_, Option<u64>>(13)?,
        run_count: row.get::<_, u64>(14)?,
        error_count: row.get::<_, u64>(15)?,
    })
}

fn map_task_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    let status_str: String = row.get(2)?;
    let triggered_str: String = row.get(8)?;
    Ok(TaskRun {
        id: row.get(0)?,
        task_id: row.get(1)?,
        status: status_from_db(&status_str),
        started_at: row.get::<_, u64>(3)?,
        finished_at: row.get::<_, Option<u64>>(4)?,
        duration_ms: row.get::<_, Option<u64>>(5)?,
        result_summary: row.get(6)?,
        error_message: row.get(7)?,
        triggered_by: trigger_from_db(&triggered_str),
    })
}

fn discovery_status_from_db(s: &str) -> crate::models::DiscoveryStatus {
    match s {
        "accepted" => crate::models::DiscoveryStatus::Accepted,
        "dismissed" => crate::models::DiscoveryStatus::Dismissed,
        "expired" => crate::models::DiscoveryStatus::Expired,
        _ => crate::models::DiscoveryStatus::Pending,
    }
}

fn map_discovery_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::models::Discovery> {
    let payload_json: String = row.get(3)?;
    let payload = serde_json::from_str(&payload_json).unwrap_or_default();
    let kind_str: String = row.get(1)?;
    let status_str: String = row.get(6)?;
    Ok(crate::models::Discovery {
        id: row.get(0)?,
        kind: crate::models::DiscoveryKind::from_str(&kind_str).unwrap_or(crate::models::DiscoveryKind::RepeatPattern),
        title: row.get(2)?,
        payload,
        confidence: row.get(4)?,
        dedup_key: row.get(5)?,
        status: discovery_status_from_db(&status_str),
        created_at: row.get::<_, i64>(7)? as u64,
        decided_at: row.get::<_, Option<i64>>(8)?.map(|t| t as u64),
        resulting_skill_id: row.get(9)?,
    })
}

fn map_source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    let type_str: String = row.get(2)?;
    Ok(Source {
        id: row.get(0)?,
        name: row.get(1)?,
        source_type: SourceType::from_str(&type_str),
        url: row.get(3)?,
        ref_spec: row.get(4)?,
        subpath: row.get(5)?,
        cache_path: row.get(6)?,
        commit_sha: row.get(7)?,
        added_at: row.get(8)?,
        last_fetched_at: row.get(9)?,
        pull_policy: row.get(10)?,
        remote_revision: row.get(11)?,
    })
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Canonicalize a path for stable project identity. Falls back to the raw value
/// if canonicalization fails (e.g. path no longer exists).
fn normalize_path(raw: &str) -> String {
    let expanded = crate::scan::expand_path(raw);
    std::fs::canonicalize(&expanded)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| expanded.to_string_lossy().to_string())
}

/// P3-10: only plausible working directories become project rows. Collectors
/// occasionally record garbage cwds — relative fragments (`claude-chrome`) or
/// the inside of a tool's own data dir (`~/.codex/sessions/2026/07`) — and
/// linking those produced phantom projects that inflated 关联项目 counts.
/// Rule: non-empty, absolute, and — when under $HOME — not inside a hidden
/// (dot) top-level directory, which is where every known agent keeps its data.
pub(crate) fn is_linkable_project_path(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    let expanded = crate::scan::expand_path(trimmed);
    if !expanded.is_absolute() {
        return false;
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = expanded.strip_prefix(&home) {
            if let Some(first) = rel.components().next() {
                if first.as_os_str().to_string_lossy().starts_with('.') {
                    return false;
                }
            }
        }
    }
    true
}

fn parse_sync_mode(s: &str) -> SyncMode {
    match s {
        "copy" => SyncMode::Copy,
        _ => SyncMode::Symlink,
    }
}

fn parse_sync_status(s: &str) -> SyncStatus {
    match s {
        "local_changed" => SyncStatus::LocalChanged,
        "center_changed" => SyncStatus::CenterChanged,
        "conflict" => SyncStatus::Conflict,
        "broken" => SyncStatus::Broken,
        _ => SyncStatus::Synced,
    }
}

fn parse_skill_status(s: &str) -> crate::models::SkillStatus {
    match s {
        "candidate" => crate::models::SkillStatus::Candidate,
        "approved" => crate::models::SkillStatus::Approved,
        "deprecated" => crate::models::SkillStatus::Deprecated,
        _ => crate::models::SkillStatus::Draft,
    }
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::{iso_to_secs, normalize_digest_source};

    #[test]
    fn iso_to_secs_parses_datetime_and_date() {
        // Full datetime.
        let s = iso_to_secs("2026-06-28 14:30:00").unwrap();
        // Date only → midnight (00:00:00) of that day.
        let d = iso_to_secs("2026-06-28").unwrap();
        assert_eq!(s, d + 14 * 3600 + 30 * 60);
        // ISO 'T' separator.
        let t = iso_to_secs("2026-06-28T14:30:00").unwrap();
        assert_eq!(t, s);
    }

    #[test]
    fn iso_to_secs_parses_rfc3339_with_offset_and_fractions() {
        // AI-Digest stores timestamps like "2026-06-25T12:08:58.993000+08:00".
        let ts = iso_to_secs("2026-06-25T12:08:58.993000+08:00").unwrap();
        // Same instant in UTC: 04:08:58 UTC.
        let expected = iso_to_secs("2026-06-25 04:08:58").unwrap();
        assert_eq!(ts, expected);
    }

    #[test]
    fn iso_to_secs_rejects_garbage() {
        assert!(iso_to_secs("not a date").is_none());
        assert!(iso_to_secs("").is_none());
    }

    #[test]
    fn normalize_digest_source_maps_human_names_to_keys() {
        assert_eq!(normalize_digest_source("Claude Code"), "claude-code");
        assert_eq!(normalize_digest_source("ZCode"), "zcode");
        assert_eq!(normalize_digest_source("Trae Solo"), "trae-solo");
        assert_eq!(normalize_digest_source("Gemini CLI"), "gemini-cli");
        // An already-slugged key passes through (lowercased).
        assert_eq!(normalize_digest_source("claude-code"), "claude-code");
        // An unknown name is slugified as a fallback.
        assert_eq!(normalize_digest_source("Some New Agent"), "some-new-agent");
    }

    #[test]
    fn digest_detect_returns_none_when_absent() {
        // No ~/.digest on the CI/this machine (normally) → None, never panics.
        let _ = super::Db::detect_digest_db();
    }

    /// PRD-08 regression: the dimension seed must be **forward-migrating** — a
    /// later build that adds new sources to ALL_TOOL_CAPABILITIES must back-fill
    /// them on the next boot, even on a DB created by an earlier build. The old
    /// permanent-sentinel design failed this (new sources were never seeded).
    #[test]
    fn dimension_seed_backfills_new_sources_on_re_run() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.keep();
        let path = dir.join("test.db");
        {
            // First boot: creates the schema and seeds whatever is in the catalog.
            let mut db = super::Db::new(&path).unwrap();
            db.init("dev").unwrap();
            let n_after_first: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM digest_tool", [], |r| r.get(0))
                .unwrap();
            // Pretend an older build: drop a couple of sources to simulate an
            // upgrade that adds them.
            db.conn()
                .execute("DELETE FROM digest_tool WHERE source IN ('opencode','codebuddy')", [])
                .unwrap();
            // Re-run the seed (simulates a second boot on the new build).
            db.migrate_digest_dimensions().unwrap();
            let n_after_backfill: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM digest_tool", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n_after_first, n_after_backfill, "deleted sources must be re-seeded");
        }
    }
}
