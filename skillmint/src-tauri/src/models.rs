use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    #[default]
    Draft,
    Candidate,
    Approved,
    Deprecated,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStatus::Draft => write!(f, "draft"),
            SkillStatus::Candidate => write!(f, "candidate"),
            SkillStatus::Approved => write!(f, "approved"),
            SkillStatus::Deprecated => write!(f, "deprecated"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub status: SkillStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub skill_directory: PathBuf,
    pub is_enabled: bool,
    pub discovery_rule: Option<String>,
    pub description: Option<String>,
    /// PRD-06: collection normalization key (e.g. "claude-code", "codex", "zcode",
    /// "cursor"). Drives deterministic attribution of `collected_*.source` to this
    /// Agent, replacing the old `name LIKE`/`derive_agent_id` hacks. `None` for
    /// user-created agents whose source is unknown (collection won't attribute).
    #[serde(default)]
    pub source: Option<String>,
}

/// PRD-06: one directory owned by an Agent (1:N). An Agent owns multiple
/// directories (e.g. Claude Code owns `~/.claude/skills` and `~/.claude/commands`);
/// the directory is a sub-record, not the entity itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDirectory {
    pub id: String,
    pub agent_id: String,
    pub path: PathBuf,
    /// Free-form role label: "skills" / "commands" / "rules" / "instructions" / ...
    /// Not CHECK-constrained so new agents don't require a schema change.
    pub role: Option<String>,
    pub is_enabled: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTarget {
    pub id: String,
    pub skill_id: String,
    pub skill_name: Option<String>,
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub mode: SyncMode,
    pub last_sync_at: Option<u64>,
    pub status: SyncStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Symlink,
    Copy,
}

impl std::fmt::Display for SyncMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncMode::Symlink => write!(f, "symlink"),
            SyncMode::Copy => write!(f, "copy"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    LocalChanged,
    CenterChanged,
    Conflict,
    Broken,
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStatus::Synced => write!(f, "synced"),
            SyncStatus::LocalChanged => write!(f, "local_changed"),
            SyncStatus::CenterChanged => write!(f, "center_changed"),
            SyncStatus::Conflict => write!(f, "conflict"),
            SyncStatus::Broken => write!(f, "broken"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub device_id: String,
    pub auto_sync_interval_minutes: u32,
    pub launch_at_login: bool,
    pub show_dock_icon: bool,
    pub onboarding_completed: bool,
    /// PRD-07: master switch for remote features.
    #[serde(default)]
    pub remote_enabled: bool,
    /// M1: appearance theme ("light" | "dark" | "system").
    #[serde(default = "crate::settings::default_theme_pub")]
    pub theme: String,
    /// PRD-08 §3.6: optional LLM config for classification + daily summaries.
    #[serde(default)]
    pub ai: crate::settings::AiConfig,
    /// P3: npm package spec invoked as `npx -y <spec>`; default `skills@latest`,
    /// pin a version (e.g. `skills@1.5.23`) for reproducibility.
    #[serde(default = "crate::settings::default_npx_package_pub")]
    pub npx_package: String,
    /// P3: override for the skills.sh search API base URL (China acceleration).
    #[serde(default)]
    pub skills_api_url: String,
    /// P3: proxy URL injected as HTTPS_PROXY/HTTP_PROXY/ALL_PROXY into npx.
    #[serde(default)]
    pub proxy_env: String,
    /// P3: inject DISABLE_TELEMETRY=1 into npx (default on — local-first).
    #[serde(default = "crate::settings::default_true_pub")]
    pub disable_telemetry: bool,
    /// P3: explicit node bin dir (or node binary path) for GUI PATH limitations.
    #[serde(default)]
    pub node_path_override: String,
    /// P4: OpenViking context-database integration (default off).
    #[serde(default)]
    pub openviking: crate::settings::OpenVikingConfig,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFailure {
    pub target_id: String,
    pub skill_id: String,
    pub skill_name: Option<String>,
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub error: String,
    /// Human-readable recovery suggestion populated from the error type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAllResult {
    pub targets: Vec<SyncTarget>,
    pub imported_skills: usize,
    pub import_conflicts: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub failures: Vec<SyncFailure>,
}

/// F6: result of checking whether the center repo is healthy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // retired with the center repo; repair tests use it as an oracle
pub enum RepoIntegrity {
    /// Center repo exists and is non-empty, or the DB has no skills yet.
    Healthy,
    /// Center repo is missing/empty but the DB still has skill records.
    MissingWithRecords,
    /// P1-5 reverse check: the center repo contains unregistered skill
    /// directories (orphans) while the DB itself is consistent.
    OrphansPresent,
}

/// Outcome of restoring a Center Repo zip backup (PRD-0 §4.8).
/// Smart-merge semantics: existing skills are never overwritten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreSummary {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictPayload {
    pub sync_target_id: String,
    pub resolution: ConflictResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillItem {
    pub name: String,
    pub exists_in_center: bool,
    pub content_match: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictContent {
    pub sync_target_id: String,
    pub skill_name: String,
    pub agent_name: String,
    pub center_content: String,
    pub local_content: String,
    /// P1-3: center skill last updated timestamp (DB updated_at or file mtime).
    pub center_updated_at: Option<u64>,
    /// P1-3: local agent copy last modified timestamp (file mtime).
    pub local_updated_at: Option<u64>,
    /// P1-3: author of the center version, if known.
    pub center_author: Option<String>,
    /// P1-3: author of the local version, if known.
    pub local_author: Option<String>,
}

/// P2-2: data dictionary payload for the data-management page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataDictionary {
    pub db_path: String,
    pub tables: Vec<TableInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub description: String,
    pub columns: Vec<String>,
}

/// P2-2: raw-data export payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawDataExport {
    pub format: String,
    pub exported_at: u64,
    pub tables: Vec<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    KeepCenter,
    KeepLocal,
    Skip,
}

// -----------------------------------------------------------------------------
// PRD-02: usage data collection (normalized, source-agnostic)
// -----------------------------------------------------------------------------

/// Status of a single data source, surfaced in the Settings "采集" block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedSource {
    pub source: String,        // Agent type, e.g. "claude-code"
    pub collector_kind: String, // direct_file / digest_db / sqlite
    pub data_path: String,
    pub status: String,        // ok / not_found / busy / schema_incompatible / unsupported / error
    pub last_collected_at: Option<u64>,
    pub record_count: i64,
}

