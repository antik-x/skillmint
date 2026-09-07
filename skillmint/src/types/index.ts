export type SyncMode = "symlink" | "copy";
export type SyncStatus =
  | "synced"
  | "local_changed"
  | "center_changed"
  | "conflict"
  | "broken";

export type SkillStatus = "draft" | "candidate" | "approved" | "deprecated";

export interface Skill {
  id: string;
  name: string;
  repo_path: string;
  created_at: number;
  updated_at: number;
  /** P1-3: lifecycle status. Defaults to draft. */
  status: SkillStatus;
}

export interface Agent {
  id: string;
  name: string;
  skill_directory: string;
  is_enabled: boolean;
  discovery_rule?: string;
  description?: string;
  /** PRD-06: collection normalization key (e.g. "claude-code", "zcode"). Drives
   * deterministic attribution of usage data to this Agent. */
  source?: string;
}

export interface SyncTarget {
  id: string;
  skill_id: string;
  skill_name?: string;
  agent_id: string;
  agent_name?: string;
  mode: SyncMode;
  last_sync_at?: number;
  status: SyncStatus;
}

export interface AgentSkillItem {
  name: string;
  exists_in_center: boolean;
  content_match?: boolean;
}

export interface ConflictContent {
  sync_target_id: string;
  skill_name: string;
  agent_name: string;
  center_content: string;
  local_content: string;
  /** P1-3: center skill last updated timestamp. */
  center_updated_at?: number;
  /** P1-3: local agent copy last modified timestamp. */
  local_updated_at?: number;
  /** P1-3: center version author, if known. */
  center_author?: string;
  /** P1-3: local version author, if known. */
  local_author?: string;
}

export type ConflictResolution = "keep_center" | "keep_local" | "skip";

/** P2-2: one core table description in the data dictionary. */
export interface TableInfo {
  name: string;
  description: string;
  columns: string[];
}

/** P2-2: data dictionary payload. */
export interface DataDictionary {
  db_path: string;
  tables: TableInfo[];
}

/** P2-2: raw-data export payload. */
export interface RawDataExport {
  format: string;
  exported_at: number;
  tables: string[];
  data: Record<string, unknown>;
}

export interface SyncFailure {
  target_id: string;
  skill_id: string;
  skill_name?: string;
  agent_id: string;
  agent_name?: string;
  error: string;
  recovery_hint?: string;
}

export interface SyncAllResult {
  targets: SyncTarget[];
  imported_skills: number;
  import_conflicts: number;
  success_count: number;
  failure_count: number;
  failures: SyncFailure[];
}

export type RepoIntegrity = "healthy" | "missing_with_records" | "orphans_present";

// PRD-02: usage data collection (mirrors Rust models.rs)
export interface CollectedSource {
  source: string;
  collector_kind: string;
  data_path: string;
  status: string; // ok | not_found | error | schema_incompatible | unsupported
  last_collected_at?: number;
  record_count: number;
}

export interface AgentUsageSummary {
  source: string;
  days: number;
  session_count: number;
  prompt_count: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens: number;
  cache_creation_input_tokens: number;
}

export interface CollectionStats {
  source: string;
  sessions: number;
  prompts: number;
  token_rows: number;
  skipped: number;
}

export interface CollectionJob {
  id: string;
  started_at: number;
  completed_at?: number;
  status: "pending" | "running" | "completed" | "failed" | "cancelled";
  progress_json: string;
  result_json?: string;
  error?: string;
}

export interface AppSettings {
  device_id: string;
  auto_sync_interval_minutes: number;
  launch_at_login: boolean;
  show_dock_icon: boolean;
  onboarding_completed: boolean;
  /** P0-3: user-configured extra scan/import exclusion names (settings.json). */
  scan_exclude_names?: string[];
  /** PRD-07: master switch for all remote (network) features. Default false. */
  remote_enabled: boolean;
  /** PRD-08 §3.6: optional LLM config for prompt classification + daily summaries. */
  ai: AiConfig;
  /** M1: appearance theme. system follows OS preference. */
  theme: "light" | "dark" | "system";
  /** P3: npm package spec invoked as `npx -y <spec>`; default `skills@latest`. */
  npx_package?: string;
  /** P3: mirror for the skills.sh search API (China acceleration). */
  skills_api_url?: string;
  /** P3: proxy URL injected into npx child processes. */
  proxy_env?: string;
  /** P3: inject DISABLE_TELEMETRY=1 into npx (default true). */
  disable_telemetry?: boolean;
  /** P3: explicit node bin dir for GUI PATH limitations. */
  node_path_override?: string;
  /** P4: OpenViking context-database integration (default off). */
  openviking?: OpenVikingConfig;
}

