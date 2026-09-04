/**
 * P3-4: typed wrappers + DTOs for the `npx skills` bridge commands
 * (commands/npxskills.rs). Single import surface for the pages that drive
 * installs, the index views, and the private hubs.
 */
import { invoke } from "../lib/invoke";

export interface AgentDef {
  key: string;
  display_name: string;
  project_dir: string;
  global_dir: string | null;
}

export interface NodeEnvInfo {
  bin_dir: string;
  node_version: string;
  problem: string | null;
}

export interface SkillIndexEntry {
  name: string;
  scope: "global" | "project" | "hub-global" | "hub-project" | string;
  managed_by: "npx" | "unmanaged" | "hub" | string;
  path: string;
  skill_md_path: string;
  source: string | null;
  source_url: string | null;
  source_type: string | null;
  ref_spec: string | null;
  hash: string;
  content_hash: string;
  status: "ok" | "modified" | "broken" | string;
  agents: string[];
  description: string | null;
  updated_at: number;
}

export interface IndexSummary {
  project_key: string;
  total: number;
  npx_global: number;
  npx_project: number;
  unmanaged: number;
  hub: number;
  modified: number;
  broken: number;
  fingerprint: string;
  rebuilt_at: number;
}

export interface InstallRequest {
  source: string;
  skills: string[];
  agents: string[];
  global: boolean;
  copy: boolean;
  project_root: string | null;
}

export interface NpxRunResult {
  success: boolean;
  exit_code: number | null;
  output: string;
  command: string;
  timed_out: boolean;
}

export interface SkillSafetyReport {
  name: string;
  path: string;
  findings: number;
}

export interface InstallResult {
  run: NpxRunResult;
  safety: SkillSafetyReport[];
  audit_id: string | null;
  index: IndexSummary | null;
}

export interface SearchHit {
  id: string;
  name: string;
  installs: number;
  source: string;
}

export interface HubStatus {
  scope: string;
  path: string;
  exists: boolean;
  git_inited: boolean;
  skill_count: number;
  remote: string | null;
  branch: string;
  ahead: number;
  behind: number;
  dirty_files: number;
  last_commit: { hash: string; message: string; time: string } | null;
}

export interface ProjectRow {
  project_id: string;
  name: string;
  path: string;
}

// ---- runtime / table ------------------------------------------------------

export function getNpxEnv() {
  return invoke<NodeEnvInfo>("npx_env");
}

export function getAgentsTable() {
  return invoke<AgentDef[]>("get_agents_table");
}

// ---- install / remove / update --------------------------------------------

export function previewInstallCommand(req: InstallRequest) {
  return invoke<string>("npx_preview_command", { req });
}

export function npxInstall(req: InstallRequest) {
  return invoke<InstallResult>("npx_install", { req });
}

export function npxRemove(skill: string, agents: string[], global: boolean, projectRoot: string | null) {
  return invoke<NpxRunResult>("npx_remove", { skill, agents, global, projectRoot });
}

export function npxUpdate(global: boolean, skill: string | null, projectRoot: string | null) {
  return invoke<NpxRunResult>("npx_update", { global, skill, projectRoot });
}

// ---- search ---------------------------------------------------------------

export function skillsSearch(query: string, limit?: number) {
  return invoke<SearchHit[]>("skills_search", { query, limit });
}

// ---- index ----------------------------------------------------------------

export function rebuildSkillIndex(projectRoot: string | null) {
  return invoke<IndexSummary>("rebuild_skill_index", { projectRoot });
}

export function getSkillIndex(projectRoot: string | null, scopes?: string[]) {
  return invoke<SkillIndexEntry[]>("get_skill_index", { projectRoot, scopes });
}

export function skillIndexStale(projectRoot: string | null) {
  return invoke<boolean>("skill_index_stale", { projectRoot });
}

export function readSkillIndexContent(skillMdPath: string) {
  return invoke<string>("read_skill_index_content", { skillMdPath });
}

// ---- private hub ----------------------------------------------------------

export function hubCreateSkill(scope: string, projectRoot: string | null, name: string, description: string) {
  return invoke<string>("hub_create_skill", { scope, projectRoot, name, description });
}

export function hubCollectSkill(scope: string, projectRoot: string | null, source: string, name?: string) {
  return invoke<string>("hub_collect_skill", { scope, projectRoot, source, name: name ?? null });
}

export function hubStatus(scope: string, projectRoot: string | null) {
  return invoke<HubStatus>("hub_status_cmd", { scope, projectRoot });
}

export function hubPush(scope: string, projectRoot: string | null) {
  return invoke<string>("hub_push", { scope, projectRoot });
}

export function hubSetRemote(scope: string, projectRoot: string | null, url: string) {
  return invoke<void>("hub_set_remote", { scope, projectRoot, url });
}

export function hubInstallSource(scope: string, projectRoot: string | null) {
  return invoke<string>("hub_install_source", { scope, projectRoot });
}

export function hubListSkills(scope: string, projectRoot: string | null) {
  return invoke<SkillIndexEntry[]>("hub_list_skills", { scope, projectRoot });
}

// ---- legacy helpers reused by the new flows -------------------------------

export function getProjects() {
  return invoke<ProjectRow[]>("get_projects");
}

export function openPathInTerminal(path: string) {
  return invoke<void>("open_path_in_terminal", { path });
}