/// One normalized session (one .jsonl file = one session for Claude Code).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedSession {
    pub id: String,            // device_id:<source>:<session_id>
    pub device_id: String,
    pub source: String,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub message_count: i64,
    pub title_or_prompt: Option<String>,
    pub cached_at: u64,
    /// Raw cwd captured during collection; linked to a `projects` row by link_projects.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Heuristic quality score 0..100 (PRD-02 §3.2).
    #[serde(default)]
    pub quality_score: Option<f64>,
}

/// One user turn / prompt sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedPrompt {
    pub id: String,            // device_id:<source>:<session_id>:<line_uuid>
    pub device_id: String,
    pub session_id: String,
    pub source: String,
    pub project_id: Option<String>,
    pub prompt_text: Option<String>,
    pub started_at: Option<u64>,
    pub duration_ms: Option<i64>,
    // Four-dimensional semantic labels (filled when source is AI-Digest; NULL otherwise).
    pub requested_action: Option<String>,
    pub target_object: Option<String>,
    pub interaction_state: Option<String>,
    pub interaction_mode: Option<String>,
    pub confidence: Option<f64>,
    pub tool_calls: Option<i64>,
    pub tool_errors: Option<i64>,
}

/// Aggregated token usage for one (session, model) pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedTokenUsage {
    pub id: String,            // device_id:<source>:<session_id>:<model_id>
    pub device_id: String,
    pub session_id: String,
    pub source: String,
    pub project_id: Option<String>,
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

/// Result of a single collector run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionStats {
    pub source: String,
    pub sessions: i64,
    pub prompts: i64,
    pub token_rows: i64,
    pub skipped: i64,
}

/// Incremental collection state for a single data file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedFileState {
    pub source: String,
    pub file_path: String,
    pub last_modified_ns: i64,
    pub last_size: i64,
    pub last_collected_at: u64,
    /// SHA-256 of file content. Used as a more accurate change-detector than
    /// mtime/size alone. `None` for legacy rows until the next collection.
    pub content_hash: Option<String>,
}

/// A background collection job (async manual collection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionJob {
    pub id: String,
    pub started_at: u64,
    pub completed_at: Option<u64>,
    pub status: String,
    pub progress_json: String,
    pub result_json: Option<String>,
    pub error: Option<String>,
}

/// P0: one LLM/ACP request audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequestLog {
    pub id: String,
    pub requested_at: u64,
    pub provider: String,
    pub fallback: bool,
    pub has_raw_text: bool,
    pub error: Option<String>,
    pub metadata_json: String,
}

/// PRD-05 §5.2: one Cursor-scored git commit (AI code contribution, NOT tokens).
/// Cursor's local `ai-tracking.db` has no token data, so this is a separate
/// metric family — it must NOT be folded into `collected_token_usage` (different
/// semantics would pollute cross-agent token sums).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectedCodeContribution {
    pub id: String,                  // device_id:cursor:<commitHash>
    pub device_id: String,
    pub source: String,              // always "cursor"
    pub project_id: Option<String>,  // P0: null (scored_commits has no path); P1 via git root
    pub commit_hash: String,
    pub branch_name: Option<String>,
    pub commit_date: Option<String>, // scored_commits.commitDate (string, parsed)
    pub scored_at: Option<u64>,      // scored_commits.scoredAt (ms→s)
    pub lines_added: Option<i64>,
    pub lines_deleted: Option<i64>,
    pub composer_lines_added: Option<i64>,  // AI-written added lines
    pub composer_lines_deleted: Option<i64>,
    pub human_lines_added: Option<i64>,
    pub human_lines_deleted: Option<i64>,
    pub tab_lines_added: Option<i64>,       // autocomplete completions
    pub ai_percentage: Option<f64>,         // float(v2AiPercentage or v1AiPercentage), 0-100
    pub commit_message: Option<String>,
    pub cached_at: u64,
}