/** P4: OpenViking integration config. `api_key` lives in the Keychain. */
export interface OpenVikingConfig {
  /** Independent opt-in; combined with `remote_enabled` gates every request. */
  enabled: boolean;
  /** Base URL of the local OpenViking HTTP server. */
  base_url: string;
  /** In-memory only; persisted to the platform credential store by the backend. */
  api_key?: string;
}

/** PRD-08: optional AI/LLM configuration (OpenAI-compatible or Anthropic). */
export interface AiConfig {
  models: AiModelConfig[];
  acp_connections: AcpConnectionConfig[];
  default_chat_model_id?: string;
  default_embedding_model_id?: string;
  prefer_acp: boolean;
  /** P0: when true, LLM analysis never falls back to cloud providers. */
  strict_local_mode: boolean;
}

/** P0: audit log entry for an LLM/ACP request. */
export interface LlmRequestLog {
  id: string;
  requested_at: number;
  provider: string;
  fallback: boolean;
  has_raw_text: boolean;
  error?: string;
  metadata_json: string;
}

/** One configured LLM endpoint (chat and/or embedding). */
export interface AiModelConfig {
  id: string;
  name: string;
  /** "openai" (default) or "anthropic". */
  provider: string;
  model: string;
  base_url?: string;
  api_key?: string;
  capabilities: ("chat" | "embedding")[];
}

/** One configured ACP (Agent Communication Protocol) connection. */
export interface AcpConnectionConfig {
  id: string;
  name: string;
  enabled: boolean;
  transport: AcpTransport;
}

export type AcpTransport =
  | { kind: "stdio"; command: string; args: string[]; env: Record<string, string> }
  | { kind: "sse"; url: string; headers: Record<string, string> };

/** P1-1: a locally installed agent CLI detected on PATH. */
export interface DetectedAgent {
  command: string;
  display_name: string;
  args: string[];
  /** 登录/前置条件提示（Q7）。 */
  login_hint?: string;
}

/** P5/Q7: 每条 ACP 连接的健康状态（最近调用结果）。 */
export interface AcpHealthEntry {
  id: string;
  /** "ok" | "cooldown" | "error" | "idle"（从未调用） */
  state: string;
  detail: string;
  /** 冷却剩余秒数（0 = 不在冷却）。 */
  cooldown_remaining: number;
}

// PRD-01: project + bindings
export interface ProjectUsageSummary {
  project_id: string;
  name: string;
  path: string;
  session_count: number;
  total_tokens: number;
}

export interface AgentDirectory {
  id: string;
  agent_id: string;
  path: string;
  /** Free-form role label: "skills" / "commands" / "rules" / "instructions" / ... */
  role?: string;
  is_enabled: boolean;
  created_at: number;
}

export interface AgentDetail {
  id: string;
  name: string;
  skill_directory: string;
  description?: string;
  is_enabled: boolean;
  last_used_at?: number;
  project_count: number;
  source?: string;
  usage_7d?: AgentUsageSummary;
  projects: ProjectUsageSummary[];
  /** PRD-06 §3.3: all directories owned by this Agent (1:N). */
  directories?: AgentDirectory[];
}

export interface SkillProjectBinding {
  id: string;
  device_id: string;
  skill_id: string;
  skill_name?: string;
  project_id?: string;
  project_name?: string;
  agent_id?: string;
  mode: string;
  local_path?: string;
  is_enabled: boolean;
  pinned_version?: string;
}

// PRD-01 patch: project detail page + multi-version skills
export interface ProjectAgentEntry {
  agent_id: string;
  agent_name: string;
  skill_directory: string;
  is_enabled: boolean;
  last_session_at?: number;
  session_count: number;
  total_tokens: number;
  skill_count?: number;
}

export interface ProjectDetail {
  project_id: string;
  name: string;
  path: string;
  last_active_at?: number;
  session_count: number;
  total_tokens: number;
  agents: ProjectAgentEntry[];
}

