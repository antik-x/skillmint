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

    /// PRD-08: load all `digest_model` rows (pricing + billing_mode), keyed by
    /// lowercased model_id. Used by the pricing engine to resolve costs. Never
    /// fails — an empty result falls back to the builtin constant in pricing.rs.
    pub fn load_model_pricing(&self) -> Result<Vec<crate::pricing::ModelPricingRow>> {
        use crate::settings::BillingMode;
        let mut stmt = self.conn.prepare(
            "SELECT model_id, input_price, cache_price, output_price, billing_mode
             FROM digest_model",
        )?;
        let rows = stmt.query_map([], |row| {
            let mode_str: Option<String> = row.get(4)?;
            Ok(crate::pricing::ModelPricingRow {
                model_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                input_price: row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                cache_price: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                output_price: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                billing_mode: mode_str
                    .as_deref()
                    .map(BillingMode::from_str)
                    .unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08: read normalized token-usage rows in an epoch-seconds window
    /// [start, end] inclusive, optionally filtered to one `source`. The window
    /// bounds are compared against `collected_sessions.start_time` (epoch secs,
    /// local time) — matching the existing `get_agent_usage_summary` caliber.
    /// Returns one `UsageSample` per `collected_token_usage` row joined to its
    /// session for project attribution.
    pub fn query_usage_samples(
        &self,
        start: i64,
        end: i64,
        source: Option<&str>,
    ) -> Result<Vec<crate::models::UsageSample>> {
        let sql = r#"SELECT tu.session_id,
                           COALESCE(tu.source, s.source),
                           s.project_path,
                           tu.model_id,
                           tu.input_tokens, tu.output_tokens, tu.reasoning_tokens,
                           tu.cache_creation_input_tokens, tu.cache_read_input_tokens,
                           tu.total_tokens, tu.model_calls, tu.tool_calls, tu.duration_ms
                    FROM collected_token_usage tu
                    LEFT JOIN collected_sessions s ON s.id = tu.session_id
                    WHERE IFNULL(s.start_time, 0) >= ?1 AND IFNULL(s.start_time, 0) <= ?2
                      AND (?3 = '' OR tu.source = ?3)"#;
        let mut stmt = self.conn.prepare(sql)?;
        let src = source.unwrap_or("");
        let rows = stmt.query_map(params![start, end, src], |row| {
            Ok(crate::models::UsageSample {
                session_id: row.get(0)?,
                source: row.get(1)?,
                project_path: row.get(2)?,
                model_id: row.get(3)?,
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                reasoning_tokens: row.get(6)?,
                cache_creation_input_tokens: row.get(7)?,
                cache_read_input_tokens: row.get(8)?,
                total_tokens: row.get(9)?,
                model_calls: row.get(10)?,
                tool_calls: row.get(11)?,
                duration_ms: row.get(12)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08: read prompt samples in an epoch-seconds window, optionally
    /// filtered to one source. Joins `collected_prompts` to its session for
    /// project_path. Semantic columns come through as-is (None until classified).
    pub fn query_prompt_samples(
        &self,
        start: i64,
        end: i64,
        source: Option<&str>,
    ) -> Result<Vec<crate::models::PromptSample>> {
        let sql = r#"SELECT p.session_id,
                           COALESCE(p.source, s.source),
                           s.project_path,
                           p.duration_ms, p.tool_calls,
                           p.requested_action, p.target_object,
                           p.interaction_state, p.interaction_mode
                    FROM collected_prompts p
                    LEFT JOIN collected_sessions s ON s.id = p.session_id
                    WHERE IFNULL(p.started_at, 0) >= ?1 AND IFNULL(p.started_at, 0) <= ?2
                      AND (?3 = '' OR p.source = ?3)"#;
        let mut stmt = self.conn.prepare(sql)?;
        let src = source.unwrap_or("");
        let rows = stmt.query_map(params![start, end, src], |row| {
            Ok(crate::models::PromptSample {
                session_id: row.get(0)?,
                source: row.get(1)?,
                project_path: row.get(2)?,
                duration_ms: row.get(3)?,
                tool_calls: row.get(4)?,
                requested_action: row.get(5)?,
                target_object: row.get(6)?,
                interaction_state: row.get(7)?,
                interaction_mode: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.3 (P1): count prompts that have text but are not yet labeled.
    /// Used by the classifier to report the eligible total when LLM is off.
    pub fn count_unclassified_prompts(&self, source: Option<&str>) -> Result<usize> {
        let sql = "SELECT COUNT(*) FROM collected_prompts
                   WHERE IFNULL(prompt_text,'') != '' AND requested_action IS NULL
                     AND (?1 = '' OR source = ?1)";
        let src = source.unwrap_or("");
        let n: i64 = self.conn.query_row(sql, params![src], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// PRD-08 §3.3 (P1): load prompts awaiting classification (text present,
    /// no label yet). Returns (id, prompt_text).
    pub fn load_unclassified_prompts(
        &self,
        source: Option<&str>,
    ) -> Result<Vec<crate::classifier::UnclassifiedPrompt>> {
        let sql = "SELECT id, prompt_text FROM collected_prompts
                   WHERE IFNULL(prompt_text,'') != '' AND requested_action IS NULL
                     AND (?1 = '' OR source = ?1)";
        let src = source.unwrap_or("");
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![src], |row| {
            Ok(crate::classifier::UnclassifiedPrompt {
                id: row.get(0)?,
                prompt_text: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.3 (P1): write the four-axis label + confidence back to a
    /// prompt row. Only updates the semantic columns; leaves text/source intact.
    pub fn update_prompt_semantic(
        &self,
        id: &str,
        label: &crate::classifier::PromptLabel,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE collected_prompts
             SET requested_action = ?1, target_object = ?2,
                 interaction_state = ?3, interaction_mode = ?4, confidence = ?5
             WHERE id = ?6",
            params![
                label.requested_action,
                label.target_object,
                label.interaction_state,
                label.interaction_mode,
                label.confidence,
                id,
            ],
        )?;
        Ok(())
    }

    /// SPEC-I2: load sessions in a UTC epoch-seconds range for weekly report context.
    pub fn query_sessions_for_day_range(
        &self,
        start: i64,
        end: i64,
    ) -> Result<Vec<crate::models::SessionStub>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_path, title_or_prompt, message_count
             FROM collected_sessions
             WHERE start_time >= ?1 AND start_time < ?2
             ORDER BY start_time",
        )?;
        let rows = stmt.query_map(params![start, end], |row| {
            Ok(crate::models::SessionStub {
                session_id: row.get(0)?,
                project_path: row.get(1)?,
                title: row.get(2)?,
                message_count: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.6 (P1): load the sessions for a calendar day (YYYY-MM-DD),
    /// compared against `start_time` in local time. Returns the minimal fields
    /// the analyzer needs to build its LLM context.
    pub fn query_sessions_for_day(&self, date: &str) -> Result<Vec<crate::analyzer::DaySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT start_time, source, project_path, title_or_prompt, message_count
             FROM collected_sessions
             WHERE date(start_time, 'unixepoch', 'localtime') = ?1
               AND start_time IS NOT NULL
             ORDER BY start_time",
        )?;
        let rows = stmt.query_map(params![date], |row| {
            Ok(crate::analyzer::DaySession {
                start_time: row.get::<_, Option<i64>>(0)?.map(|n| n.max(0) as u64),
                source: row.get(1)?,
                project_path: row.get(2)?,
                title_or_prompt: row.get(3)?,
                message_count: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.6 (P1): persist a generated daily summary (replace by date).
    pub fn save_daily_summary(
        &self,
        date: &str,
        summary: &crate::analyzer::DailySummary,
        model: &str,
    ) -> Result<()> {
        let highlights = serde_json::to_string(&summary.highlights)?;
        let activities = serde_json::to_string(&summary.activities)?;
        let now = now_secs();
        self.conn.execute(
            "INSERT OR REPLACE INTO digest_summary
             (date, highlights, activities, model, provider, created_at)
             VALUES (?1, ?2, ?3, ?4, 'skillmint', ?5)",
            params![date, highlights, activities, model, now],
        )?;
        Ok(())
    }

    /// PRD-08 §3.6 (P1): load a cached daily summary for a date, if any.
    pub fn get_daily_summary(&self, date: &str) -> Result<Option<crate::analyzer::DailySummary>> {
        let row = self.conn.query_row(
            "SELECT highlights, activities FROM digest_summary WHERE date = ?1",
            params![date],
            |row| {
                let h: String = row.get(0)?;
                let a: String = row.get(1)?;
                Ok((h, a))
            },
        );
        match row {
            Ok((h, a)) => {
                let highlights: Vec<String> = serde_json::from_str(&h).unwrap_or_default();
                let activities: Vec<crate::analyzer::ActivityItem> =
                    serde_json::from_str(&a).unwrap_or_default();
                Ok(Some(crate::analyzer::DailySummary {
                    date: date.to_string(),
                    highlights,
                    activities,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// PRD-11: list cached daily summaries within a date range (inclusive),
    /// ordered by date descending.
    pub fn list_daily_summaries(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<crate::models::DailySummaryMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, model, provider, created_at
             FROM digest_summary
             WHERE date >= ?1 AND date <= ?2
             ORDER BY date DESC",
        )?;
        let rows = stmt.query_map(params![start_date, end_date], |row| {
            Ok(crate::models::DailySummaryMeta {
                date: row.get(0)?,
                model: row.get(1)?,
                provider: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-11: compute aggregate value metrics from all cached daily summaries.
    pub fn get_summary_value_metrics(&self) -> Result<crate::models::SummaryValueMetrics> {
        let covered_days: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT date) FROM digest_summary",
            [],
            |row| row.get(0),
        )?;
        let total_summaries: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM digest_summary",
            [],
            |row| row.get(0),
        )?;

        let mut total_activities: i64 = 0;
        let mut stmt = self
            .conn
            .prepare("SELECT activities FROM digest_summary")?;
        let rows = stmt.query_map([], |row| {
            let activities_json: String = row.get(0)?;
            let activities: Vec<crate::analyzer::ActivityItem> =
                serde_json::from_str(&activities_json).unwrap_or_default();
            Ok(activities.len() as i64)
        })?;
        for len in rows {
            total_activities += len?;
        }

        let mut covered_projects: i64 = 0;
        if total_summaries > 0 {
            let mut stmt = self
                .conn
                .prepare("SELECT activities FROM digest_summary")?;
            let rows = stmt.query_map([], |row| {
                let activities_json: String = row.get(0)?;
                let activities: Vec<crate::analyzer::ActivityItem> =
                    serde_json::from_str(&activities_json).unwrap_or_default();
                let projects: std::collections::HashSet<String> = activities
                    .into_iter()
                    .filter_map(|a| {
                        let p = a.project.trim();
                        if p.is_empty() || p == "-" {
                            None
                        } else {
                            Some(p.to_string())
                        }
                    })
                    .collect();
                Ok(projects)
            })?;
            let mut all_projects = std::collections::HashSet::new();
            for set in rows {
                all_projects.extend(set?);
            }
            covered_projects = all_projects.len() as i64;
        }

        Ok(crate::models::SummaryValueMetrics {
            covered_days,
            total_summaries,
            total_activities,
            covered_projects,
        })
    }

    /// PRD-08 §3.4 (P1): role-profile rows — counts of (source, requested_action)
    /// over classified prompts in a window, filtered by `min_confidence`. Drives
    /// the tool×action row-normalized heatmap.
    pub fn query_role_profile(
        &self,
        start: i64,
        end: i64,
        min_confidence: f64,
    ) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT p.source, p.requested_action, COUNT(*) AS n
               FROM collected_prompts p
               WHERE IFNULL(p.started_at, 0) >= ?1 AND IFNULL(p.started_at, 0) <= ?2
                 AND p.requested_action IS NOT NULL
                 AND IFNULL(p.confidence, 1.0) >= ?3
               GROUP BY p.source, p.requested_action"#,
        )?;
        let rows = stmt.query_map(params![start, end, min_confidence], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.4 (P3): drill-down — fetch the classified prompts that back a
    /// given (source, requested_action) cell, within a window. `action` is
    /// optional so the same query powers a whole-row drill-down too. Used by the
    /// role-profile heatmap cell click → prompt list drawer.
    pub fn query_prompts_for_cell(
        &self,
        start: i64,
        end: i64,
        source: &str,
        action: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CellPrompt>> {
        let sql = r#"SELECT p.prompt_text, p.started_at, p.requested_action,
                            p.target_object, p.interaction_state, p.confidence
                     FROM collected_prompts p
                     WHERE IFNULL(p.started_at, 0) >= ?1 AND IFNULL(p.started_at, 0) <= ?2
                       AND p.source = ?3
                       AND p.requested_action IS NOT NULL
                       AND (?4 = '' OR p.requested_action = ?4)
                     ORDER BY p.started_at DESC
                     LIMIT ?5"#;
        let mut stmt = self.conn.prepare(sql)?;
        let act = action.unwrap_or("");
        let rows = stmt.query_map(params![start, end, source, act, limit], |row| {
            Ok(CellPrompt {
                prompt_text: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                started_at: row.get::<_, Option<i64>>(1)?,
                requested_action: row.get::<_, Option<String>>(2)?,
                target_object: row.get::<_, Option<String>>(3)?,
                interaction_state: row.get::<_, Option<String>>(4)?,
                confidence: row.get::<_, Option<f64>>(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.5 (P1): per-source prompt durations in a window, for the
    /// agent-coefficient-by-tool calc. Only sources whose prompts carry a
    /// non-null duration contribute (the "trusted duration" set).
    pub fn query_prompt_durations_by_source(
        &self,
        start: i64,
        end: i64,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT p.source, p.duration_ms
               FROM collected_prompts p
               WHERE IFNULL(p.started_at, 0) >= ?1 AND IFNULL(p.started_at, 0) <= ?2
                 AND p.duration_ms IS NOT NULL"#,
        )?;
        let rows = stmt.query_map(params![start, end], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-08 §3.8 (P2): detect an AI-Digest `~/.digest/digest.db` and report
    /// its row counts so the UI can show an import preview. Returns None if no
    /// digest.db exists or it can't be read.
    pub fn detect_digest_db() -> Option<(std::path::PathBuf, DigestDbStats)> {
        let path = dirs::home_dir()?.join(".digest/digest.db");
        if !path.is_file() {
            return None;
        }
        let uri = sqlite_uri(&path, "?mode=ro&immutable=1");
        let conn = rusqlite::Connection::open_with_flags(
            &uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap_or(0)
        };
        let stats = DigestDbStats {
            sessions: count("sessions"),
            prompts: count("prompts"),
            token_usage: count("token_usage"),
        };
        Some((path, stats))
    }

    /// PRD-08 §3.8 (P2): read an AI-Digest `~/.digest/digest.db` into memory.
    /// This is a static method so it can run WITHOUT holding `Mutex<Db>`.
    /// Returns an error only when the digest.db cannot be opened or read;
    /// individual malformed rows are logged and skipped.
    pub fn read_digest_db() -> Result<DigestData> {
        let path = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("no home dir"))?
            .join(".digest/digest.db");
        if !path.is_file() {
            anyhow::bail!("~/.digest/digest.db not found");
        }
        let uri = sqlite_uri(&path, "?mode=ro&immutable=1");
        let conn = rusqlite::Connection::open_with_flags(
            &uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        let mut data = DigestData::default();

        // Sessions.
        match conn.prepare(
            "SELECT id, source, start_time, end_time, project_path, title, message_count
             FROM sessions",
        ) {
            Ok(mut stmt) => {
                let rows = stmt.query_map([], |row| {
                    Ok(DigestSessionRow {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        start_time: row.get(2)?,
                        end_time: row.get(3)?,
                        project_path: row.get(4)?,
                        title: row.get(5)?,
                        message_count: row.get(6)?,
                    })
                });
                match rows {
                    Ok(iter) => {
                        for (idx, r) in iter.enumerate() {
                            match r {
                                Ok(row) => data.sessions.push(row),
                                Err(e) => eprintln!("[digest-import] skipping sessions row {}: {}", idx, e),
                            }
                        }
                    }
                    Err(e) => eprintln!("[digest-import] failed to iterate sessions: {}", e),
                }
            }
            Err(e) => eprintln!("[digest-import] sessions table unreadable: {}", e),
        }

        // Token usage.
        match conn.prepare(
            "SELECT session_id, source, project_path, model_id,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_creation_input_tokens, cache_read_input_tokens,
                    total_tokens, model_calls, tool_calls, duration_ms
             FROM token_usage",
        ) {
            Ok(mut stmt) => {
                let rows = stmt.query_map([], |row| {
                    Ok(DigestTokenRow {
                        session_id: row.get(0)?,
                        source: row.get(1)?,
                        project_path: row.get(2)?,
                        model_id: row.get(3)?,
                        input_tokens: row.get(4)?,
                        output_tokens: row.get(5)?,
                        reasoning_tokens: row.get(6)?,
                        cache_creation_input_tokens: row.get(7)?,
                        cache_read_input_tokens: row.get(8)?,
                        total_tokens: row.get(9)?,
                        model_calls: row.get(10)?,
                        tool_calls: row.get(11)?,
                        duration_ms: row.get(12)?,
                    })
                });
                match rows {
                    Ok(iter) => {
                        for (idx, r) in iter.enumerate() {
                            match r {
                                Ok(row) => data.token_usage.push(row),
                                Err(e) => eprintln!("[digest-import] skipping token_usage row {}: {}", idx, e),
                            }
                        }
                    }
                    Err(e) => eprintln!("[digest-import] failed to iterate token_usage: {}", e),
                }
            }
            Err(e) => eprintln!("[digest-import] token_usage table unreadable: {}", e),
        }

        // Prompts.
        match conn.prepare(
            "SELECT turn_id, session_id, source, project_path, prompt_text, started_at,
                    duration_ms, tool_calls, requested_action, target_object,
                    interaction_state, interaction_mode
             FROM prompts",
        ) {
            Ok(mut stmt) => {
                let rows = stmt.query_map([], |row| {
                    Ok(DigestPromptRow {
                        turn_id: row.get(0)?,
                        session_id: row.get(1)?,
                        source: row.get(2)?,
                        project_path: row.get(3)?,
                        prompt_text: row.get(4)?,
                        started_at: row.get(5)?,
                        duration_ms: row.get(6)?,
                        tool_calls: row.get(7)?,
                        requested_action: row.get(8)?,
                        target_object: row.get(9)?,
                        interaction_state: row.get(10)?,
                        interaction_mode: row.get(11)?,
                    })
                });
                match rows {
                    Ok(iter) => {
                        for (idx, r) in iter.enumerate() {
                            match r {
                                Ok(row) => data.prompts.push(row),
                                Err(e) => eprintln!("[digest-import] skipping prompts row {}: {}", idx, e),
                            }
                        }
                    }
                    Err(e) => eprintln!("[digest-import] failed to iterate prompts: {}", e),
                }
            }
            Err(e) => eprintln!("[digest-import] prompts table unreadable: {}", e),
        }

        Ok(data)
    }

    /// PRD-08 §3.8 (P2): import pre-read AI-Digest data into the local
    /// `collected_*` tables. This method is short and fully transactional:
    /// holding `Mutex<Db>` only for the local write, not for the external read.
    /// Idempotent merge by PK — rows already present are kept.
    pub fn import_digest_data(&mut self, device_id: &str, data: DigestData) -> Result<DigestImportSummary> {
        let mut summary = DigestImportSummary::default();
        let now = now_secs();
        let tx = self.conn.transaction()?;

        let result: Result<()> = (|| {
            for row in &data.sessions {
                let source = normalize_digest_source(&row.source);
                let session_pk = format!("{device_id}:import:{}", row.id);
                let project_id = row
                    .project_path
                    .as_ref()
                    .and_then(|p| Self::ensure_project_by_path_conn(&tx, device_id, p).unwrap_or(None));
                tx.execute(
                    "INSERT OR IGNORE INTO collected_sessions
                     (id, device_id, source, project_id, agent_id, start_time, end_time,
                      message_count, title_or_prompt, cached_at, project_path, quality_score)
                     VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, 10.0)",
                    params![
                        session_pk, device_id, source, project_id,
                        row.start_time.as_deref().and_then(iso_to_secs),
                        row.end_time.as_deref().and_then(iso_to_secs),
                        row.message_count.unwrap_or(0), row.title, now, row.project_path,
                    ],
                )?;
                summary.sessions += 1;
            }

            for row in &data.token_usage {
                let source = normalize_digest_source(&row.source);
                let session_pk = format!("{device_id}:import:{}", row.session_id);
                let project_id = row
                    .project_path
                    .as_ref()
                    .and_then(|p| Self::ensure_project_by_path_conn(&tx, device_id, p).unwrap_or(None));
                tx.execute(
                    "INSERT OR IGNORE INTO collected_token_usage
                     (id, device_id, session_id, source, project_id, model_id,
                      input_tokens, output_tokens, reasoning_tokens,
                      cache_creation_input_tokens, cache_read_input_tokens,
                      total_tokens, model_calls, tool_calls, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        format!("{session_pk}:{}", row.model_id.clone().unwrap_or_default()),
                        device_id, session_pk, source, project_id, row.model_id,
                        row.input_tokens, row.output_tokens, row.reasoning_tokens,
                        row.cache_creation_input_tokens, row.cache_read_input_tokens,
                        row.total_tokens, row.model_calls, row.tool_calls, row.duration_ms,
                    ],
                )?;
                summary.token_rows += 1;
            }

            for row in &data.prompts {
                let source = normalize_digest_source(&row.source);
                let session_pk = format!("{device_id}:import:{}", row.session_id);
                let prompt_pk = format!("{device_id}:import:{}", row.turn_id);
                let project_id = row
                    .project_path
                    .as_ref()
                    .and_then(|p| Self::ensure_project_by_path_conn(&tx, device_id, p).unwrap_or(None));
                tx.execute(
                    "INSERT OR IGNORE INTO collected_prompts
                     (id, device_id, session_id, source, project_id, prompt_text, started_at,
                      duration_ms, requested_action, target_object, interaction_state,
                      interaction_mode, confidence, tool_calls, tool_errors)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1.0, ?13, NULL)",
                    params![
                        prompt_pk, device_id, session_pk, source, project_id,
                        row.prompt_text, row.started_at.as_deref().and_then(iso_to_secs),
                        row.duration_ms, row.requested_action, row.target_object,
                        row.interaction_state, row.interaction_mode, row.tool_calls,
                    ],
                )?;
                summary.prompts += 1;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                tx.commit()?;
                self.invalidate_window_cache();
                Ok(summary)
            }
            Err(e) => {
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    /// PRD-08 §3.8 (P2): convenience wrapper that reads the external digest.db
    /// and imports it. Prefer using `read_digest_db` + `import_digest_data` in
    /// commands so the external read does not hold `Mutex<Db>`.
    pub fn import_from_digest_db(&mut self, device_id: &str) -> Result<DigestImportSummary> {
        let data = Self::read_digest_db()?;
        self.import_digest_data(device_id, data)
    }

    /// PRD-08 §3.6c (P2): the newest `cached_at` across collected tables, used to
    /// decide whether a cached window result is still valid.
    pub fn latest_data_through(&self) -> i64 {
        let cs: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(cached_at),0) FROM collected_sessions", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        cs
    }

    /// PRD-08 §3.6c (P2): read a cached window payload. Returns None if absent
    /// or stale (`data_through` older than the current newest data).
    pub fn get_cached_window_metrics(&self, cache_key: &str) -> Option<String> {
        let fresh_as_of = self.latest_data_through();
        let row = self.conn.query_row(
            "SELECT payload, data_through FROM analysis_window_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
        );
        match row {
            Ok((payload, data_through)) => {
                // Valid only if data_through >= the newest collected row.
                if data_through.unwrap_or(0) >= fresh_as_of {
                    Some(payload)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// PRD-08 §3.6c (P2): store a computed window payload, stamping it with the
    /// current newest-data timestamp so future freshness checks work.
    pub fn set_cached_window_metrics(&self, cache_key: &str, payload: &str) -> Result<()> {
        let now = now_secs() as i64;
        let data_through = self.latest_data_through();
        self.conn.execute(
            "INSERT OR REPLACE INTO analysis_window_cache (cache_key, payload, computed_at, data_through)
             VALUES (?1, ?2, ?3, ?4)",
            params![cache_key, payload, now, data_through],
        )?;
        Ok(())
    }

    /// PRD-08 §3.6c (P2): drop all cached window results (e.g. after an import
    /// that changes the data set). Best-effort.
    pub fn invalidate_window_cache(&self) {
        let _ = self.conn.execute("DELETE FROM analysis_window_cache", []);
    }

    // Skills
    pub fn insert_skill(&self, skill: &Skill) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO skills (id, name, repo_path, created_at, updated_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![skill.id, skill.name, skill.repo_path.to_string_lossy().to_string(), skill.created_at, skill.updated_at, skill.status.to_string()],
        )?;
        Ok(())
    }

    pub fn get_skills(&self) -> Result<Vec<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete_skill(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sync_targets WHERE skill_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM skills WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_skill_by_name(&self, name: &str) -> Result<Option<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills WHERE name = ?1",
        )?;
        let mut rows = stmt.query_map(params![name], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get a skill by its primary key id.
    pub fn get_skill_by_id(&self, id: &str) -> Result<Option<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-09: update skill metadata after an in-app edit (name/path/timestamp).
    pub fn update_skill(&self, skill: &Skill) -> Result<()> {
        self.conn.execute(
            "UPDATE skills SET name = ?1, repo_path = ?2, updated_at = ?3, status = ?4 WHERE id = ?5",
            params![
                skill.name,
                skill.repo_path.to_string_lossy().to_string(),
                skill.updated_at,
                skill.status.to_string(),
                skill.id
            ],
        )?;
        Ok(())
    }

    /// P1-3: update only the lifecycle status of a skill.
    pub fn update_skill_status(&self, id: &str, status: SkillStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE skills SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.to_string(), now_secs(), id],
        )?;
        Ok(())
    }

    /// P1-4: rename a skill in ONE transaction — the row id is preserved so
    /// FK references (kg_skill_nodes, sync_targets) stay intact. The
    /// skill_name shown on sync targets comes from a JOIN with skills.name,
    /// so this single UPDATE refreshes every display automatically.
    pub fn rename_skill_tx(
        &self,
        skill_id: &str,
        new_name: &str,
        new_repo_path: &std::path::Path,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE skills SET name = ?1, repo_path = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                new_name,
                new_repo_path.to_string_lossy().to_string(),
                now_secs(),
                skill_id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Agents
    pub fn insert_agent(&self, agent: &Agent) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO agents (id, name, skill_directory, is_enabled, discovery_rule, description, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![agent.id, agent.name, agent.skill_directory.to_string_lossy().to_string(), agent.is_enabled as i32, agent.discovery_rule, agent.description, agent.source],
        )?;
        Ok(())
    }

    pub fn get_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.conn.prepare("SELECT id, name, skill_directory, is_enabled, discovery_rule, description, source FROM agents ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(Agent {
                id: row.get(0)?,
                name: row.get(1)?,
                skill_directory: PathBuf::from(row.get::<_, String>(2)?),
                is_enabled: row.get::<_, i32>(3)? != 0,
                discovery_rule: row.get(4)?,
                description: row.get(5)?,
                source: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-06 §5.2: insert an agent directory sub-record.
    pub fn insert_agent_directory(&self, dir: &AgentDirectory) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_directories (id, agent_id, path, role, is_enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![dir.id, dir.agent_id, dir.path.to_string_lossy().to_string(), dir.role, dir.is_enabled as i32, dir.created_at],
        )?;
        Ok(())
    }

    /// PRD-06 §5.2: list all directories owned by an agent.
    pub fn get_agent_directories(&self, agent_id: &str) -> Result<Vec<AgentDirectory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, path, role, is_enabled, created_at FROM agent_directories WHERE agent_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(AgentDirectory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                role: row.get(3)?,
                is_enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Look up a single agent directory by id.
    pub fn get_agent_directory_by_id(&self, directory_id: &str) -> Result<Option<AgentDirectory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, path, role, is_enabled, created_at FROM agent_directories WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![directory_id], |row| {
            Ok(AgentDirectory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                role: row.get(3)?,
                is_enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-06 §3.3: delete an agent directory by id. Caller owns any disk-side
    /// decisions (we only remove the row + its sync_targets here, never the files).
    /// Returns the count of sync_targets orphaned by the deletion, so the caller
    /// can warn the user / offer cleanup.
    pub fn delete_agent_directory(&self, directory_id: &str) -> Result<usize> {
        let orphaned = self.conn.execute(
            "DELETE FROM sync_targets WHERE agent_directory_id = ?1",
            params![directory_id],
        )?;
        let deleted = self.conn.execute(
            "DELETE FROM agent_directories WHERE id = ?1",
            params![directory_id],
        )?;
        if deleted == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(orphaned)
    }

    /// PRD-06 §3.3: rename the role label of an agent directory (free-form string).
    pub fn update_agent_directory_role(
        &self,
        directory_id: &str,
        role: Option<&str>,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE agent_directories SET role = ?1 WHERE id = ?2",
            params![role, directory_id],
        )?;
        if updated == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(())
    }

    /// PRD-06 §3.3: toggle whether a directory participates in scan/sync.
    pub fn update_agent_directory_enabled(
        &self,
        directory_id: &str,
        is_enabled: bool,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE agent_directories SET is_enabled = ?1 WHERE id = ?2",
            params![is_enabled as i32, directory_id],
        )?;
        if updated == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(())
    }

    /// Read cached skill scan results for one agent directory.
    pub fn get_directory_skills(
        &self,
        agent_id: &str,
        path: &std::path::Path,
    ) -> Result<Vec<AgentSkillItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, exists_in_center, content_match FROM agent_directory_skills
             WHERE agent_id = ?1 AND path = ?2 ORDER BY name",
        )?;
        let rows = stmt.query_map(
            params![agent_id, path.to_string_lossy().to_string()],
            |row| {
                Ok(AgentSkillItem {
                    name: row.get(0)?,
                    exists_in_center: row.get::<_, i64>(1)? != 0,
                    content_match: row.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Persist skill scan results for one agent directory, replacing any prior rows.
    pub fn replace_directory_skills(
        &mut self,
        agent_id: &str,
        path: &std::path::Path,
        items: &[AgentSkillItem],
        scanned_at: u64,
    ) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
            params![agent_id, &path_str],
        )?;
        let mut stmt = tx.prepare(
            "INSERT INTO agent_directory_skills
             (agent_id, path, name, exists_in_center, content_match, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for item in items {
            stmt.execute(params![
                agent_id,
                &path_str,
                &item.name,
                item.exists_in_center as i32,
                item.content_match.map(|v| v as i32),
                scanned_at as i64,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    /// Drop cached scan results for one agent directory.
    pub fn delete_directory_skills(
        &self,
        agent_id: &str,
        path: &std::path::Path,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
            params![agent_id, path.to_string_lossy().to_string()],
        )?;
        Ok(())
    }

    /// Drop cached scan results for an entire agent (used when the agent is removed/rescanned).
    pub fn delete_agent_directory_skills(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1",
            params![agent_id],
        )?;
        Ok(())
    }

    /// Drop all cached directory-skill scan results. Infrequent: used after global
    /// sync or center-repo changes that could affect every agent's content_match.
    pub fn clear_directory_skill_cache(&self) -> Result<()> {
        self.conn.execute("DELETE FROM agent_directory_skills", [])?;
        Ok(())
    }

    /// Batch count of cached skills per agent, scoped to each agent's primary skill_directory.
    pub fn count_agent_directory_skills(
        &self,
        agent_id_to_path: &[(String, PathBuf)],
    ) -> Result<HashMap<String, usize>> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        if agent_id_to_path.is_empty() {
            return Ok(counts);
        }
        // SQLite does not support tuple IN, so query per agent. The batch is typically small
        // (tens of agents) and this keeps the query simple and indexed.
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*) FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
        )?;
        for (agent_id, path) in agent_id_to_path {
            let n: i64 = stmt.query_row(
                params![agent_id, path.to_string_lossy().to_string()],
                |row| row.get(0),
            )?;
            counts.insert(agent_id.clone(), n as usize);
        }
        Ok(counts)
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sync_targets WHERE agent_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM agents WHERE id = ?1", params![id])?;
        Ok(())
    }

    // Sync targets
    pub fn insert_sync_target(&self, target: &SyncTarget) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_targets (id, skill_id, agent_id, mode, last_sync_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                target.id,
                target.skill_id,
                target.agent_id,
                target.mode.to_string(),
                target.last_sync_at,
                target.status.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn get_sync_targets(&self) -> Result<Vec<SyncTarget>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT 
                t.id, t.skill_id, s.name as skill_name, t.agent_id, a.name as agent_name,
                t.mode, t.last_sync_at, t.status
            FROM sync_targets t
            JOIN skills s ON t.skill_id = s.id
            JOIN agents a ON t.agent_id = a.id
            ORDER BY s.name, a.name"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let mode_str: String = row.get(5)?;
            let status_str: String = row.get(7)?;
            Ok(SyncTarget {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                agent_id: row.get(3)?,
                agent_name: row.get(4)?,
                mode: parse_sync_mode(&mode_str),
                last_sync_at: row.get(6)?,
                status: parse_sync_status(&status_str),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_sync_target_status(&self, id: &str, status: SyncStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_targets SET status = ?1 WHERE id = ?2",
            params![status.to_string(), id],
        )?;
        Ok(())
    }

    /// Update both status and last_sync_at after a successful sync operation.
    pub fn update_sync_target_synced(&self, id: &str, status: SyncStatus, last_sync_at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_targets SET status = ?1, last_sync_at = ?2 WHERE id = ?3",
            params![status.to_string(), last_sync_at, id],
        )?;
        Ok(())
    }

    pub fn get_sync_target_by_id(&self, id: &str) -> Result<Option<SyncTarget>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT 
                t.id, t.skill_id, s.name as skill_name, t.agent_id, a.name as agent_name,
                t.mode, t.last_sync_at, t.status
            FROM sync_targets t
            JOIN skills s ON t.skill_id = s.id
            JOIN agents a ON t.agent_id = a.id
            WHERE t.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let mode_str: String = row.get(5)?;
            let status_str: String = row.get(7)?;
            Ok(SyncTarget {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                agent_id: row.get(3)?,
                agent_name: row.get(4)?,
                mode: parse_sync_mode(&mode_str),
                last_sync_at: row.get(6)?,
                status: parse_sync_status(&status_str),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    // ----- PRD-02: usage data collection -------------------------------------

    pub fn upsert_collected_source(&self, src: &CollectedSource) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_sources
               (source, collector_kind, data_path, status, last_collected_at, record_count)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                src.source,
                src.collector_kind,
                src.data_path,
                src.status,
                src.last_collected_at,
                src.record_count,
            ],
        )?;
        Ok(())
    }

    pub fn get_collected_sources(&self) -> Result<Vec<CollectedSource>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source, collector_kind, data_path, status, last_collected_at, record_count
               FROM collected_sources ORDER BY source"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CollectedSource {
                source: row.get(0)?,
                collector_kind: row.get(1)?,
                data_path: row.get(2)?,
                status: row.get(3)?,
                last_collected_at: row.get(4)?,
                record_count: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ----- Incremental collection file states --------------------------------

    pub fn get_collected_file_state(
        &self,
        source: &str,
        file_path: &str,
    ) -> Result<Option<CollectedFileState>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source, file_path, last_modified_ns, last_size, last_collected_at, content_hash
               FROM collector_file_states WHERE source = ?1 AND file_path = ?2"#,
        )?;
        let mut rows = stmt.query_map(params![source, file_path], |row| {
            Ok(CollectedFileState {
                source: row.get(0)?,
                file_path: row.get(1)?,
                last_modified_ns: row.get(2)?,
                last_size: row.get(3)?,
                last_collected_at: row.get(4)?,
                content_hash: row.get(5)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn upsert_collected_file_state(&self, state: &CollectedFileState) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collector_file_states
               (source, file_path, last_modified_ns, last_size, last_collected_at, content_hash)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                state.source,
                state.file_path,
                state.last_modified_ns,
                state.last_size,
                state.last_collected_at,
                state.content_hash,
            ],
        )?;
        Ok(())
    }

    pub fn list_collected_file_states(&self, source: &str) -> Result<Vec<CollectedFileState>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source, file_path, last_modified_ns, last_size, last_collected_at, content_hash
               FROM collector_file_states WHERE source = ?1"#,
        )?;
        let rows = stmt.query_map(params![source], |row| {
            Ok(CollectedFileState {
                source: row.get(0)?,
                file_path: row.get(1)?,
                last_modified_ns: row.get(2)?,
                last_size: row.get(3)?,
                last_collected_at: row.get(4)?,
                content_hash: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete_collected_file_state(&self, source: &str, file_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM collector_file_states WHERE source = ?1 AND file_path = ?2",
            params![source, file_path],
        )?;
        Ok(())
    }

    // ----- Background collection jobs ----------------------------------------

    pub fn create_collection_job(&self, id: &str, started_at: u64) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO collection_jobs (id, started_at, status, progress_json)
               VALUES (?1, ?2, 'running', '{}')"#,
            params![id, started_at as i64],
        )?;
        Ok(())
    }

    pub fn update_collection_job_progress(
        &self,
        id: &str,
        progress_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE collection_jobs SET progress_json = ?1 WHERE id = ?2",
            params![progress_json, id],
        )?;
        Ok(())
    }

    pub fn complete_collection_job(
        &self,
        id: &str,
        completed_at: u64,
        result_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            r#"UPDATE collection_jobs
               SET status = 'completed', completed_at = ?1, result_json = ?2
               WHERE id = ?3"#,
            params![completed_at as i64, result_json, id],
        )?;
        Ok(())
    }

    pub fn fail_collection_job(&self, id: &str, completed_at: u64, error: &str) -> Result<()> {
        self.conn.execute(
            r#"UPDATE collection_jobs
               SET status = 'failed', completed_at = ?1, error = ?2
               WHERE id = ?3"#,
            params![completed_at as i64, error, id],
        )?;
        Ok(())
    }

    pub fn cancel_collection_job(&self, id: &str, completed_at: u64) -> Result<()> {
        self.conn.execute(
            r#"UPDATE collection_jobs
               SET status = 'cancelled', completed_at = ?1
               WHERE id = ?2"#,
            params![completed_at as i64, id],
        )?;
        Ok(())
    }

    pub fn get_collection_job(&self, id: &str) -> Result<Option<CollectionJob>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, started_at, completed_at, status, progress_json, result_json, error
               FROM collection_jobs WHERE id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(CollectionJob {
                id: row.get(0)?,
                started_at: row.get::<_, i64>(1)? as u64,
                completed_at: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                status: row.get(3)?,
                progress_json: row.get(4)?,
                result_json: row.get(5)?,
                error: row.get(6)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_recent_collection_jobs(&self, limit: usize) -> Result<Vec<CollectionJob>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, started_at, completed_at, status, progress_json, result_json, error
               FROM collection_jobs
               ORDER BY started_at DESC
               LIMIT ?1"#,
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(CollectionJob {
                id: row.get(0)?,
                started_at: row.get::<_, i64>(1)? as u64,
                completed_at: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                status: row.get(3)?,
                progress_json: row.get(4)?,
                result_json: row.get(5)?,
                error: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// SPEC-F3: write an install audit row.
    pub fn log_install_audit(
        &self,
        id: &str,
        skill_name: &str,
        source: &str,
        findings: &[crate::models::SafetyFinding],
        confirmed_risks: &[String],
        confirmed: bool,
        created_at: u64,
    ) -> Result<()> {
        let findings_json = serde_json::to_string(findings).unwrap_or_else(|_| "[]".to_string());
        let confirmed_json = serde_json::to_string(confirmed_risks).unwrap_or_else(|_| "[]".to_string());
        self.conn.execute(
            r#"INSERT INTO install_audit (id, skill_name, source, findings_json, confirmed_risks_json, confirmed, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                id,
                skill_name,
                source,
                findings_json,
                confirmed_json,
                confirmed as i32,
                created_at as i64,
            ],
        )?;
        Ok(())
    }

    /// SPEC-F3: count install audit rows (test helper).
    pub fn count_install_audit(&self, skill_name: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM install_audit WHERE skill_name = ?1",
            [skill_name],
            |row| row.get(0),
        )?)
    }

    pub fn upsert_collected_session(&self, s: &CollectedSession) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_sessions
               (id, device_id, source, project_id, agent_id, start_time, end_time,
                message_count, title_or_prompt, cached_at, project_path, quality_score)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                s.id,
                s.device_id,
                s.source,
                s.project_id,
                s.agent_id,
                s.start_time,
                s.end_time,
                s.message_count,
                s.title_or_prompt,
                s.cached_at,
                s.project_path,
                s.quality_score,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_collected_prompt(&self, p: &CollectedPrompt) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_prompts
               (id, device_id, session_id, source, project_id, prompt_text, started_at,
                duration_ms, requested_action, target_object, interaction_state,
                interaction_mode, confidence, tool_calls, tool_errors)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"#,
            params![
                p.id,
                p.device_id,
                p.session_id,
                p.source,
                p.project_id,
                p.prompt_text,
                p.started_at,
                p.duration_ms,
                p.requested_action,
                p.target_object,
                p.interaction_state,
                p.interaction_mode,
                p.confidence,
                p.tool_calls,
                p.tool_errors,
            ],
        )?;
        Ok(())
    }

    /// Upsert token usage keyed by (device, source, session, model). Accumulates across
    /// re-collects by summing into the existing row only if the incoming totals are larger
    /// (prevents double counting when a file is partially re-read). Simplest correct approach:
    /// INSERT OR REPLACE with the freshly computed per-(session,model) totals.
    pub fn upsert_collected_token_usage(&self, t: &CollectedTokenUsage) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_token_usage
               (id, device_id, session_id, source, project_id, model_id, input_tokens,
                output_tokens, reasoning_tokens, cache_creation_input_tokens,
                cache_read_input_tokens, total_tokens, model_calls, tool_calls, duration_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"#,
            params![
                t.id,
                t.device_id,
                t.session_id,
                t.source,
                t.project_id,
                t.model_id,
                t.input_tokens,
                t.output_tokens,
                t.reasoning_tokens,
                t.cache_creation_input_tokens,
                t.cache_read_input_tokens,
                t.total_tokens,
                t.model_calls,
                t.tool_calls,
                t.duration_ms,
            ],
        )?;
        Ok(())
    }

    /// PRD-05 §5.2: upsert one Cursor code-contribution row (per scored commit).
    pub fn upsert_collected_code_contribution(&self, c: &CollectedCodeContribution) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_code_contributions
               (id, device_id, source, project_id, commit_hash, branch_name, commit_date,
                scored_at, lines_added, lines_deleted, composer_lines_added,
                composer_lines_deleted, human_lines_added, human_lines_deleted,
                tab_lines_added, ai_percentage, commit_message, cached_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"#,
            params![
                c.id,
                c.device_id,
                c.source,
                c.project_id,
                c.commit_hash,
                c.branch_name,
                c.commit_date,
                c.scored_at.map(|t| t as i64),
                c.lines_added,
                c.lines_deleted,
                c.composer_lines_added,
                c.composer_lines_deleted,
                c.human_lines_added,
                c.human_lines_deleted,
                c.tab_lines_added,
                c.ai_percentage,
                c.commit_message,
                c.cached_at as i64,
            ],
        )?;
        Ok(())
    }

    /// PRD-05 §5.2: count code-contribution rows for a source (record_count for cursor).
    pub fn count_collected_code_contributions(&self, source: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM collected_code_contributions WHERE source = ?1",
            params![source],
            |row| row.get(0),
        )?)
    }

    /// Count rows collected for a given source (for record_count in collected_sources).
    pub fn count_collected_sessions(&self, source: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM collected_sessions WHERE source = ?1",
            params![source],
            |row| row.get(0),
        )?)
    }

    // ----- PRD-01/02: project discovery + linking ----------------------------

    /// Get or create a project row for the given path, returning its id.
    /// Path is normalized (canonicalized) before lookup. Name is derived from the
    /// last path segment.
    pub fn ensure_project_by_path(&self, device_id: &str, raw_path: &str) -> Result<Option<String>> {
        Self::ensure_project_by_path_conn(&self.conn, device_id, raw_path)
    }

    /// Transaction-aware core of `ensure_project_by_path`. Lets import/write
    /// paths run the project lookup/insert on the same connection (including a
    /// transaction) to avoid split-brain inserts and partial rollbacks.
    fn ensure_project_by_path_conn(
        conn: &rusqlite::Connection,
        device_id: &str,
        raw_path: &str,
    ) -> Result<Option<String>> {
        let normalized = normalize_path(raw_path);
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

    /// Link collected sessions that carry a `project_path` to the projects table,
    /// and upsert agent_instances counts. Returns the number of sessions linked.
    pub fn link_sessions_to_projects(&self, device_id: &str) -> Result<i64> {
        // Read all (session_id, source, project_path, start_time) that are unlinked.
        let mut stmt = self.conn.prepare(
            "SELECT id, source, project_path, start_time FROM collected_sessions
             WHERE project_path IS NOT NULL AND project_path <> ''",
        )?;
        let rows: Vec<(String, String, String, Option<u64>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .map(|(id, src, path, ts)| (id, src, path, ts.map(|t| t as u64)))
            .collect();
        drop(stmt);

        let mut linked = 0i64;
        for (session_id, source, path, start_ts) in rows {
            let project_id = match self.ensure_project_by_path(device_id, &path)? {
                Some(p) => p,
                None => continue,
            };
            // Update the session's project_id.
            self.conn.execute(
                "UPDATE collected_sessions SET project_id = ?1 WHERE id = ?2",
                params![project_id, session_id],
            )?;
            // Update token_usage rows for this session to carry the project_id too.
            self.conn.execute(
                "UPDATE collected_token_usage SET project_id = ?1 WHERE session_id = ?2",
                params![project_id, session_id],
            )?;
            // PRD-06 §3.2: attribute the session to an Agent via `agents.source`
            // (deterministic equality), replacing the old `derive_agent_id` hack.
            // If no Agent carries this source, skip attribution but still link the project.
            let agent_key: Option<String> = self
                .conn
                .query_row(
                    "SELECT id FROM agents WHERE source = ?1 LIMIT 1",
                    params![&source],
                    |row| row.get(0),
                )
                .ok();
            if let Some(agent_key) = agent_key {
                let ai_id = format!("{device_id}:{agent_key}:{project_id}");
                let now = now_secs();
                self.conn.execute(
                    r#"INSERT INTO agent_instances
                       (id, device_id, agent_id, project_id, last_session_at, session_count,
                        total_tokens, total_prompts, created_at, updated_at)
                       VALUES (?1, ?2, ?3, ?4, ?5, 1, 0, 0, ?6, ?7)
                       ON CONFLICT(device_id, agent_id, project_id) DO UPDATE SET
                         last_session_at = MAX(COALESCE(excluded.last_session_at, last_session_at), COALESCE(last_session_at, 0)),
                         session_count = agent_instances.session_count + 1,
                         updated_at = excluded.updated_at"#,
                    params![ai_id, device_id, agent_key, project_id, start_ts.unwrap_or(now), now, now],
                )?;
            }
            linked += 1;
        }

        // Populate agents.last_used_at and project_count from agent_instances (PRD-01 §5.1).
        let now = now_secs();
        self.conn.execute(
            r#"UPDATE agents SET last_used_at = (
                  SELECT MAX(ai.last_session_at) FROM agent_instances ai
                  WHERE ai.agent_id = agents.id
               ),
               project_count = (
                  SELECT COUNT(DISTINCT ai.project_id) FROM agent_instances ai
                  WHERE ai.agent_id = agents.id
               ),
               updated_at = ?1
               WHERE EXISTS (SELECT 1 FROM agent_instances ai WHERE ai.agent_id = agents.id)"#,
            params![now],
        )?;
        Ok(linked)
    }

    /// Look up a skill id by name (case-insensitive). Returns None if no such skill.
    pub fn skill_id_by_name(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM skills WHERE lower(name) = lower(?1)",
                params![name],
                |row| row.get(0),
            )
            .ok())
    }

    // ----- PRD-02: skill usage attribution -----------------------------------

    pub fn upsert_skill_attribution(&self, a: &SkillUsageAttribution) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO skill_usage_attributions
               (id, device_id, skill_id, skill_name, source, session_id, project_id,
                agent_id, attribution_type, attributed_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                a.id,
                a.device_id,
                a.skill_id,
                a.skill_name,
                a.source,
                a.session_id,
                a.project_id,
                a.agent_id,
                a.attribution_type,
                a.attributed_at,
            ],
        )?;
        Ok(())
    }

    /// Aggregate skill usage over the last N days (days==0 => all time).
    pub fn get_skill_usage_summary(&self, days: u32, now: u64) -> Result<Vec<SkillUsageSummary>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT skill_name,
                      MAX(skill_id),
                      COUNT(*) AS usage_count,
                      COUNT(DISTINCT session_id) AS session_count,
                      COUNT(DISTINCT project_id) AS project_count
               FROM skill_usage_attributions
               WHERE IFNULL(attributed_at, 0) >= ?1
               GROUP BY skill_name
               ORDER BY usage_count DESC"#,
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok(SkillUsageSummary {
                skill_name: row.get(0)?,
                skill_id: row.get(1)?,
                usage_count: row.get(2)?,
                session_count: row.get(3)?,
                project_count: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-09 §3.3c: per-skill discovery metrics used to rank the Discover page.
    ///
    /// Returns a map keyed by lowercased skill name. `usage_count` comes from
    /// `skill_usage_attributions`; `correction_count` counts prompts in the
    /// same sessions whose `interaction_state` is "补充澄清" or "纠偏修正".
    /// Days==0 means all time.
    pub fn get_skill_discovery_metrics(
        &self,
        days: u32,
        now: u64,
    ) -> Result<std::collections::HashMap<String, (i64, i64)>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT
                 a.skill_name,
                 COUNT(*) AS usage_count,
                 COALESCE(SUM(CASE
                   WHEN p.interaction_state IN ('补充澄清', '纠偏修正') THEN 1
                   ELSE 0
                 END), 0) AS correction_count
               FROM skill_usage_attributions a
               LEFT JOIN collected_prompts p
                 ON a.device_id = p.device_id AND a.session_id = p.session_id
               WHERE IFNULL(a.attributed_at, 0) >= ?1
               GROUP BY a.skill_name"#,
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (name, usage, correction) = row?;
            map.insert(name.to_lowercase(), (usage, correction));
        }
        Ok(map)
    }

    /// PRD-02 §3.2: per-day timeseries of token usage for a source.
    /// Returns Vec<(date_string, token_count)> bucketed by day.
    pub fn get_usage_timeseries(&self, source: &str, days: u32) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT date(s.start_time, 'unixepoch', 'localtime') AS day,
                      SUM(COALESCE(tu.tokens, 0)) AS tokens
               FROM collected_sessions s
               LEFT JOIN (
                 SELECT session_id, SUM(total_tokens) AS tokens
                 FROM collected_token_usage
                 GROUP BY session_id
               ) tu ON tu.session_id = s.id
               WHERE s.source = ?1 AND s.start_time IS NOT NULL
               GROUP BY day
               ORDER BY day"#,
        )?;
        let rows = stmt.query_map(params![source], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let all: Vec<(String, i64)> = rows.filter_map(|r| r.ok()).collect();
        if days == 0 {
            return Ok(all);
        }
        // Trim to last N days.
        let len = all.len();
        let start = len.saturating_sub(days as usize);
        Ok(all[start..].to_vec())
    }

    /// PRD-02 §3.3c: skill health score (0-100) based on usage, coverage, and correction rate.
    /// Returns (score, suggestion) for a single skill.
    pub fn get_skill_health(&self, skill_name: &str, days: u32, now: u64) -> Result<(f64, String)> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let usage: Option<(i64, i64, i64)> = self.conn.query_row(
            r#"SELECT COUNT(*), COUNT(DISTINCT session_id), COUNT(DISTINCT project_id)
               FROM skill_usage_attributions
               WHERE skill_name = ?1 AND IFNULL(attributed_at, 0) >= ?2"#,
            params![skill_name, cutoff],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).ok();
        let (count, sessions, projects) = usage.unwrap_or((0, 0, 0));

        // Health = usage density (40pts) + coverage breadth (40pts) + recency bonus (20pts).
        let density = (count as f64 * 5.0).min(40.0);
        let coverage = ((sessions as f64 * 4.0) + (projects as f64 * 8.0)).min(40.0);
        let recency = if count > 0 { 20.0 } else { 0.0 };
        let score = (density + coverage + recency).round();

        let suggestion = if score < 30.0 {
            format!("「{}」使用率低，建议检查描述是否清晰或是否已被弃用。", skill_name)
        } else if score < 60.0 && projects <= 1 {
            format!("「{}」覆盖项目少（{}），考虑推广到更多项目。", skill_name, projects)
        } else {
            String::new()
        };
        Ok((score, suggestion))
    }

    /// Per-project usage over the last N days (project profile).
    pub fn get_project_usage_summary(
        &self,
        device_id: &str,
        days: u32,
        now: u64,
    ) -> Result<Vec<ProjectUsageSummary>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT p.id, p.name, p.path,
                      COUNT(DISTINCT s.id) AS session_count,
                      COALESCE(SUM(t.total_tokens), 0) AS total_tokens
               FROM projects p
               LEFT JOIN collected_sessions s
                 ON s.project_id = p.id AND IFNULL(s.start_time, 0) >= ?1
               LEFT JOIN collected_token_usage t
                 ON t.session_id = s.id
               WHERE p.device_id = ?2
               GROUP BY p.id
               ORDER BY total_tokens DESC"#,
        )?;
        let rows = stmt.query_map(params![cutoff, device_id], |row| {
            Ok(ProjectUsageSummary {
                project_id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                session_count: row.get(3)?,
                total_tokens: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Aggregate usage for a source over the last `days` days (P0 data-tab payload).
    /// `days == 0` means "all time".
    pub fn get_agent_usage_summary(
        &self,
        source: &str,
        days: u32,
        now: u64,
    ) -> Result<AgentUsageSummary> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };

        let session_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collected_sessions WHERE source = ?1 AND IFNULL(start_time, 0) >= ?2",
            params![source, cutoff],
            |row| row.get(0),
        )?;
        let prompt_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collected_prompts WHERE source = ?1 AND IFNULL(started_at, 0) >= ?2",
            params![source, cutoff],
            |row| row.get(0),
        )?;
        let totals = self.conn.query_row(
            r#"SELECT
                  COALESCE(SUM(input_tokens), 0),
                  COALESCE(SUM(output_tokens), 0),
                  COALESCE(SUM(cache_read_input_tokens), 0),
                  COALESCE(SUM(cache_creation_input_tokens), 0),
                  COALESCE(SUM(total_tokens), 0)
               FROM collected_token_usage
               WHERE source = ?1
                 AND session_id IN (SELECT id FROM collected_sessions WHERE IFNULL(start_time,0) >= ?2)"#,
            params![source, cutoff],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?, // input
                    row.get::<_, i64>(1)?, // output
                    row.get::<_, i64>(2)?, // cache_read
                    row.get::<_, i64>(3)?, // cache_creation
                    row.get::<_, i64>(4)?, // total
                ))
            },
        )?;

        Ok(AgentUsageSummary {
            source: source.to_string(),
            days,
            session_count,
            prompt_count,
            input_tokens: totals.0,
            output_tokens: totals.1,
            cache_read_input_tokens: totals.2,
            cache_creation_input_tokens: totals.3,
            total_tokens: totals.4,
        })
    }

    // ----- SPEC-I2: discovery inbox + weekly reports -------------------------

    /// Insert or skip discovery rows. For each candidate, if a row with the same
    /// dedup_key already exists in `pending` status, skip it. Otherwise insert as
    /// pending. Returns the number actually inserted.
    pub fn insert_discoveries(&mut self, discoveries: &[crate::models::Discovery]) -> Result<usize> {
        let mut inserted = 0usize;
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO discoveries
                 (id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                 WHERE NOT EXISTS (
                     SELECT 1 FROM discoveries WHERE dedup_key = ?6 AND status = 'pending'
                 )",
            )?;
            for d in discoveries {
                let rows = stmt.execute(params![
                    d.id,
                    d.kind.to_string(),
                    d.title,
                    d.payload.to_string(),
                    d.confidence,
                    d.dedup_key,
                    d.status.to_string(),
                    d.created_at as i64,
                    d.decided_at.map(|t| t as i64),
                    d.resulting_skill_id,
                ])?;
                inserted += rows;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// List discoveries filtered by status (empty string = all).
    pub fn list_discoveries(&self, status: Option<&str>) -> Result<Vec<crate::models::Discovery>> {
        match status {
            None | Some("") => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
                     FROM discoveries ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map([], map_discovery_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
            Some(s) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
                     FROM discoveries WHERE status = ?1 ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(params![s], map_discovery_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
        }
    }

    /// Update a discovery's status, decided_at, and optional resulting_skill_id.
    pub fn update_discovery_status(
        &self,
        id: &str,
        status: crate::models::DiscoveryStatus,
        decided_at: Option<u64>,
        resulting_skill_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE discoveries
             SET status = ?1, decided_at = ?2, resulting_skill_id = ?3
             WHERE id = ?4",
            params![
                status.to_string(),
                decided_at.map(|t| t as i64),
                resulting_skill_id,
                id,
            ],
        )?;
        Ok(())
    }

    /// Get a single discovery by id.
    pub fn get_discovery_by_id(&self, id: &str) -> Result<Option<crate::models::Discovery>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
             FROM discoveries WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_discovery_row)?;
        Ok(rows.next().transpose()?)
    }

    /// True if the dedup_key was dismissed within `days`.
    pub fn is_dedup_cooling(&self, dedup_key: &str, days: u32) -> Result<bool> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM discoveries
             WHERE dedup_key = ?1 AND status = 'dismissed' AND decided_at >= ?2",
            params![dedup_key, cutoff],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Mark pending discoveries older than `days` as expired.
    pub fn expire_stale_discoveries(&self, days: u32) -> Result<usize> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let updated = self.conn.execute(
            "UPDATE discoveries
             SET status = 'expired'
             WHERE status = 'pending' AND created_at < ?1",
            params![cutoff],
        )?;
        Ok(updated)
    }

    // ----- SPEC-C1: adoption_events ------------------------------------------

    /// Record an adoption decision. Called inside the same logical step as the
    /// discoveries status update so the ledger is always consistent with the
    /// inbox (callers wrap both in a transaction when they need atomicity).
    pub fn insert_adoption_event(
        &self,
        discovery_id: &str,
        decision: &str,
        reject_reason: Option<&str>,
        resulting_skill_id: Option<&str>,
        decided_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO adoption_events
             (discovery_id, decision, reject_reason, resulting_skill_id, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                discovery_id,
                decision,
                reject_reason,
                resulting_skill_id,
                decided_at as i64,
            ],
        )?;
        Ok(())
    }

    /// Most recent adoption_event for a discovery (by id), if any. Used to pick
    /// the cooling tier after a rejection.
    pub fn latest_adoption_event(&self, discovery_id: &str) -> Result<Option<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT decision, reject_reason FROM adoption_events
             WHERE discovery_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![discovery_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Most recent `rejected` reason for a dedup_key. Drives the tiered cooling
    /// (SPEC-C1 T3): duplicate=90d, trivial=30d, wrong=14d. Returns None when
    /// there is no recorded rejection (historical dismissed items fall back to
    /// the legacy uniform window).
    pub fn latest_reject_reason_for_dedup(&self, dedup_key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT ae.reject_reason FROM adoption_events ae
             JOIN discoveries d ON d.id = ae.discovery_id
             WHERE d.dedup_key = ?1 AND ae.decision = 'rejected'
               AND ae.reject_reason IS NOT NULL
             ORDER BY ae.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![dedup_key], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    // ----- SPEC-C1: gate_rejections ------------------------------------------

    /// Record a gate rejection. The pipeline calls this at the three discard
    /// points (below threshold / daily limit / cooling) plus the rule gate.
    pub fn insert_gate_rejection(
        &self,
        reason: &str,
        kind: &str,
        confidence: f64,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let now = now_secs() as i64;
        self.conn.execute(
            "INSERT INTO gate_rejections (reason, kind, confidence, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![reason, kind, confidence, payload.to_string(), now],
        )?;
        Ok(())
    }

    /// List gate rejections, optionally filtered by reason. Newest first.
    pub fn list_gate_rejections(
        &self,
        reason: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::models::GateRejection>> {
        let sql = match reason {
            Some(r) if !r.is_empty() => "SELECT id, reason, kind, confidence, payload, created_at
                 FROM gate_rejections WHERE reason = ?1
                 ORDER BY created_at DESC LIMIT ?2",
            _ => "SELECT id, reason, kind, confidence, payload, created_at
                 FROM gate_rejections ORDER BY created_at DESC LIMIT ?1",
        };
        let map_row = |row: &rusqlite::Row| {
            let payload_json: String = row.get(4)?;
            let payload = serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::GateRejection {
                id: row.get(0)?,
                reason: row.get(1)?,
                kind: row.get(2)?,
                confidence: row.get(3)?,
                payload,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        };
        match reason {
            Some(r) if !r.is_empty() => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map(params![r, limit as i64], map_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
            _ => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map(params![limit as i64], map_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
        }
    }

    /// Delete gate rejections older than `days`. Called from the pipeline run
    /// so the table never grows unbounded (SPEC-C1 T4 cleanup).
    pub fn prune_gate_rejections(&self, days: u32) -> Result<usize> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let deleted = self.conn.execute(
            "DELETE FROM gate_rejections WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    // ----- SPEC-C3: trash_items ----------------------------------------------

    /// Insert a trash row pointing at the on-disk snapshot. Returns the row id
    /// so callers can delete the snapshot if a later step fails.
    pub fn insert_trash_item(
        &self,
        item_type: &str,
        original_id: &str,
        original_name: &str,
        snapshot_path: &str,
        metadata: &serde_json::Value,
        deleted_at: u64,
        expires_at: u64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO trash_items
             (item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                item_type,
                original_id,
                original_name,
                snapshot_path,
                metadata.to_string(),
                deleted_at as i64,
                expires_at as i64,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List all trash items, newest first.
    pub fn list_trash_items(&self) -> Result<Vec<crate::models::TrashItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items ORDER BY deleted_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Fetch a single trash item by id.
    pub fn get_trash_item(&self, id: i64) -> Result<Option<crate::models::TrashItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Delete a trash row by id. Does NOT touch the snapshot directory; callers
    /// are responsible for removing the on-disk snapshot.
    pub fn delete_trash_item(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM trash_items WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Return trash rows whose `expires_at` has passed (for startup cleanup).
    pub fn expired_trash_items(&self) -> Result<Vec<crate::models::TrashItem>> {
        let now = now_secs() as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items WHERE expires_at < ?1 ORDER BY expires_at ASC",
        )?;
        let rows = stmt.query_map(params![now], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Fetch raw sync_targets rows for a skill (used when snapshotting a skill
    /// for the trash bin). Returns serialized JSON so it survives a restore
    /// even if the schema evolves. Joins skills/agents for display names.
    pub fn raw_sync_targets_for_skill(&self, skill_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.skill_id, s.name, t.agent_id, a.name, t.mode, t.last_sync_at, t.status
             FROM sync_targets t
             LEFT JOIN skills s ON t.skill_id = s.id
             LEFT JOIN agents a ON t.agent_id = a.id
             WHERE t.skill_id = ?1",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "skill_id": row.get::<_, String>(1)?,
                "skill_name": row.get::<_, Option<String>>(2)?,
                "agent_id": row.get::<_, String>(3)?,
                "agent_name": row.get::<_, Option<String>>(4)?,
                "mode": row.get::<_, String>(5)?,
                "last_sync_at": row.get::<_, Option<i64>>(6)?,
                "status": row.get::<_, String>(7)?,
            }))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Fetch raw skill_project_bindings rows for a skill.
    pub fn raw_bindings_for_skill(&self, skill_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, skill_id, project_id, agent_id, mode, local_path, is_enabled, pinned_version, updated_at
             FROM skill_project_bindings WHERE skill_id = ?1",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "device_id": row.get::<_, String>(1)?,
                "skill_id": row.get::<_, String>(2)?,
                "project_id": row.get::<_, Option<String>>(3)?,
                "agent_id": row.get::<_, Option<String>>(4)?,
                "mode": row.get::<_, String>(5)?,
                "local_path": row.get::<_, Option<String>>(6)?,
                "is_enabled": row.get::<_, i64>(7)? != 0,
                "pinned_version": row.get::<_, Option<String>>(8)?,
                "updated_at": row.get::<_, i64>(9)?,
            }))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Upsert a weekly report.
    pub fn upsert_weekly_report(&self, report: &crate::models::WeeklyReport) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO weekly_reports
             (week_start, content, generated_at, model, provider)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                report.week_start,
                serde_json::to_string(&report.content)?,
                report.generated_at as i64,
                report.model,
                report.provider,
            ],
        )?;
        Ok(())
    }

    /// Load a weekly report by its Monday-start date.
    pub fn get_weekly_report(&self, week_start: &str) -> Result<Option<crate::models::WeeklyReport>> {
        let mut stmt = self.conn.prepare(
            "SELECT week_start, content, generated_at, model, provider
             FROM weekly_reports WHERE week_start = ?1",
        )?;
        let mut rows = stmt.query_map(params![week_start], |row| {
            let content_json: String = row.get(1)?;
            let content = serde_json::from_str(&content_json).unwrap_or_default();
            Ok(crate::models::WeeklyReport {
                week_start: row.get(0)?,
                content,
                generated_at: row.get::<_, i64>(2)? as u64,
                model: row.get(3)?,
                provider: row.get(4)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    // ----- PRD-02: high-value prompts + suggestion report --------------------

    /// Find high-value prompts (repeated >= threshold times) for skill sedimentation.
    pub fn get_high_value_prompts(&self, min_repeat: i64, limit: i64) -> Result<Vec<HighValuePrompt>> {
        // Group by normalized prompt text prefix; count occurrences.
        let mut stmt = self.conn.prepare(
            r#"SELECT substr(prompt_text, 1, 120) AS key,
                      source,
                      COUNT(*) AS cnt,
                      MAX(session_id) AS sample
               FROM collected_prompts
               WHERE prompt_text IS NOT NULL AND length(prompt_text) > 4
               GROUP BY key, source
               HAVING cnt >= ?1
               ORDER BY cnt DESC
               LIMIT ?2"#,
        )?;
        let rows = stmt.query_map(params![min_repeat, limit], |row| {
            Ok(HighValuePrompt {
                prompt_text: row.get::<_, String>(0)?,
                source: row.get::<_, Option<String>>(1)?,
                repeat_count: row.get(2)?,
                sample_session_id: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ----- PRD-01: skill-project bindings ------------------------------------

    pub fn upsert_skill_project_binding(&self, b: &SkillProjectBinding) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO skill_project_bindings
               (id, device_id, skill_id, project_id, agent_id, mode, local_path,
                is_enabled, created_at, updated_at, pinned_version)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                b.id,
                b.device_id,
                b.skill_id,
                b.project_id,
                b.agent_id,
                b.mode,
                b.local_path,
                b.is_enabled as i32,
                now_secs(),
                now_secs(),
                b.pinned_version,
            ],
        )?;
        Ok(())
    }

    pub fn get_skill_project_bindings(&self, device_id: &str) -> Result<Vec<SkillProjectBinding>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.device_id, b.skill_id, s.name, b.project_id, p.name,
                      b.agent_id, b.mode, b.local_path, b.is_enabled, b.pinned_version
               FROM skill_project_bindings b
               LEFT JOIN skills s ON b.skill_id = s.id
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.device_id = ?1
               ORDER BY s.name"#,
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok(SkillProjectBinding {
                id: row.get(0)?,
                device_id: row.get(1)?,
                skill_id: row.get(2)?,
                skill_name: row.get(3)?,
                project_id: row.get(4)?,
                project_name: row.get(5)?,
                agent_id: row.get(6)?,
                mode: row.get(7)?,
                local_path: row.get(8)?,
                is_enabled: row.get::<_, i64>(9)? != 0,
                pinned_version: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-F: look up a single binding by id (with joins for display names).
    pub fn get_skill_project_binding(&self, id: &str) -> Result<Option<SkillProjectBinding>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.device_id, b.skill_id, s.name, b.project_id, p.name,
                      b.agent_id, b.mode, b.local_path, b.is_enabled, b.pinned_version
               FROM skill_project_bindings b
               LEFT JOIN skills s ON b.skill_id = s.id
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SkillProjectBinding {
                id: row.get(0)?,
                device_id: row.get(1)?,
                skill_id: row.get(2)?,
                skill_name: row.get(3)?,
                project_id: row.get(4)?,
                project_name: row.get(5)?,
                agent_id: row.get(6)?,
                mode: row.get(7)?,
                local_path: row.get(8)?,
                is_enabled: row.get::<_, i64>(9)? != 0,
                pinned_version: row.get(10)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-01 patch FR-F: set the pinned version of a binding (None = follow latest).
    pub fn set_binding_pinned_version(&self, id: &str, version: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_project_bindings SET pinned_version = ?1, updated_at = ?2 WHERE id = ?3",
            params![version, now_secs(), id],
        )?;
        Ok(())
    }

    /// PRD-01 patch FR-F: which versions are pinned by any binding for a skill?
    pub fn get_pinned_versions_for_skill(&self, skill_id: &str) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT pinned_version FROM skill_project_bindings
             WHERE skill_id = ?1 AND pinned_version IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// PRD-01 patch FR-F §4.5d: for a given (skill, version), return the project
    /// names whose bindings pin to that version. Used to populate
    /// SkillVersion.pinned_by.
    pub fn get_pinned_projects_for_version(
        &self,
        skill_id: &str,
        version: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT p.name
               FROM skill_project_bindings b
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.skill_id = ?1 AND b.pinned_version = ?2"#,
        )?;
        let rows = stmt.query_map(params![skill_id, version], |row| {
            row.get::<_, Option<String>>(0)
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .flatten()
            .collect())
    }

    pub fn delete_skill_project_binding(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM skill_project_bindings WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ----- PRD-02 Phase 2: skill bundles --------------------------------------

    pub fn create_bundle(
        &self,
        device_id: &str,
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<SkillBundle> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO skill_bundles (id, device_id, name, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, device_id, name, description, now, now],
        )?;
        Ok(SkillBundle {
            id: id.to_string(),
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            skill_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn update_bundle(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<SkillBundle> {
        let now = now_secs();
        self.conn.execute(
            "UPDATE skill_bundles SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, description, now, id],
        )?;
        self.get_bundle(id)?.ok_or_else(|| anyhow::anyhow!("Bundle not found"))
    }

    pub fn delete_bundle(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM skill_bundles WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_bundle(&self, id: &str) -> Result<Option<SkillBundle>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                      (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) as skill_count
               FROM skill_bundles b
               WHERE b.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SkillBundle {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
                skill_count: row.get(5)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_bundles(&self, device_id: &str) -> Result<Vec<SkillBundle>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                      (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) as skill_count
               FROM skill_bundles b
               WHERE b.device_id = ?1
               ORDER BY b.updated_at DESC"#,
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok(SkillBundle {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
                skill_count: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_bundle_item(
        &self,
        device_id: &str,
        id: &str,
        bundle_id: &str,
        skill_id: &str,
        sort_order: i64,
    ) -> Result<SkillBundleItem> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO skill_bundle_items (id, device_id, bundle_id, skill_id, sort_order, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, device_id, bundle_id, skill_id, sort_order, now],
        )?;
        let skill_name = self
            .get_skill_by_id(skill_id)?
            .map(|s| s.name)
            .unwrap_or_default();
        Ok(SkillBundleItem {
            id: id.to_string(),
            skill_id: skill_id.to_string(),
            skill_name,
            sort_order,
        })
    }

    pub fn remove_bundle_item(&self, bundle_id: &str, skill_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skill_bundle_items WHERE bundle_id = ?1 AND skill_id = ?2",
            params![bundle_id, skill_id],
        )?;
        Ok(())
    }

    pub fn get_bundle_items(&self, bundle_id: &str) -> Result<Vec<SkillBundleItem>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT i.id, i.skill_id, COALESCE(s.name, i.skill_id), i.sort_order
               FROM skill_bundle_items i
               LEFT JOIN skills s ON i.skill_id = s.id
               WHERE i.bundle_id = ?1
               ORDER BY i.sort_order ASC, i.added_at ASC"#,
        )?;
        let rows = stmt.query_map(params![bundle_id], |row| {
            Ok(SkillBundleItem {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                sort_order: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_bundle_item_order(&self, item_id: &str, sort_order: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_bundle_items SET sort_order = ?1 WHERE id = ?2",
            params![sort_order, item_id],
        )?;
        Ok(())
    }

    pub fn get_projects(&self, device_id: &str) -> Result<Vec<(String, String, String, Option<u64>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, last_active_at FROM projects WHERE device_id = ?1 ORDER BY last_active_at DESC NULLS LAST",
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?.map(|t| t as u64),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 §3.2c: classify projects as stale (>30 days inactive) or path-invalid.
    /// Updates `is_stale` flag and returns the count flagged stale.
    pub fn classify_projects(&self) -> Result<i64> {
        let now = now_secs();
        let stale_threshold = now.saturating_sub(30 * 86400);
        // Mark stale: last_active older than 30 days (or never active).
        self.conn.execute(
            "UPDATE projects SET is_stale = 1 WHERE last_active_at IS NOT NULL AND last_active_at < ?1",
            params![stale_threshold],
        )?;
        // Count how many were flagged.
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM projects WHERE is_stale = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    /// Remove a project record (and its agent_instances).
    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM agent_instances WHERE project_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Promote a project-local skill to global: move its directory into center_repo
    /// and update the skill's repo_path. Returns the new repo_path.
    pub fn promote_skill_to_global(&self, skill_id: &str, center_repo: &std::path::Path) -> Result<String> {
        let skill = self
            .get_skill_by_id(skill_id)?
            .ok_or_else(|| anyhow::anyhow!("skill not found"))?;
        let dest = center_repo.join(&skill.name);
        if !dest.exists() {
            std::fs::rename(&skill.repo_path, &dest)
                .or_else(|_| {
                    // rename may fail across volumes; fall back to copy.
                    crate::fs::copy_dir_all(&skill.repo_path, &dest)?;
                    crate::fs::remove_path(&skill.repo_path).ok();
                    Ok::<(), anyhow::Error>(())
                })?;
        }
        self.conn.execute(
            "UPDATE skills SET repo_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![dest.to_string_lossy(), now_secs(), skill_id],
        )?;
        Ok(dest.to_string_lossy().to_string())
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

    pub fn upsert_kg_node(&self, n: &KgNode) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_nodes
               (id, label, type, source, description, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![n.id, n.label, n.node_type, n.source, n.description, now_secs(), now_secs()],
        )?;
        Ok(())
    }

    pub fn link_skill_node(&self, device_id: &str, skill_id: &str, node_id: &str, relevance: f64) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_skill_nodes
               (device_id, skill_id, node_id, relevance, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![device_id, skill_id, node_id, relevance, now_secs()],
        )?;
        Ok(())
    }

    pub fn upsert_kg_edge(&self, e: &KgEdge) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_edges
               (id, device_id, source_id, target_id, relation, weight, reason,
                is_manual, is_rejected, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                e.id,
                e.device_id,
                e.source_id,
                e.target_id,
                e.relation,
                e.weight,
                e.reason,
                e.is_manual as i32,
                e.is_rejected as i32,
                now_secs(),
                now_secs(),
            ],
        )?;
        Ok(())
    }

    /// Reject an auto edge (user clicked ✗); sets is_rejected=1.
    pub fn reject_kg_edge(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE kg_edges SET is_rejected = 1, updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    /// Confirm an auto edge (user clicked ✓); sets is_manual=1.
    pub fn confirm_kg_edge(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE kg_edges SET is_manual = 1, is_rejected = 0, updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    /// Load the full graph (non-rejected edges, all nodes), capped at max_nodes.
    pub fn get_kg_graph(&self, max_nodes: i64) -> Result<KgGraph> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, type, source, description FROM kg_nodes ORDER BY label LIMIT ?1",
        )?;
        let nodes: Vec<KgNode> = stmt
            .query_map(params![max_nodes], |row| {
                Ok(KgNode {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    node_type: row.get(2)?,
                    source: row.get(3)?,
                    description: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        let edges = if node_ids.is_empty() {
            vec![]
        } else {
            // Collect edges among the loaded nodes, excluding rejected ones.
            let placeholders = node_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, device_id, source_id, target_id, relation, weight, reason, is_manual, is_rejected
                 FROM kg_edges
                 WHERE is_rejected = 0 AND source_id IN ({placeholders}) AND target_id IN ({placeholders})"
            );
            let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
            for id in &node_ids {
                params_vec.push(id);
            }
            for id in &node_ids {
                params_vec.push(id);
            }
            let mut stmt2 = self.conn.prepare(&sql)?;
            let edge_rows = stmt2
                .query_map(params_vec.as_slice(), |row| {
                    Ok(KgEdge {
                        id: row.get(0)?,
                        device_id: row.get(1)?,
                        source_id: row.get(2)?,
                        target_id: row.get(3)?,
                        relation: row.get(4)?,
                        weight: row.get(5)?,
                        reason: row.get(6)?,
                        is_manual: row.get::<_, i64>(7)? != 0,
                        is_rejected: row.get::<_, i64>(8)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            edge_rows
        };
        Ok(KgGraph { nodes, edges })
    }

    /// Get all skills (id, name) for graph recommendation lookups.
    pub fn get_skill_names(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM skills ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get a single agent by id (with last_used_at / project_count).
    pub fn get_agent_by_id(&self, id: &str) -> Result<Option<Agent>> {        let mut stmt = self.conn.prepare(
            "SELECT id, name, skill_directory, is_enabled, discovery_rule, description, source FROM agents WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Agent {
                id: row.get(0)?,
                name: row.get(1)?,
                skill_directory: PathBuf::from(row.get::<_, String>(2)?),
                is_enabled: row.get::<_, i64>(3)? != 0,
                discovery_rule: row.get(4)?,
                description: row.get(5)?,
                source: row.get(6)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get last_used_at and project_count for an agent.
    pub fn get_agent_usage_meta(&self, id: &str) -> Result<(Option<u64>, i64)> {
        let row: (Option<i64>, Option<i64>) = self.conn.query_row(
            "SELECT last_used_at, project_count FROM agents WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((row.0.map(|t| t as u64), row.1.unwrap_or(0)))
    }

    /// Get projects associated with an agent (via agent_instances).
    pub fn get_agent_projects(&self, device_id: &str, agent_id: &str) -> Result<Vec<ProjectUsageSummary>> {
        // PRD-06 §3.2: join collected sessions by `agents.source` (deterministic),
        // replacing the old `CASE WHEN name LIKE 'Claude%'` string-guessing hack.
        let mut stmt = self.conn.prepare(
            r#"SELECT p.id, p.name, p.path, COUNT(DISTINCT s.id), COALESCE(SUM(t.total_tokens),0)
               FROM agent_instances ai
               JOIN projects p ON ai.project_id = p.id
               LEFT JOIN collected_sessions s
                 ON s.project_id = p.id
                AND s.source = (SELECT a.source FROM agents a WHERE a.id = ai.agent_id)
               LEFT JOIN collected_token_usage t ON t.session_id = s.id
               WHERE ai.device_id = ?1 AND ai.agent_id = ?2
               GROUP BY p.id ORDER BY p.last_active_at DESC"#,
        )?;
        let rows = stmt.query_map(params![device_id, agent_id], |row| {
            Ok(ProjectUsageSummary {
                project_id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                session_count: row.get(3)?,
                total_tokens: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-A/B: all agents active in a single project (with usage stats).
    pub fn get_project_agents(
        &self,
        device_id: &str,
        project_id: &str,
    ) -> Result<Vec<ProjectAgentEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT a.id, a.name, a.skill_directory, a.is_enabled,
                      ai.last_session_at, ai.session_count, ai.total_tokens
               FROM agent_instances ai
               JOIN agents a ON ai.agent_id = a.id
               WHERE ai.device_id = ?1 AND ai.project_id = ?2
               ORDER BY ai.total_tokens DESC, ai.last_session_at DESC NULLS LAST"#,
        )?;
        let rows = stmt.query_map(params![device_id, project_id], |row| {
            Ok(ProjectAgentEntry {
                agent_id: row.get(0)?,
                agent_name: row.get(1)?,
                skill_directory: row.get(2)?,
                is_enabled: row.get::<_, i64>(3)? != 0,
                last_session_at: row.get::<_, Option<i64>>(4)?.map(|t| t as u64),
                session_count: row.get(5)?,
                total_tokens: row.get(6)?,
                skill_count: None,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-A: a project's aggregate usage (sessions + tokens).
    pub fn get_project_aggregate(&self, device_id: &str, project_id: &str) -> Result<(i64, i64)> {
        let row: (i64, i64) = self.conn.query_row(
            r#"SELECT COUNT(DISTINCT s.id), COALESCE(SUM(t.total_tokens), 0)
               FROM collected_sessions s
               LEFT JOIN collected_token_usage t ON t.session_id = s.id
               WHERE s.device_id = ?1 AND s.project_id = ?2"#,
            params![device_id, project_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(row)
    }

    /// Get the concept nodes linked to a given skill.
    pub fn get_skill_concepts(&self, device_id: &str, skill_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT n.id, n.label FROM kg_skill_nodes sn
               JOIN kg_nodes n ON sn.node_id = n.id
               WHERE sn.device_id = ?1 AND sn.skill_id = ?2 AND n.type = 'concept'"#,
        )?;
        let rows = stmt.query_map(params![device_id, skill_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get all (source_id, target_id) pairs the user rejected (for negative-record persistence).
    pub fn get_rejected_edge_pairs(&self) -> Result<HashSet<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_id FROM kg_edges WHERE is_rejected = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            let a: String = row.get(0)?;
            let b: String = row.get(1)?;
            // Normalize order so pair comparison is order-independent.
            Ok(if a < b { (a, b) } else { (b, a) })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get skill co-occurrence pairs from attribution data grouped by project.
    /// Returns (skill_id_a, skill_id_b, count) for skills used in the same project.
    pub fn get_skill_cooccurrence(&self, device_id: &str) -> Result<Vec<(String, String, i64)>> {
        // Find skills attributed within each project, then pair them.
        let mut stmt = self.conn.prepare(
            r#"SELECT skill_id, project_id FROM skill_usage_attributions
               WHERE device_id = ?1 AND project_id IS NOT NULL AND skill_id IS NOT NULL"#,
        )?;
        let mut project_skills: HashMap<String, Vec<String>> = HashMap::new();
        let rows = stmt.query_map(params![device_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows.filter_map(|r| r.ok()) {
            project_skills.entry(r.1).or_default().push(r.0);
        }
        // Build co-occurrence counts.
        let mut counts: HashMap<(String, String), i64> = HashMap::new();
        for skills in project_skills.values() {
            let deduped: HashSet<&String> = skills.iter().collect();
            let list: Vec<&String> = deduped.into_iter().collect();
            for i in 0..list.len() {
                for j in (i + 1)..list.len() {
                    let (a, b) = if list[i] < list[j] {
                        (list[i].clone(), list[j].clone())
                    } else {
                        (list[j].clone(), list[i].clone())
                    };
                    *counts.entry((a, b)).or_default() += 1;
                }
            }
        }
        Ok(counts.into_iter().map(|((a, b), c)| (a, b, c)).collect())
    }

    // ----- PRD-07: remote sources -------------------------------------------

    pub fn insert_source(&self, s: &Source) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO sources
               (id, name, source_type, url, ref_spec, subpath, cache_path,
                commit_sha, added_at, last_fetched_at, pull_policy, remote_revision)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                s.id,
                s.name,
                s.source_type.to_string(),
                s.url,
                s.ref_spec,
                s.subpath,
                s.cache_path,
                s.commit_sha,
                s.added_at,
                s.last_fetched_at,
                s.pull_policy,
                s.remote_revision,
            ],
        )?;
        Ok(())
    }

    pub fn get_sources(&self) -> Result<Vec<Source>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, name, source_type, url, ref_spec, subpath, cache_path,
                      commit_sha, added_at, last_fetched_at, pull_policy, remote_revision
               FROM sources ORDER BY name"#,
        )?;
        let rows = stmt.query_map([], map_source_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_source_by_id(&self, id: &str) -> Result<Option<Source>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, name, source_type, url, ref_spec, subpath, cache_path,
                      commit_sha, added_at, last_fetched_at, pull_policy, remote_revision
               FROM sources WHERE id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], map_source_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn delete_source(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sources WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ----- PRD-10: scheduled tasks ------------------------------------------

    /// Seed default scheduled tasks on first run. Idempotent: only inserts when
    /// the scheduled_tasks table is empty.
    pub fn ensure_default_tasks(&self) -> Result<()> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM scheduled_tasks", [], |row| row.get(0))?;
        if count > 0 {
            return Ok(());
        }

        let defaults = vec![
            default_task(TaskKind::CollectUsageData, "使用数据采集与分析", "定时采集各 Agent 的使用数据并更新分析视图。", ScheduleStrategy::Interval { value: 30, unit: IntervalUnit::Minutes }, true),
            default_task(TaskKind::ScanAgents, "Agent 目录扫描", "扫描本机已安装的 Agent 并更新目录列表。", ScheduleStrategy::Interval { value: 1, unit: IntervalUnit::Hours }, false),
            default_task(TaskKind::SyncAllSkills, "Skill 全量同步", "将所有 Skill 同步到已启用的 Agent 目录。", ScheduleStrategy::Cron { expression: "0 3 * * *".to_string() }, false),
            default_task(TaskKind::ImportAgentSkills, "从 Agent 导入 Skill", "当 Center Repo 为空时，从 Agent 目录导入现有 Skill。", ScheduleStrategy::Manual, false),
            default_task(TaskKind::ScanAgentDirectories, "Agent 目录 Skill 扫描", "刷新各 Agent 目录下的 Skill 缓存。", ScheduleStrategy::Interval { value: 1, unit: IntervalUnit::Hours }, false),
            default_task(TaskKind::ScanProjects, "项目扫描", "扫描近期使用过的项目并关联到 Agent。", ScheduleStrategy::Interval { value: 1, unit: IntervalUnit::Hours }, false),
            default_task(TaskKind::GenerateKnowledgeGraph, "知识图谱生成", "分析所有 Skill 并生成/更新知识图谱。", ScheduleStrategy::Cron { expression: "0 2 * * *".to_string() }, false),
            default_task(TaskKind::GenerateDailySummary, "每日 AI 摘要生成", "生成前一日的 AI 工作摘要。", ScheduleStrategy::Cron { expression: "0 8 * * *".to_string() }, false),
            default_task(TaskKind::SyncRemoteSources, "远程源同步", "拉取已连接远程源的最新 Skill 变更。", ScheduleStrategy::Cron { expression: "0 4 * * *".to_string() }, false),
            default_task(TaskKind::BackupCenterRepo, "Center Repo 备份", "将 Center Repo 打包备份到本地备份目录。", ScheduleStrategy::Cron { expression: "0 2 * * 0".to_string() }, false),
            default_task(TaskKind::RunDiscoveryPipeline, "发现管线", "运行发现管线，从采集数据中识别高价值发现。", ScheduleStrategy::Cron { expression: "30 7 * * *".to_string() }, false),
            default_task(TaskKind::GenerateWeeklyReport, "周报生成", "生成本周周报。", ScheduleStrategy::Cron { expression: "0 8 * * 1".to_string() }, false),
        ];

        for task in defaults {
            self.upsert_scheduled_task(&task)?;
        }
        Ok(())
    }

    pub fn get_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, task_kind, name, description, enabled, strategy_kind,
                      strategy_value, strategy_unit, strategy_expression,
                      created_at, updated_at, last_run_at, last_status,
                      next_run_at, run_count, error_count
               FROM scheduled_tasks ORDER BY created_at"#,
        )?;
        let rows = stmt.query_map([], map_scheduled_task_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_scheduled_task(&self, id: &str) -> Result<Option<ScheduledTask>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, task_kind, name, description, enabled, strategy_kind,
                      strategy_value, strategy_unit, strategy_expression,
                      created_at, updated_at, last_run_at, last_status,
                      next_run_at, run_count, error_count
               FROM scheduled_tasks WHERE id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], map_scheduled_task_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn upsert_scheduled_task(&self, task: &ScheduledTask) -> Result<()> {
        let (kind, value, unit, expr) = strategy_to_db(&task.strategy);
        self.conn.execute(
            r#"INSERT OR REPLACE INTO scheduled_tasks
               (id, task_kind, name, description, enabled, strategy_kind,
                strategy_value, strategy_unit, strategy_expression,
                created_at, updated_at, last_run_at, last_status,
                next_run_at, run_count, error_count)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"#,
            params![
                task.id,
                task.task_kind.to_string(),
                task.name,
                task.description,
                task.enabled as i32,
                kind,
                value,
                unit,
                expr,
                task.created_at,
                task.updated_at,
                task.last_run_at,
                task.last_status.as_ref().map(status_to_db),
                task.next_run_at,
                task.run_count,
                task.error_count,
            ],
        )?;
        Ok(())
    }

    pub fn delete_scheduled_task(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM scheduled_tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_task_run(&self, run: &TaskRun) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO task_runs
               (id, task_id, status, started_at, finished_at, duration_ms,
                result_summary, error_message, triggered_by)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
            params![
                run.id,
                run.task_id,
                status_to_db(&run.status),
                run.started_at,
                run.finished_at,
                run.duration_ms,
                run.result_summary,
                run.error_message,
                trigger_to_db(&run.triggered_by),
            ],
        )?;
        Ok(())
    }

    pub fn update_task_run(&self, run: &TaskRun) -> Result<()> {
        self.conn.execute(
            r#"UPDATE task_runs SET status = ?2, finished_at = ?3, duration_ms = ?4,
                result_summary = ?5, error_message = ?6, triggered_by = ?7
               WHERE id = ?1"#,
            params![
                run.id,
                status_to_db(&run.status),
                run.finished_at,
                run.duration_ms,
                run.result_summary,
                run.error_message,
                trigger_to_db(&run.triggered_by),
            ],
        )?;
        Ok(())
    }

    pub fn get_task_runs(&self, task_id: &str, limit: u32) -> Result<Vec<TaskRun>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, task_id, status, started_at, finished_at, duration_ms,
                      result_summary, error_message, triggered_by
               FROM task_runs WHERE task_id = ?1
               ORDER BY started_at DESC LIMIT ?2"#,
        )?;
        let rows = stmt.query_map(params![task_id, limit], map_task_run_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_running_task_run(&self, task_id: &str) -> Result<Option<TaskRun>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, task_id, status, started_at, finished_at, duration_ms,
                      result_summary, error_message, triggered_by
               FROM task_runs WHERE task_id = ?1 AND status = 'running'
               ORDER BY started_at DESC LIMIT 1"#,
        )?;
        let mut rows = stmt.query_map(params![task_id], map_task_run_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn delete_old_task_runs(&self, task_id: &str, keep: u32) -> Result<u64> {
        let cutoff: Option<i64> = self.conn.query_row(
            r#"SELECT started_at FROM task_runs
               WHERE task_id = ?1 ORDER BY started_at DESC LIMIT 1 OFFSET ?2"#,
            params![task_id, keep],
            |row| row.get(0),
        ).optional()?;
        if let Some(ts) = cutoff {
            let deleted = self.conn.execute(
                "DELETE FROM task_runs WHERE task_id = ?1 AND started_at <= ?2",
                params![task_id, ts],
            )?;
            Ok(deleted as u64)
        } else {
            Ok(0)
        }
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

impl Db {
    /// P0: convenience helper to log a `llm::ChatOutcome` with request metadata.
    pub fn log_llm_request(
        &self,
        outcome: &crate::llm::ChatOutcome,
        request_kind: &str,
    ) {
        let log = crate::models::LlmRequestLog {
            id: crate::db::new_id(),
            requested_at: now_secs(),
            provider: outcome.provider.clone(),
            fallback: outcome.fallback,
            has_raw_text: true,
            error: outcome.error.clone(),
            metadata_json: serde_json::json!({"kind": request_kind}).to_string(),
        };
        let _ = self.insert_llm_request_log(&log);
    }

    /// P0: insert an LLM/ACP request audit log entry.
    pub fn insert_llm_request_log(&self, log: &crate::models::LlmRequestLog) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO llm_request_logs
               (id, requested_at, provider, fallback, has_raw_text, error, metadata_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                log.id,
                log.requested_at as i64,
                log.provider,
                log.fallback as i32,
                log.has_raw_text as i32,
                log.error,
                log.metadata_json,
            ],
        )?;
        Ok(())
    }

    /// P0: list recent LLM/ACP request audit logs, newest first.
    pub fn list_llm_request_logs(&self, limit: usize) -> Result<Vec<crate::models::LlmRequestLog>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, requested_at, provider, fallback, has_raw_text, error, metadata_json
               FROM llm_request_logs
               ORDER BY requested_at DESC
               LIMIT ?1"#,
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::models::LlmRequestLog {
                id: row.get(0)?,
                requested_at: row.get::<_, i64>(1)?.max(0) as u64,
                provider: row.get(2)?,
                fallback: row.get::<_, i32>(3)? != 0,
                has_raw_text: row.get::<_, i32>(4)? != 0,
                error: row.get(5)?,
                metadata_json: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    /// P0: mark any 'running' collection jobs as failed (e.g. after an app crash).
    pub fn reset_stale_collection_jobs(&self) -> Result<()> {
        let now = now_secs();
        self.conn.execute(
            r#"UPDATE collection_jobs
               SET status = 'failed', completed_at = ?1, error = 'app restarted while job was running'
               WHERE status = 'running'"#,
            params![now as i64],
        )?;
        Ok(())
    }

    /// P2-2: return the local SQLite path and a human-readable data dictionary.
    pub fn export_data_dictionary(&self) -> Result<DataDictionary> {
        let tables = vec![
            TableInfo {
                name: "skills".to_string(),
                description: "中心仓库中的 Skill 元数据".to_string(),
                columns: vec!["id", "name", "repo_path", "created_at", "updated_at", "status"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "collected_sessions".to_string(),
                description: "从各 Agent 采集的会话记录".to_string(),
                columns: vec!["id", "device_id", "source", "project_id", "agent_id", "start_time", "end_time", "message_count", "title_or_prompt", "cached_at"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "skill_versions".to_string(),
                description: "Skill 的历史版本快照".to_string(),
                columns: vec!["id", "device_id", "skill_id", "version", "content", "created_at", "created_by"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "skill_bundles".to_string(),
                description: "用户创建的 Skill 组合".to_string(),
                columns: vec!["id", "device_id", "name", "description", "created_at", "updated_at"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "projects".to_string(),
                description: "从会话中识别的项目".to_string(),
                columns: vec!["id", "device_id", "name", "path", "first_seen_at", "last_active_at"].into_iter().map(String::from).collect(),
            },
        ];
        Ok(DataDictionary {
            db_path: self.path.to_string_lossy().to_string(),
            tables,
        })
    }

    /// P2-3: seed a built-in official example source when no remote/local sources exist.
    pub fn ensure_official_example_source(
        &self,
        center_repo: &std::path::Path,
    ) -> Result<Option<Source>> {
        let existing: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
        if existing > 0 {
            return Ok(None);
        }

        let examples_dir = center_repo.join("examples");
        let sample_skill_dir = examples_dir.join("hello-skillmint");
        let skill_md = sample_skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            std::fs::create_dir_all(&sample_skill_dir)?;
            std::fs::write(
                &skill_md,
                r#"---
name: hello-skillmint
description: SkillMint 官方示例 Skill，演示 frontmatter 格式。
author: SkillMint
---

# hello-skillmint

这是一个官方示例 Skill。

## Usage

在任意项目中让 Agent 使用本 Skill，观察 SkillMint 如何把它同步到本地 Agent 目录。
"#,
            )?;
        }

        let source = Source {
            id: new_id(),
            source_type: SourceType::Local,
            name: "SkillMint 官方示例".to_string(),
            url: examples_dir.to_string_lossy().to_string(),
            ref_spec: String::new(),
            subpath: String::new(),
            cache_path: String::new(),
            commit_sha: String::new(),
            added_at: now_secs(),
            last_fetched_at: None,
            pull_policy: "manual".to_string(),
            remote_revision: String::new(),
        };
        self.insert_source(&source)?;
        Ok(Some(source))
    }

    /// P2-2: export the requested tables as a single JSON object.
    pub fn export_raw_data(&self, tables: &[String]) -> Result<RawDataExport> {
        let allowed: std::collections::HashSet<&str> = [
            "skills", "agents", "sync_targets", "collected_sessions", "collected_prompts",
            "skill_versions", "skill_bundles", "skill_bundle_items", "projects",
            "skill_project_bindings", "llm_request_logs", "collection_jobs",
        ]
        .iter()
        .cloned()
        .collect();

        let mut data = serde_json::Map::new();
        for table in tables {
            if !allowed.contains(table.as_str()) {
                anyhow::bail!("unsupported table: {table}");
            }
            let rows = self.dump_table_as_json(table)?;
            data.insert(table.clone(), serde_json::Value::Array(rows));
        }

        Ok(RawDataExport {
            format: "json".to_string(),
            exported_at: now_secs(),
            tables: tables.to_vec(),
            data: serde_json::Value::Object(data),
        })
    }

    fn dump_table_as_json(&self, table: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(&format!("SELECT * FROM {table}"))?;
        let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let mut rows = Vec::new();
        let mut result = stmt.query([])?;
        while let Some(row) = result.next()? {
            let mut obj = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => serde_json::Value::Number(n.into()),
                    rusqlite::types::ValueRef::Real(n) => serde_json::json!(n),
                    rusqlite::types::ValueRef::Text(s) => serde_json::Value::String(
                        std::str::from_utf8(s).unwrap_or("").to_string()
                    ),
                    rusqlite::types::ValueRef::Blob(b) => serde_json::Value::String(format!("<blob {} bytes>", b.len())),
                };
                obj.insert(col.clone(), value);
            }
            rows.push(serde_json::Value::Object(obj));
        }
        Ok(rows)
    }

    /// SPEC-F3: delete all collected data while preserving skills, agents, settings,
    /// sync targets and project bindings. Runs inside a transaction.
    pub fn clear_collected_data(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let tables = [
            "collected_sources",
            "collected_sessions",
            "collected_prompts",
            "collected_token_usage",
            "collected_code_contributions",
            "collector_file_states",
            "collection_jobs",
            "analysis_window_cache",
            "digest_summary",
            "llm_request_logs",
        ];
        for table in tables {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// SPEC-F3: reset the entire database except for the center repo files on disk.
    /// Drops all user tables and re-creates the schema. Runs inside a transaction.
    pub fn reset_database(&mut self, device_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let tables = [
            "agents",
            "agent_directories",
            "agent_instances",
            "skills",
            "sync_targets",
            "projects",
            "skill_project_bindings",
            "skill_bundles",
            "skill_bundle_items",
            "collected_sources",
            "collected_sessions",
            "collected_prompts",
            "collected_token_usage",
            "collected_code_contributions",
            "collector_file_states",
            "collection_jobs",
            "kg_nodes",
            "kg_edges",
            "kg_skill_nodes",
            "sources",
            "digest_model",
            "digest_tool",
            "digest_summary",
            "analysis_window_cache",
            "discoveries",
            "weekly_reports",
            "scheduled_tasks",
            "task_runs",
            "llm_request_logs",
            "install_audit",
        ];
        for table in tables {
            let _ = tx.execute(&format!("DROP TABLE IF EXISTS {table}"), []);
        }
        tx.commit()?;
        self.init(device_id)?;
        Ok(())
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