/// Aggregated usage for an agent over the last N days (P0 data-tab payload).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentUsageSummary {
    pub source: String,
    pub days: u32,
    pub session_count: i64,
    pub prompt_count: i64,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
}

// -----------------------------------------------------------------------------
// PRD-02: project profile + skill usage attribution
// -----------------------------------------------------------------------------

/// A skill-usage attribution record (one per detected skill invocation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageAttribution {
    pub id: String,            // device_id:<source>:<sessionId>:<tool_use_id>
    pub device_id: String,
    pub skill_id: Option<String>, // linked skills.id if a same-named skill exists; else NULL
    pub skill_name: String,    // normalized name (plugin prefix stripped)
    pub source: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub attribution_type: String, // "direct"
    pub attributed_at: u64,
}

/// One skill's usage summary over the last N days.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillUsageSummary {
    pub skill_name: String,
    pub skill_id: Option<String>,
    pub usage_count: i64,
    pub session_count: i64,
    pub project_count: i64,
}

/// One project's usage summary over the last N days (project profile).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectUsageSummary {
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub session_count: i64,
    pub total_tokens: i64,
}

/// Agent detail payload (PRD-01 §3.1 detail page): agent info + usage + projects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentDetail {
    pub id: String,
    pub name: String,
    pub skill_directory: String,
    pub description: Option<String>,
    pub is_enabled: bool,
    pub last_used_at: Option<u64>,
    pub project_count: i64,
    pub source: Option<String>,
    pub usage_7d: Option<AgentUsageSummary>,
    pub projects: Vec<ProjectUsageSummary>,
    /// PRD-06 §3.3: all directories owned by this Agent (1:N). Empty for agents
    /// created before the refactor until the migration backfills a primary row.
    #[serde(default)]
    pub directories: Vec<AgentDirectory>,
}

// -----------------------------------------------------------------------------
// PRD-02: prompt-driven skill generation + reports
// -----------------------------------------------------------------------------

/// A high-value prompt identified for skill sedimentation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HighValuePrompt {
    pub prompt_text: String,
    pub repeat_count: i64,
    pub sample_session_id: String,
    pub source: Option<String>,
}

/// 「高频使用 Prompt」分页结果（分组键 = prompt 前 120 字符 + source）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HighValuePromptPage {
    pub items: Vec<HighValuePrompt>,
    pub total: i64,
}

/// 用户忽略的高频 Prompt 分组（永久生效，可在列表底部恢复）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IgnoredPromptGroup {
    pub group_key: String,
    pub source: Option<String>,
    pub prompt_sample: Option<String>,
    pub created_at: u64,
}

/// A skill-suggestion weekly report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillSuggestionReport {
    pub generated_at: u64,
    pub top_prompts: Vec<HighValuePrompt>,
    pub underused_skills: Vec<String>,
    pub top_skills: Vec<String>,
}

// -----------------------------------------------------------------------------
// PRD-01: project-level skill mode + bindings
// -----------------------------------------------------------------------------

/// Skill scope mode (global vs project).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillScopeMode {
    Global,
    Project,
}

impl Default for SkillScopeMode {
    fn default() -> Self {
        SkillScopeMode::Global
    }
}

impl std::fmt::Display for SkillScopeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillScopeMode::Global => write!(f, "global"),
            SkillScopeMode::Project => write!(f, "project"),
        }
    }
}

/// A skill-project binding (reference of a global skill to a project).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillProjectBinding {
    pub id: String,
    pub device_id: String,
    pub skill_id: String,
    pub skill_name: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub agent_id: Option<String>,
    pub mode: String, // local_copy / symlink / reference
    pub local_path: Option<String>,
    pub is_enabled: bool,
    /// PRD-01 patch FR-F: pinned version for multi-version skills.
    /// None  = follow `latest`; Some("v2") = locked to that snapshot.
    #[serde(default)]
    pub pinned_version: Option<String>,
}

// -----------------------------------------------------------------------------
// PRD-02 Phase 2: skill bundles (named reusable collections of skills)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundle {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub skill_count: i64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundleItem {
    pub id: String,
    pub skill_id: String,
    pub skill_name: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyBundleResult {
    pub applied: Vec<String>,
    pub skipped: Vec<(String, String)>, // (skill_name, reason)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleExport {
    pub name: String,
    pub description: Option<String>,
    pub exported_at: u64,
    pub skills: Vec<BundleExportSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleExportSkill {
    pub name: String,
    pub required: bool,
}

// -----------------------------------------------------------------------------
// PRD-01 patch: project detail page + skill multi-version management
// -----------------------------------------------------------------------------

/// One agent's usage profile *within a single project* (PRD-01 patch FR-A/B).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectAgentEntry {
    pub agent_id: String,
    pub agent_name: String,
    pub skill_directory: String,
    pub is_enabled: bool,
    pub last_session_at: Option<u64>,
    pub session_count: i64,
    pub total_tokens: i64,
    /// Number of skills physically present in this agent's skill directory.
    /// Filled lazily (None = not yet counted).
    pub skill_count: Option<i64>,
}