export type SkillSource = "symlink" | "local_copy" | "local" | "broken";

export interface ResolvedSkill {
  name: string;
  source: string; // SkillSource serialized
  target_path?: string;
  content_match?: boolean;
  is_registered: boolean;
  pinned_version?: string;
}

export interface SkillVersion {
  version: string; // "latest" | "v1" | ...
  created_at: number;
  note?: string;
  pinned_by: string[]; // project names pinning to this version (PRD §4.5d)
}

export type DiffStrategy =
  | "keep_center"
  | "keep_project"
  | "keep_project_backup"
  | "versionize";

export interface ResolveResult {
  strategy: string;
  new_version?: string;
  project_path: string;
  latest_path: string;
}

/** G2: result of rolling back a skill to a historical version. */
export interface RollbackResult {
  saved_as: string;
  rolled_to: string;
}

// PRD-02: prompt-driven generation + reports
export interface HighValuePrompt {
  prompt_text: string;
  repeat_count: number;
  sample_session_id: string;
  source?: string;
}

/** 「高频使用 Prompt」分页结果（分组键 = prompt 前 120 字符 + source）。 */
export interface HighValuePromptPage {
  items: HighValuePrompt[];
  total: number;
}

/** 用户忽略的高频 Prompt 分组（永久生效，可在列表底部恢复）。 */
export interface IgnoredPromptGroup {
  group_key: string;
  source?: string;
  prompt_sample?: string;
  created_at: number;
}

export interface SkillSuggestionReport {
  generated_at: number;
  top_prompts: HighValuePrompt[];
  underused_skills: string[];
  top_skills: string[];
}

export interface SkillUsageSummary {
  skill_name: string;
  skill_id?: string;
  usage_count: number;
  session_count: number;
  project_count: number;
}

// PRD-03: knowledge graph
export interface KgNode {
  id: string;
  label: string;
  type: string;
  source?: string;
  description?: string;
}

export interface KgEdge {
  id: string;
  device_id: string;
  source_id: string;
  target_id: string;
  relation: string;
  weight?: number;
  reason?: string;
  is_manual: boolean;
  is_rejected: boolean;
}

export interface KgGraph {
  nodes: KgNode[];
  edges: KgEdge[];
}

export interface SkillRecommendation {
  skill_id: string;
  skill_name: string;
  reason: string;
  matched_concepts: string[];
}

export interface TaskRecommendation {
  recommendations: SkillRecommendation[];
  gaps: string[];
}

/** Outcome of restoring a Center Repo zip backup (PRD-0 §4.8). */
export interface RestoreSummary {
  imported: string[];
  skipped: string[];
}

/** PRD-09 §4: a local point-in-time snapshot of the Center Repo. */
export interface SnapshotInfo {
  id: string;
  created_at: number;
  note?: string;
  skill_count: number;
}

/** PRD-09 §4: one Git commit entry returned by `git_versions`. */
export interface GitCommit {
  sha: string;
  message: string;
  author: string;
  timestamp: number;
}

// =============================================================================
// PRD-07: remote sources, unified discovery, safety scan
// =============================================================================

export type SourceType = "github" | "git" | "local";

/** A connected remote source (PRD-07 §3.1). */
export interface Source {
  id: string;
  name: string;
  source_type: SourceType;
  /** github: owner/repo; git: URL; local: path. */
  url: string;
  ref_spec: string;
  subpath: string;
  cache_path: string;
  commit_sha: string;
  added_at: number;
  last_fetched_at?: number;
  pull_policy: string;
  remote_revision: string;
}

/** One skill discovered inside a source's cache (PRD-07 §3.1/§3.3). */
export interface SkillRemoteMeta {
  skill_name: string;
  source_id: string;
  skill_path: string;
  description?: string;
  computed_hash?: string;
  installed_locally: boolean;
}

/** Result of scanning a skill's SKILL.md for high-risk instructions (PRD-07 §3.2). */
export interface SafetyFinding {
  line: number;
  rule: string;
  excerpt: string;
}

export interface SafetyScanResult {
  clean: boolean;
  findings: SafetyFinding[];
}