/// Full payload for the project detail page (PRD-01 patch FR-A).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub last_active_at: Option<u64>,
    pub session_count: i64,
    pub total_tokens: i64,
    pub agents: Vec<ProjectAgentEntry>,
}

/// How a skill physically exists in an agent's directory (PRD-01 patch FR-C).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// A symlink pointing at the center-repo original.
    Symlink,
    /// An independent copy (decoupled from the original).
    LocalCopy,
    /// Project-local skill living under `.skillmint/skills/`, no original.
    Local,
    /// Symlink broken or path missing.
    Broken,
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSource::Symlink => write!(f, "symlink"),
            SkillSource::LocalCopy => write!(f, "local_copy"),
            SkillSource::Local => write!(f, "local"),
            SkillSource::Broken => write!(f, "broken"),
        }
    }
}

/// A resolved view of one skill in an agent directory (PRD-01 patch FR-C).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedSkill {
    pub name: String,
    /// Serialized [`SkillSource`].
    pub source: String,
    /// Absolute path the symlink points at (only for `symlink`).
    pub target_path: Option<String>,
    /// Whether the copy matches the center original (None if no original to compare).
    pub content_match: Option<bool>,
    /// Whether this skill is registered in `skill_project_bindings` for the project.
    pub is_registered: bool,
    /// Pinned version label, if registered and pinned (e.g. "v2").
    pub pinned_version: Option<String>,
}

/// Strategy for resolving a content diff (PRD-01 patch FR-F §4.5c).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiffStrategy {
    /// Overwrite the project copy with center `latest` (project variant lost).
    KeepCenter,
    /// Overwrite center `latest` with the project copy (feed back upstream).
    KeepProject,
    /// Snapshot current `latest` as a new `v<x>` and pin the project to it.
    Versionize,
}

impl std::fmt::Display for DiffStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffStrategy::KeepCenter => write!(f, "keep_center"),
            DiffStrategy::KeepProject => write!(f, "keep_project"),
            DiffStrategy::Versionize => write!(f, "versionize"),
        }
    }
}

/// Result of a diff resolution (PRD-01 patch FR-F).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolveResult {
    /// The strategy that was applied (echoed back).
    pub strategy: String,
    /// New version created, if any (only for `Versionize`, e.g. "v2").
    pub new_version: Option<String>,
    /// Path of the project copy after resolution.
    pub project_path: String,
    /// Path of the center latest after resolution.
    pub latest_path: String,
}

// -----------------------------------------------------------------------------
// PRD-03: knowledge graph
// -----------------------------------------------------------------------------

/// A concept/scenario/practice node in the skill knowledge graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KgNode {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub source: Option<String>,
    pub description: Option<String>,
}

/// An edge between two kg nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KgEdge {
    pub id: String,
    pub device_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub weight: Option<f64>,
    pub reason: Option<String>,
    pub is_manual: bool,
    pub is_rejected: bool,
}

/// Graph payload for the knowledge-graph view (nodes + edges).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KgGraph {
    pub nodes: Vec<KgNode>,
    pub edges: Vec<KgEdge>,
}

/// A skill recommended for a task, with reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRecommendation {
    pub skill_id: String,
    pub skill_name: String,
    pub reason: String,
    pub matched_concepts: Vec<String>,
}

/// Task-driven recommendation result (matched skills + concept gaps).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskRecommendation {
    pub recommendations: Vec<SkillRecommendation>,
    pub gaps: Vec<String>,
}

// -----------------------------------------------------------------------------
// PRD-07: remote sources, unified discovery, safety scan
// -----------------------------------------------------------------------------

/// Kind of a remote source. `local` is a directory on disk the user pointed at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Github,
    Git,
    Local,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Github => write!(f, "github"),
            SourceType::Git => write!(f, "git"),
            SourceType::Local => write!(f, "local"),
        }
    }
}

impl SourceType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "git" => SourceType::Git,
            "local" => SourceType::Local,
            _ => SourceType::Github,
        }
    }
}

/// A connected remote source (PRD-07 §3.1). Lives in the `sources` SQLite table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub source_type: SourceType,
    /// github: `owner/repo`; git: full URL; local: absolute path.
    pub url: String,
    pub ref_spec: String,
    /// Subdirectory within the repo to scan for skills (empty = root).
    #[serde(default)]
    pub subpath: String,
    /// Absolute cache directory under `~/.skillmint/cache/sources/<id>/`.
    pub cache_path: String,
    /// Commit SHA of the last successful fetch (empty for local sources).
    #[serde(default)]
    pub commit_sha: String,
    pub added_at: u64,
    pub last_fetched_at: Option<u64>,
    /// "manual" (default) or "on_startup".
    #[serde(default = "default_pull_policy")]
    pub pull_policy: String,
    /// Latest remote commit SHA (for update detection); refreshed on fetch.
    #[serde(default)]
    pub remote_revision: String,
}

fn default_pull_policy() -> String {
    "manual".to_string()
}