/** One row of the unified search result (PRD-07 §3.3). */
export interface SearchResult {
  skill_name: string;
  origin: "local" | "remote";
  source_id?: string;
  source_name?: string;
  description?: string;
  installed_locally: boolean;
  usage_count: number;
  /** Number of clarification/correction prompts tied to this skill (PRD-09 §3.3c). */
  correction_count: number;
  /** For remote results: skill dir relative to the source cache root. Needed
   * to locate the skill on install/preview when a source uses a subpath. */
  skill_path: string;
  /** Which field produced the match: "name" | "description" | "tags" | "body". */
  match_field?: string;
  /** Context snippet when the match came from the SKILL.md body, with the
   * matched term wrapped in «» for frontend highlighting. */
  snippet?: string;
}

/** Outcome of installing a remote skill (PRD-07 §3.2). */
export interface InstallRemoteResult {
  skill: Skill;
  synced_agents: string[];
  /** Agent names where a same-named skill already existed and was skipped. */
  skipped_agents?: string[];
}

// =============================================================================
// PRD-02 Phase 2: skill bundles (named reusable collections of skills)
// =============================================================================

export interface SkillBundle {
  id: string;
  name: string;
  description?: string;
  skill_count: number;
  created_at: number;
  updated_at: number;
}

export interface SkillBundleItem {
  id: string;
  skill_id: string;
  skill_name: string;
  sort_order: number;
}

export interface ApplyBundleResult {
  applied: string[];
  skipped: [string, string][]; // [skill_name, reason]
}

export interface BundleExport {
  name: string;
  description?: string;
  exported_at: number;
  skills: BundleExportSkill[];
}

export interface BundleExportSkill {
  name: string;
  required: boolean;
}

// =============================================================================
// PRD-08: full analysis integration (mirrors Rust WindowMetrics / dimensions).
// These are the shapes returned by the `get_window_metrics` command.
// =============================================================================

/** A window-comparison delta vs a baseline (previous period or YoY).
 * `pct` is null when the baseline is 0 (cannot divide). */
export interface Delta {
  current: number;
  previous: number;
  delta: number;
  pct?: number | null;
  note?: string;
}

/** One metric in the comparison table: current value + optional mom/yoy. */
export interface ComparisonEntry {
  current: number;
  mom?: Delta | null;
  yoy?: Delta | null;
}

/** Inclusive ISO-date window range [start, end]. */
export interface WindowRange {
  start: string;
  end: string;
}

export interface TokenScale {
  total_tokens: number;
  fresh_tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  model_calls: number;
  tool_calls: number;
  duration_hours: number;
}

export interface TokenDistribution {
  by_platform: Record<string, number>;
  by_project: Record<string, number>;
  by_model: Record<string, number>;
}

export interface TokenEfficiency {
  tokens_per_session: number;
  fresh_tokens_per_session: number;
  tokens_per_call: number;
  tokens_per_hour: number;
}

export interface HeavySession {
  session_id: string;
  project?: string | null;
  model?: string | null;
  total_tokens: number;
  cache_read: number;
  model_calls: number;
}

export interface TokenCost {
  est_cost_cny: number;
  by_platform_cny: Record<string, number>;
  billing_mix: Record<string, number>;
}

export interface TokenDimension {
  scale?: TokenScale;
  distribution: TokenDistribution;
  efficiency?: TokenEfficiency;
  cost: TokenCost;
  diagnostics: {
    cache_ratio: number;
    heavy_sessions: HeavySession[];
  };
}

export interface PromptPenetration {
  total_prompts: number;
  by_platform: Record<string, number>;
  by_project: Record<string, number>;
}

export interface PromptMaturity {
  agent_coefficient_ms_per_prompt: number;
  agent_coefficient_min_per_prompt: number;
  avg_tool_calls_per_prompt: number;
  total_tool_calls: number;
}

export interface PromptSemantics {
  classified_ratio: number;
  requested_action: Record<string, number>;
  target_object: Record<string, number>;
  interaction_state: Record<string, number>;
  interaction_mode: Record<string, number>;
}

export interface PromptQuality {
  score: number;
  clarification_correction_rate: number;
  planning_ratio: number;
  test_object_ratio: number;
  improvement_suggestions: string[];
}

export interface PromptDimension {
  penetration: PromptPenetration;
  maturity?: PromptMaturity;
  semantics: PromptSemantics;
  quality: PromptQuality;
}

export interface Leverage {
  est_cost_cny: number;
  variable_cost_cny: number;
  subscription_cost_cny: number;
  fresh_tokens: number;
  output_proxy: number;
  leverage_per_cny: number;
  cost_per_prompt_cny: number;
}

/** The top-level window-metrics payload from `get_window_metrics`. */
export interface WindowMetrics {
  kind: "day" | "week" | "month";
  ref_date: string;
  current_window: WindowRange;
  previous_window: WindowRange;
  yoy_window: WindowRange;
  has_previous_baseline: boolean;
  has_yoy_baseline: boolean;
  comparison: Record<string, ComparisonEntry>;
  token_dimension: TokenDimension;
  prompt_dimension: PromptDimension;
  leverage: Leverage;
  /** PRD-08 §3.4/§3.5 (P1): entity-derived metrics (tool profile view). */
  entity_metrics?: EntityMetrics;
}

/** Tool × action row-normalized shares (Goal 1: each tool's main role). */
export interface RoleProfile {
  tools: Record<string, Record<string, number>>;
  degraded: boolean;
}

export interface CostSplit {
  variable_cny: number;
  variable_calls: number;
  subscription_calls: number;
  subscription_intensity: string;
}

export interface AgentCoefficientByTool {
  tools: Record<string, number>;
}

export interface EntityMetrics {
  role_profile: RoleProfile;
  cost_split: CostSplit;
  agent_coefficient_by_tool: AgentCoefficientByTool;
}

/** Outcome of the classifier run (counts + whether it was skipped). */
export interface ClassifyResult {
  eligible: number;
  classified: number;
  skipped_no_key: boolean;
}

/** One activity within a daily summary. */
export interface ActivityItem {
  time_range: string;
  project: string;
  category: string;
  summary: string;
  details: string[];
}

export interface DailySummary {
  date: string;
  highlights: string[];
  activities: ActivityItem[];
}

/** Outcome of generate_daily_summary (tagged union on `status`). */
export type SummaryOutcome =
  | { status: "generated"; summary: DailySummary }
  | { status: "no_key" }
  | { status: "no_sessions" }
  | { status: "failed"; reason: string };

/** P-今天: outcome of ensure_daily_summary (idempotent refresh for the dual story cards). */
export type EnsureSummaryOutcome =
  | { status: "fresh" }
  | { status: "generated"; summary: DailySummary }
  | { status: "no_sessions" }
  | { status: "failed"; reason: string };

/** M1: a discovery candidate surfaced by the analysis pipeline. */
export type DiscoveryKind =
  | "repeat_pattern"
  | "high_value_prompt"
  | "skill_feedback"
  | "capability_gap";

export interface DiscoveryEvidence {
  session_id?: string;
  prompt_text?: string;
  started_at?: number;
  project?: string;
  source?: string;
}

export interface DiscoveryDraftSkill {
  name: string;
  body: string;
}

export interface Discovery {
  id: string;
  kind: DiscoveryKind;
  title: string;
  payload: {
    evidence: DiscoveryEvidence[];
    stats?: Record<string, number>;
    note?: string;
    draft_skill?: DiscoveryDraftSkill;
  };
  confidence: number;
  status: "pending" | "accepted" | "dismissed" | "expired";
  created_at: number;
  decided_at?: number;
}

/** SPEC-C1 T2: compact sync summary returned by the adopt-and-sync path. */
export interface SyncSummary {
  success: number;
  failed: number;
  failures: SyncFailure[];
}

/** Result of `decide_discovery(id, action, reason?)`. */
export interface DecideDiscoveryResult {
  created_skill_id?: string | null;
  /** Present only for `action="accept"` (the adopt-and-sync direct path). */
  sync_summary?: SyncSummary | null;
  discovery?: Discovery;
  mock?: boolean;
  note?: string;
}

/** Dismiss reason values accepted by `decide_discovery(..., "dismiss", reason)`. */
export type DismissReason = "wrong" | "trivial" | "duplicate";

/** SPEC-C1 T4: a gate-rejection ledger row. */
export interface GateRejection {
  id: number;
  reason: "below_threshold" | "daily_limit" | "cooling" | "rule_gate";
  kind: string;
  confidence: number;
  payload: Record<string, unknown>;
  created_at: number;
}

/** SPEC-C3: a recycle-bin row. The frontend computes remaining days from
 * `expires_at` and `deleted_at`. */