/// One skill discovered inside a source's cache (PRD-07 §3.1/§3.3). This is a
/// computed view built by scanning the cache dir, not a persisted table — the
/// frontend receives it via `list_source_skills` / `search_all`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRemoteMeta {
    pub skill_name: String,
    pub source_id: String,
    /// Path of the skill dir relative to the source cache root.
    pub skill_path: String,
    /// Description from SKILL.md frontmatter (best-effort).
    #[serde(default)]
    pub description: Option<String>,
    /// Content hash of the skill dir (for change detection).
    #[serde(default)]
    pub computed_hash: Option<String>,
    /// Whether an identically-named skill exists in the center repo.
    #[serde(default)]
    pub installed_locally: bool,
}

/// Result of scanning a skill's SKILL.md for high-risk instructions (PRD-07 §3.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SafetyScanResult {
    pub clean: bool,
    /// Each finding: (line_no, matched_rule, line_excerpt).
    pub findings: Vec<SafetyFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyFinding {
    pub line: usize,
    pub rule: String,
    pub excerpt: String,
}

/// One row of the unified search result (PRD-07 §3.3). Combines local and
/// remote matches, de-duplicated by skill name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub skill_name: String,
    /// "local" or "remote".
    pub origin: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub description: Option<String>,
    pub installed_locally: bool,
    pub usage_count: i64,
    /// Number of clarification/correction prompts associated with this skill's
    /// usage sessions over the ranking window (PRD-09 §3.3c).
    #[serde(default)]
    pub correction_count: i64,
    /// For remote results: skill dir path relative to the source cache root.
    /// Required to locate the skill on install/preview when a source has a
    /// subpath (otherwise `skill_name` alone would resolve to the wrong place).
    /// Empty for local results.
    #[serde(default)]
    pub skill_path: String,
    /// Which field produced the match: "name" | "description" | "tags" | "body".
    #[serde(default)]
    pub match_field: Option<String>,
    /// Context snippet when the match came from the SKILL.md body, with the
    /// matched term wrapped in «» so the frontend can render it as a highlight.
    #[serde(default)]
    pub snippet: Option<String>,
}

// =============================================================================
// PRD-08: full analysis integration (AI-Digest analysis methods, ported to Rust).
// These structs are the serialized shape of the analysis engine output. The raw
// inputs still live in the `collected_*` tables (PRD-02); these structs are the
// *derived* layer shown on the upgraded Usage page. Field names mirror the
// AI-Digest JSON contract so the two stay caliber-comparable.
// =============================================================================

/// One row of token usage, normalized for metric computation. Built from
/// `collected_token_usage` (the source of truth) plus the derived cost in CNY.
/// Mirrors AI-Digest `TokenUsage` (models.py). Durations are in milliseconds.
#[derive(Debug, Clone, Default)]
pub struct UsageSample {
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

/// One user prompt/turn sample for the collaboration dimension. Mirrors
/// AI-Digest `PromptSample`. The four semantic axes are filled by the LLM
/// classifier (P1); until then they stay `None` but the columns exist.
#[derive(Debug, Clone, Default)]
pub struct PromptSample {
    pub session_id: String,
    pub source: String,
    pub project_path: Option<String>,
    pub duration_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub requested_action: Option<String>,
    pub target_object: Option<String>,
    pub interaction_state: Option<String>,
    pub interaction_mode: Option<String>,
}

/// A window-comparison delta: current value vs a baseline (previous / YoY).
/// `pct` is `None` when the baseline is 0 (cannot divide) — the UI shows "—".
/// Mirrors AI-Digest `_delta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub current: f64,
    pub previous: f64,
    pub delta: f64,
    pub pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One metric in the comparison table: current value plus optional 环比 (mom)
/// and 同比 (yoy) deltas. `None` when no baseline window has data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonEntry {
    pub current: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mom: Option<Delta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yoy: Option<Delta>,
}

/// Token-dimension scale block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenScale {
    pub total_tokens: i64,
    pub fresh_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub model_calls: i64,
    pub tool_calls: i64,
    pub duration_hours: f64,
}

/// Token-dimension cost block. `est_cost_cny` is a reference estimate (参考估算).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenCost {
    pub est_cost_cny: f64,
    pub by_platform_cny: std::collections::BTreeMap<String, f64>,
    /// billing_mode -> count of (session,model) usage rows.
    pub billing_mix: std::collections::BTreeMap<String, i64>,
}

/// Token-dimension diagnostics (cache ratio + context-bloat candidates).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenDiagnostics {
    pub cache_ratio: f64,
    pub heavy_sessions: Vec<HeavySession>,
}

/// A top-10%-by-tokens session — a possible context-bloat signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavySession {
    pub session_id: String,
    pub project: Option<String>,
    pub model: Option<String>,
    pub total_tokens: i64,
    pub cache_read: i64,
    pub model_calls: i64,
}

/// The full token-dimension payload. Distribution maps are BTreeMap so JSON
/// output is deterministic (stable key order) for snapshot tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenDimension {
    #[serde(skip_serializing_if = "TokenScale::is_empty", default)]
    pub scale: TokenScale,
    pub distribution: TokenDistribution,
    #[serde(skip_serializing_if = "TokenEfficiency::is_empty", default)]
    pub efficiency: TokenEfficiency,
    pub cost: TokenCost,
    pub diagnostics: TokenDiagnostics,
}

/// Token distribution (by platform / project top-10 / model). `by_platform` is
/// shared with the source filter, so it's keyed by the raw `source` value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenDistribution {
    pub by_platform: std::collections::BTreeMap<String, i64>,
    pub by_project: std::collections::BTreeMap<String, i64>,
    pub by_model: std::collections::BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenEfficiency {
    pub tokens_per_session: i64,
    pub fresh_tokens_per_session: i64,
    pub tokens_per_call: i64,
    pub tokens_per_hour: i64,
}

impl TokenScale {
    fn is_empty(&self) -> bool {
        self.total_tokens == 0
    }
}
impl TokenEfficiency {
    fn is_empty(&self) -> bool {
        self.tokens_per_session == 0 && self.tokens_per_hour == 0
    }
}

/// Prompt-dimension penetration block (counts by platform/project).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptPenetration {
    pub total_prompts: i64,
    pub by_platform: std::collections::BTreeMap<String, i64>,
    pub by_project: std::collections::BTreeMap<String, i64>,
}

/// Prompt-dimension maturity block (agent autonomy coefficient).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptMaturity {
    pub agent_coefficient_ms_per_prompt: i64,
    pub agent_coefficient_min_per_prompt: f64,
    pub avg_tool_calls_per_prompt: f64,
    pub total_tool_calls: i64,
}

/// Prompt-dimension semantic distribution (the four axes). Only over classified
/// prompts; classified_ratio tells the UI how representative these are.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptSemantics {
    pub classified_ratio: f64,
    pub requested_action: std::collections::BTreeMap<String, i64>,
    pub target_object: std::collections::BTreeMap<String, i64>,
    pub interaction_state: std::collections::BTreeMap<String, i64>,
    pub interaction_mode: std::collections::BTreeMap<String, i64>,
}

/// Prompt-dimension quality block (0-100 score + driver ratios + suggestions).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptQuality {
    pub score: i64,
    pub clarification_correction_rate: f64,
    pub planning_ratio: f64,
    pub test_object_ratio: f64,
    pub improvement_suggestions: Vec<String>,
}

/// The full prompt-dimension payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptDimension {
    pub penetration: PromptPenetration,
    #[serde(skip_serializing_if = "PromptMaturity::is_empty", default)]
    pub maturity: PromptMaturity,
    pub semantics: PromptSemantics,
    pub quality: PromptQuality,
}

impl PromptMaturity {
    fn is_empty(&self) -> bool {
        self.agent_coefficient_ms_per_prompt == 0 && self.total_tool_calls == 0
    }
}

/// Cost-efficiency leverage: the bridge between the token (cost) and prompt
/// (collaboration) dimensions. Denominator is variable cost ONLY (hard rule).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Leverage {
    pub est_cost_cny: f64,
    pub variable_cost_cny: f64,
    pub subscription_cost_cny: f64,
    pub fresh_tokens: i64,
    pub output_proxy: i64,
    pub leverage_per_cny: f64,
    pub cost_per_prompt_cny: f64,
}

/// The top-level window-metrics payload returned by `get_window_metrics`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMetrics {
    pub kind: String,
    pub ref_date: String,
    pub current_window: WindowRange,
    pub previous_window: WindowRange,
    pub yoy_window: WindowRange,
    pub has_previous_baseline: bool,
    pub has_yoy_baseline: bool,
    /// 9-scalar comparison table (sessions/tokens/cost/prompts/quality/...),
    /// each with current + optional mom/yoy delta.
    pub comparison: std::collections::BTreeMap<String, ComparisonEntry>,
    pub token_dimension: TokenDimension,
    pub prompt_dimension: PromptDimension,
    pub leverage: Leverage,
    /// PRD-08 §3.4/§3.5 (P1): entity-derived metrics for the tool-profile view.
    #[serde(default)]
    pub entity_metrics: EntityMetrics,
}

/// An ISO-date window range [start, end] inclusive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowRange {
    pub start: String,
    pub end: String,
}

// -----------------------------------------------------------------------------
// PRD-11: AI daily-summary value module
// -----------------------------------------------------------------------------

/// Metadata for one cached daily summary, returned by `list_daily_summaries`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySummaryMeta {
    pub date: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub created_at: u64,
}

/// Aggregated value metrics computed from cached daily summaries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryValueMetrics {
    pub covered_days: i64,
    pub total_summaries: i64,
    pub total_activities: i64,
    pub covered_projects: i64,
}

// -----------------------------------------------------------------------------
// PRD-08 §3.4/§3.5 (P1): entity-derived metrics (role profile, cost split,
// per-tool agent coefficient). Computed directly from the collected_* tables
// (single source of truth) — no separate entity tables needed.
// -----------------------------------------------------------------------------