export interface TrashItem {
  id: number;
  item_type: "skill";
  original_id: string;
  original_name: string;
  snapshot_path: string;
  metadata: Record<string, unknown>;
  deleted_at: number;
  expires_at: number;
}

/** SPEC-C3 T2: outcome of restoring a trash item. */
export interface RestoreResult {
  restored_id: string;
  /** Project bindings that were skipped because their project no longer exists. */
  skipped_bindings: string[];
  /** The name actually used after conflict resolution (rename strategy). */
  final_name: string;
}

/** PRD-11: metadata for one cached daily summary. */
export interface DailySummaryMeta {
  date: string;
  model?: string;
  provider?: string;
  created_at: number;
}

/** PRD-11: aggregate value metrics from cached daily summaries. */
export interface SummaryValueMetrics {
  covered_days: number;
  total_summaries: number;
  total_activities: number;
  covered_projects: number;
}

/** PRD-08 §3.8 (P2): AI-Digest digest.db detection preview. */
export interface DigestDbPreview {
  path: string;
  sessions: number;
  prompts: number;
  token_usage: number;
}

/** PRD-08 §3.8 (P2): outcome of importing a digest.db. */
export interface DigestImportSummary {
  sessions: number;
  prompts: number;
  token_rows: number;
}

/** PRD-08 §3.4 (P3): one prompt backing a heatmap cell drill-down. */
export interface CellPrompt {
  prompt_text: string;
  started_at?: number | null;
  requested_action?: string | null;
  target_object?: string | null;
  interaction_state?: string | null;
  confidence?: number | null;
}

// =============================================================================
// PRD-10: scheduled tasks
// =============================================================================

export type TaskKind =
  | "collect_usage_data"
  | "scan_agents"
  | "sync_all_skills"
  | "import_agent_skills"
  | "scan_agent_directories"
  | "scan_projects"
  | "generate_knowledge_graph"
  | "generate_daily_summary"
  | "sync_remote_sources"
  | "backup_center_repo";

export type IntervalUnit = "minutes" | "hours" | "days";

export interface ScheduleStrategy {
  kind: "manual" | "interval" | "cron";
  value?: number;
  unit?: IntervalUnit;
  expression?: string;
}

export type RunStatus = "pending" | "running" | "success" | "failed" | "skipped";
export type TriggerSource = "schedule" | "manual" | "startup";

export interface ScheduledTask {
  id: string;
  task_kind: TaskKind;
  name: string;
  description: string;
  enabled: boolean;
  strategy: ScheduleStrategy;
  created_at: number;
  updated_at: number;
  last_run_at?: number;
  last_status?: RunStatus;
  next_run_at?: number;
  run_count: number;
  error_count: number;
}

export interface TaskRun {
  id: string;
  task_id: string;
  status: RunStatus;
  started_at: number;
  finished_at?: number;
  duration_ms?: number;
  result_summary: string;
  error_message?: string;
  triggered_by: TriggerSource;
}

// =============================================================================
// SPEC-I5/I6: discovery decisions + growth metrics + weekly report
// =============================================================================

export interface CompoundingCurve {
  skill_id: string;
  skill_name: string;
  weeks: string[];
  counts: number[];
}

export interface ActiveSkill {
  skill_id?: string;
  skill_name: string;
  usage_count: number;
  last_used_at?: number;
  dormant: boolean;
}

export interface CapabilityMapItem {
  category: string;
  skill_count?: number;
  active?: boolean;
  has_skill?: boolean;
  prompt_count_7d?: number;
}

export interface GrowthMetrics {
  skill_count_by_source: Record<string, number>;
  compounding_curves: CompoundingCurve[];
  active_skills: ActiveSkill[];
  capability_map: CapabilityMapItem[];
}

export interface WeeklyReportProject {
  project_id: string;
  name: string;
  summary: string;
}

export interface WeeklyReportPitfall {
  session_id: string;
  project_id?: string;
  title: string;
  lesson: string;
}

export interface WeeklyReportGrowth {
  new_skills?: string[];
  eliminated_patterns?: string[];
  accepted_count?: number;
  dismissed_count?: number;
}

export interface WeeklyReport {
  week_start: string;
  content: {
    projects: WeeklyReportProject[];
    pitfalls: WeeklyReportPitfall[];
    growth: WeeklyReportGrowth;
  };
  generated_at: number;
  model?: string | null;
  provider?: string;
}