/// Tool × requested_action row-normalized shares (Goal 1: each tool's main
/// role). Maps source -> action -> share (0..1). Mirrors `metrics.role_profile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleProfile {
    /// source -> { action: share }. A ★ in the UI marks any share ≥ 0.5.
    pub tools: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    /// True when the ≥0.6-confidence set was empty and we fell back to all
    /// labels. The UI shows a "low-confidence data" note when true.
    pub degraded: bool,
}

/// Cost split by billing mode (Goal 3: variable vs subscription). Variable cost
/// is real money and the ONLY thing feeding the leverage denominator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostSplit {
    pub variable_cny: f64,
    pub variable_calls: i64,
    pub subscription_calls: i64,
    /// "high" (>50 calls) / "medium" (>10) / "low" — subscription usage intensity.
    pub subscription_intensity: String,
}

/// Per-tool agent coefficient (Goal 2): avg prompt duration in minutes, only
/// over sources that persist trustworthy duration. Sources without trusted
/// duration are omitted entirely (not 0) — the fix for "zcode冒充全局".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCoefficientByTool {
    /// source -> avg minutes per prompt (trusted duration only).
    pub tools: std::collections::BTreeMap<String, f64>,
}

/// The full entity-metrics bundle (Goal 1+2+3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityMetrics {
    pub role_profile: RoleProfile,
    pub cost_split: CostSplit,
    pub agent_coefficient_by_tool: AgentCoefficientByTool,
}

/// PRD-09 §4: a local point-in-time snapshot of the Center Repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: String,
    pub created_at: u64,
    pub note: Option<String>,
    pub skill_count: i64,
}

/// PRD-09 §4: one Git commit entry returned by `git_versions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: u64,
}

// -----------------------------------------------------------------------------
// PRD-10: scheduled tasks
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    CollectUsageData,
    ScanAgents,
    SyncAllSkills,
    ImportAgentSkills,
    ScanAgentDirectories,
    ScanProjects,
    GenerateKnowledgeGraph,
    GenerateDailySummary,
    SyncRemoteSources,
    BackupCenterRepo,
    RunDiscoveryPipeline,
    GenerateWeeklyReport,
}

impl std::fmt::Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskKind::CollectUsageData => "collect_usage_data",
            TaskKind::ScanAgents => "scan_agents",
            TaskKind::SyncAllSkills => "sync_all_skills",
            TaskKind::ImportAgentSkills => "import_agent_skills",
            TaskKind::ScanAgentDirectories => "scan_agent_directories",
            TaskKind::ScanProjects => "scan_projects",
            TaskKind::GenerateKnowledgeGraph => "generate_knowledge_graph",
            TaskKind::GenerateDailySummary => "generate_daily_summary",
            TaskKind::SyncRemoteSources => "sync_remote_sources",
            TaskKind::BackupCenterRepo => "backup_center_repo",
            TaskKind::RunDiscoveryPipeline => "run_discovery_pipeline",
            TaskKind::GenerateWeeklyReport => "generate_weekly_report",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for TaskKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "collect_usage_data" => Ok(TaskKind::CollectUsageData),
            "scan_agents" => Ok(TaskKind::ScanAgents),
            "sync_all_skills" => Ok(TaskKind::SyncAllSkills),
            "import_agent_skills" => Ok(TaskKind::ImportAgentSkills),
            "scan_agent_directories" => Ok(TaskKind::ScanAgentDirectories),
            "scan_projects" => Ok(TaskKind::ScanProjects),
            "generate_knowledge_graph" => Ok(TaskKind::GenerateKnowledgeGraph),
            "generate_daily_summary" => Ok(TaskKind::GenerateDailySummary),
            "sync_remote_sources" => Ok(TaskKind::SyncRemoteSources),
            "backup_center_repo" => Ok(TaskKind::BackupCenterRepo),
            "run_discovery_pipeline" => Ok(TaskKind::RunDiscoveryPipeline),
            "generate_weekly_report" => Ok(TaskKind::GenerateWeeklyReport),
            other => Err(format!("unknown task kind: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalUnit {
    Minutes,
    Hours,
    Days,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleStrategy {
    Manual,
    Interval { value: u32, unit: IntervalUnit },
    Cron { expression: String },
}

impl Default for ScheduleStrategy {
    fn default() -> Self {
        ScheduleStrategy::Manual
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    Schedule,
    Manual,
    Startup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub task_kind: TaskKind,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub strategy: ScheduleStrategy,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_run_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_status: Option<RunStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_run_at: Option<u64>,
    pub run_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    pub status: RunStatus,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub duration_ms: Option<u64>,
    pub result_summary: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_message: Option<String>,
    pub triggered_by: TriggerSource,
}

// -----------------------------------------------------------------------------
// SPEC-I2: discovery inbox + weekly reports
// -----------------------------------------------------------------------------

/// Kind of a discovery candidate / inbox item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryKind {
    RepeatPattern,
    HighValuePrompt,
    SkillFeedback,
    CapabilityGap,
}

impl std::fmt::Display for DiscoveryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryKind::RepeatPattern => write!(f, "repeat_pattern"),
            DiscoveryKind::HighValuePrompt => write!(f, "high_value_prompt"),
            DiscoveryKind::SkillFeedback => write!(f, "skill_feedback"),
            DiscoveryKind::CapabilityGap => write!(f, "capability_gap"),
        }
    }
}

impl std::str::FromStr for DiscoveryKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "repeat_pattern" => Ok(DiscoveryKind::RepeatPattern),
            "high_value_prompt" => Ok(DiscoveryKind::HighValuePrompt),
            "skill_feedback" => Ok(DiscoveryKind::SkillFeedback),
            "capability_gap" => Ok(DiscoveryKind::CapabilityGap),
            other => Err(format!("unknown discovery kind: {}", other)),
        }
    }
}

/// Lifecycle status of a discovery item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryStatus {
    Pending,
    Accepted,
    Dismissed,
    Expired,
}

impl std::fmt::Display for DiscoveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryStatus::Pending => write!(f, "pending"),
            DiscoveryStatus::Accepted => write!(f, "accepted"),
            DiscoveryStatus::Dismissed => write!(f, "dismissed"),
            DiscoveryStatus::Expired => write!(f, "expired"),
        }
    }
}

/// A persisted discovery item in the inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub id: String,
    pub kind: DiscoveryKind,
    pub title: String,
    pub payload: serde_json::Value,
    pub confidence: f64,
    pub dedup_key: String,
    pub status: DiscoveryStatus,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub decided_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resulting_skill_id: Option<String>,
}

/// A weekly report (Monday-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyReport {
    pub week_start: String,
    pub content: WeeklyReportContent,
    pub generated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider: Option<String>,
}

/// Structured content of a weekly report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyReportContent {
    pub projects: Vec<WeeklyProjectSummary>,
    pub pitfalls: Vec<WeeklyPitfall>,
    pub growth: WeeklyGrowthSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyProjectSummary {
    pub project_id: String,
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyPitfall {
    pub session_id: String,
    pub project_id: Option<String>,
    pub title: String,
    pub lesson: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyGrowthSummary {
    pub new_skills: Vec<String>,
    pub eliminated_patterns: Vec<String>,
    pub accepted_count: i64,
    pub dismissed_count: i64,
}

/// Result of a discovery pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryRunResult {
    pub inserted: usize,
    pub expired: usize,
}

/// Result of deciding on a discovery item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryDecisionResult {
    pub discovery: Discovery,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_skill_id: Option<String>,
    /// SPEC-C1 T2: outcome of the adopt-and-sync direct path. Present only for
    /// `action="accept"`; `accept_edited` returns `created_skill_id` without
    /// syncing (the user must edit first).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sync_summary: Option<SyncSummary>,
}

/// SPEC-C1 T2: compact summary of an adopt-and-sync run. Failed agents keep
/// their `recovery_hint` so the inbox can surface a "已入库，N 个 Agent 同步失败"
/// affordance without rolling back the created skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSummary {
    pub success: usize,
    pub failed: usize,
    pub failures: Vec<SyncFailure>,
}

/// SPEC-C1 T4: a gate rejection ledger row. `payload` carries a compact
/// candidate summary (title/kind/confidence) for diagnostics and the
/// low_confidence inbox band.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateRejection {
    pub id: i64,
    pub reason: String,
    pub kind: String,
    pub confidence: f64,
    pub payload: serde_json::Value,
    pub created_at: u64,
}

/// SPEC-C3: a recycle-bin row. `expires_at` is the unix-seconds deadline after
/// which startup cleanup purges the row and its snapshot directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashItem {
    pub id: i64,
    pub item_type: String,
    pub original_id: String,
    pub original_name: String,
    pub snapshot_path: String,
    pub metadata: serde_json::Value,
    pub deleted_at: u64,
    pub expires_at: u64,
}

/// SPEC-C3 T2: outcome of restoring a trash item. When the restore used the
/// `rename` strategy or skipped stale project bindings, the caller surfaces the
/// details so the user knows what landed where.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreResult {
    pub restored_id: String,
    /// Project bindings that were skipped because their project no longer exists.
    pub skipped_bindings: Vec<String>,
    /// The name actually used after conflict resolution (rename strategy).
    pub final_name: String,
}

/// Growth metrics panel (four blocks).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrowthMetrics {
    pub skill_count_by_source: std::collections::BTreeMap<String, i64>,
    pub compounding_curves: Vec<CompoundingCurve>,
    pub active_skills: Vec<ActiveSkillMetric>,
    pub capability_map: Vec<CapabilityCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundingCurve {
    pub skill_id: String,
    pub skill_name: String,
    pub weeks: Vec<String>,
    pub counts: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSkillMetric {
    pub skill_id: Option<String>,
    pub skill_name: String,
    pub usage_count: i64,
    pub last_used_at: Option<u64>,
    pub dormant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCategory {
    pub category: String,
    pub has_skill: bool,
    pub prompt_count_7d: i64,
}

/// SPEC-I2: minimal session stub used to build weekly-report LLM context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStub {
    pub session_id: String,
    pub project_path: Option<String>,
    pub title: Option<String>,
    pub message_count: i64,
}
