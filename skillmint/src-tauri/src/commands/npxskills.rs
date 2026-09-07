//! P3-1/P3-3/P3-4/P3-5: Tauri command surface for the `npx skills` bridge —
//! install/remove/update (always through the real CLI), lock-driven index
//! rebuilds, skills.sh search, and private-hub management.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};

use super::AppState;
use super::CmdTimer;
use crate::hub::{self, HubStatus};
use crate::index::{self, IndexSummary, SkillIndexEntry};
use crate::npx::{
    self, build_add_args, build_remove_args, build_update_args, equivalent_command, ListedSkill,
    NpxConfig, NpxRunResult, NodeEnv, SkillsInvocation,
};

fn npx_config(settings: &crate::settings::Settings) -> NpxConfig {
    NpxConfig {
        package_spec: if settings.npx_package.trim().is_empty() {
            "skills@latest".to_string()
        } else {
            settings.npx_package.trim().to_string()
        },
        skills_api_url: settings.skills_api_url.trim().to_string(),
        proxy: settings.proxy_env.trim().to_string(),
        disable_telemetry: settings.disable_telemetry,
        node_path_override: settings.node_path_override.trim().to_string(),
    }
}

fn project_root_opt(project_root: Option<String>) -> Option<PathBuf> {
    project_root
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

fn project_key(project_root: &Option<PathBuf>) -> String {
    project_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "-".into())
}

// ---------------------------------------------------------------------------
// Runtime / table
// ---------------------------------------------------------------------------

/// Node/npx runtime detection result for the settings + install panels.
#[tauri::command]
pub fn npx_env(state: State<'_, AppState>) -> Result<NodeEnv, String> {
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    Ok(npx::detect_node_env(&config))
}

/// The static agent matrix for the install panel's agent multi-select.
#[tauri::command]
pub fn get_agents_table() -> Vec<AgentDefDto> {
    crate::agents_table::AGENTS
        .iter()
        .map(|a| AgentDefDto {
            key: a.key.to_string(),
            display_name: a.display_name.to_string(),
            project_dir: a.project_dir.to_string(),
            global_dir: a.global_dir.map(|g| g.to_string()),
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentDefDto {
    pub key: String,
    pub display_name: String,
    pub project_dir: String,
    pub global_dir: Option<String>,
}

// ---------------------------------------------------------------------------
// Install / remove / update / list (always via the CLI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InstallRequest {
    /// `owner/repo`, full git URL, or a local path (private hub).
    pub source: String,
    /// Skill names inside the source; empty = interactive selection is NOT
    /// allowed from the GUI, the caller must resolve names first.
    pub skills: Vec<String>,
    /// Agent CLI keys (empty = all agents the CLI auto-detects via `*`).
    pub agents: Vec<String>,
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub copy: bool,
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallResult {
    pub run: NpxRunResult,
    /// Safety findings per installed skill (SKILL.md body scan).
    pub safety: Vec<SkillSafetyReport>,
    pub audit_id: Option<String>,
    pub index: Option<IndexSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillSafetyReport {
    pub name: String,
    pub path: String,
    pub findings: usize,
}

fn resolve_agents(agents: &[String]) -> Vec<String> {
    if !agents.is_empty() {
        return agents.to_vec();
    }
    // Empty selection = "all detected agents": the CLI's `*` would also install
    // into agents the user never uses, so we intersect the matrix with what
    // exists on this machine instead.
    let mut keys: Vec<String> = Vec::new();
    for a in crate::agents_table::AGENTS {
        let exists = a
            .global_dir
            .map(|g| crate::agents_table::expand_global(g).is_dir())
            .unwrap_or(false);
        if exists {
            keys.push(a.key.to_string());
        }
    }
    keys
}

/// Install via `npx skills add`. Streams live output on the `npx-output` event,
/// rebuilds the index, runs the SKILL.md safety scan on what landed, and
/// records the install_audit row.
#[tauri::command(async)]
pub fn npx_install(app: AppHandle, state: State<'_, AppState>, req: InstallRequest) -> Result<InstallResult, String> {
    let _t = CmdTimer::new("npx_install");
    if req.skills.is_empty() {
        return Err("未指定要安装的 skill（GUI 安装必须显式指定，避免交互卡死）".into());
    }
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    let project_root = project_root_opt(req.project_root.clone());
    let agents = resolve_agents(&req.agents);
    let invocation = SkillsInvocation {
        args: build_add_args(&req.source, &req.skills, &agents, req.global, req.copy),
        project_root: project_root.clone(),
        timeout_secs: 600,
    };
    let run = npx::run_skills(&config, &invocation, |line| {
        let _ = app.emit("npx-output", line.to_string());
    })
    .map_err(|e| e.to_string())?;

    // Post-install bookkeeping: refresh the index, scan what landed, audit.
    let mut safety = Vec::new();
    let mut audit_id = None;
    let index_summary = if run.success {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let summary = index::rebuild(&db, project_root.as_deref()).map_err(|e| e.to_string())?;
        let mut findings_total: Vec<crate::models::SafetyFinding> = Vec::new();
        for skill in &req.skills {
            // Find the installed path: global canonical or project canonical.
            let candidates = if req.global {
                vec![crate::skills_lock::global_canonical_path().join(skill)]
            } else if let Some(root) = &project_root {
                vec![root.join(crate::skills_lock::PROJECT_CANONICAL_DIR).join(skill)]
            } else {
                Vec::new()
            };
            for path in candidates {
                let md = path.join("SKILL.md");
                if md.is_file() {
                    if let Ok(body) = std::fs::read_to_string(&md) {
                        let result = crate::remote::scan_safety(&body);
                        let n = result.findings.len();
                        if n > 0 {
                            findings_total.extend(result.findings.clone());
                        }
                        safety.push(SkillSafetyReport {
                            name: skill.clone(),
                            path: path.to_string_lossy().to_string(),
                            findings: n,
                        });
                    }
                }
            }
        }
        let id = crate::db::new_id();
        let _ = db.log_install_audit(
            &id,
            &req.skills.join(","),
            &req.source,
            &findings_total,
            &[],
            false,
            chrono::Utc::now().timestamp_millis() as u64,
        );
        audit_id = Some(id);
        Some(summary)
    } else {
        None
    };

    Ok(InstallResult {
        run,
        safety,
        audit_id,
        index: index_summary,
    })
}

/// Remove via `npx skills remove`, snapshotting the canonical folder to the
/// trash first (review-first deletion).
#[tauri::command(async)]
pub fn npx_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    skill: String,
    agents: Vec<String>,
    global: bool,
    project_root: Option<String>,
) -> Result<NpxRunResult, String> {
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    let project_root = project_root_opt(project_root);
    // Trash snapshot of the canonical folder before the CLI touches it.
    {
        let canonical = if global {
            crate::skills_lock::global_canonical_path().join(&skill)
        } else if let Some(root) = &project_root {
            root.join(crate::skills_lock::PROJECT_CANONICAL_DIR).join(&skill)
        } else {
            return Err("项目级卸载必须提供 project_root".into());
        };
        if canonical.is_dir() {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let _ = hub::snapshot_to_trash(&db, &skill, &canonical);
        }
    }
    let invocation = SkillsInvocation {
        args: build_remove_args(&skill, &agents, global),
        project_root,
        timeout_secs: 300,
    };
    let run = npx::run_skills(&config, &invocation, |line| {
        let _ = app.emit("npx-output", line.to_string());
    })
    .map_err(|e| e.to_string())?;
    Ok(run)
}

/// Update via `npx skills update` (whole scope or one skill).
#[tauri::command(async)]
pub fn npx_update(
    app: AppHandle,
    state: State<'_, AppState>,
    global: bool,
    skill: Option<String>,
    project_root: Option<String>,
) -> Result<NpxRunResult, String> {
    let _t = CmdTimer::new("npx_update");
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    let invocation = SkillsInvocation {
        args: build_update_args(skill.as_deref(), global),
        project_root: project_root_opt(project_root),
        timeout_secs: 600,
    };
    let run = npx::run_skills(&config, &invocation, |line| {
        let _ = app.emit("npx-output", line.to_string());
    })
    .map_err(|e| e.to_string())?;
    Ok(run)
}

/// `skills ls --json` passthrough (the CLI's own view of installed skills).
#[tauri::command]
pub fn npx_list(
    state: State<'_, AppState>,
    global: bool,
    project_root: Option<String>,
) -> Result<Vec<ListedSkill>, String> {
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    npx::list_skills(&config, global, project_root_opt(project_root)).map_err(|e| e.to_string())
}

/// Preview the exact CLI command an action would run — shown before the user
/// confirms (Mole-style transparency), no side effects.
#[tauri::command]
pub fn npx_preview_command(
    state: State<'_, AppState>,
    req: InstallRequest,
) -> Result<String, String> {
    let config = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        npx_config(&s)
    };
    let agents = resolve_agents(&req.agents);
    let invocation = SkillsInvocation {
        args: build_add_args(&req.source, &req.skills, &agents, req.global, req.copy),
        project_root: project_root_opt(req.project_root),
        timeout_secs: 600,
    };
    Ok(equivalent_command(&config, &invocation))
}

// ---------------------------------------------------------------------------
// Search (skills.sh API, mirror-aware)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillsSearchHit {
    pub id: String,
    pub name: String,
    pub installs: i64,
    pub source: String,
}

/// Search the skills hub registry (skills.sh by default, `SKILLS_API_URL`
/// mirror when configured). The CLI's own `find` is interactive fzf, so the
/// app talks to the same HTTP API directly. P3 review fix (B5): the request
/// honors the `proxy_env` setting — the CLI's git steps get the proxy via env
/// injection, so the search path must not be the odd one out.
#[tauri::command(async)]
pub fn skills_search(state: State<'_, AppState>, query: String, limit: Option<u32>) -> Result<Vec<SkillsSearchHit>, String> {
    let _t = CmdTimer::new("skills_search");
    let (base, proxy) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        let url = s.skills_api_url.trim().to_string();
        let base = if url.is_empty() { "https://skills.sh".to_string() } else { url };
        (base, s.proxy_env.trim().to_string())
    };
    let limit = limit.unwrap_or(20).clamp(1, 50);
    let url = format!("{}/api/search?q={}&limit={}", base.trim_end_matches('/'), urlencoding_encode(&query), limit);
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15));
    if !proxy.is_empty() {
        builder = builder
            .proxy(reqwest::Proxy::all(&proxy).map_err(|e| format!("代理地址无效（{proxy}）：{e}"))?);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let resp = client.get(&url).header("User-Agent", "SkillMint").send().map_err(|e| format!("搜索失败（{url}）：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("搜索服务返回 {}（{url}）", resp.status()));
    }
    #[derive(serde::Deserialize)]
    struct Raw {
        skills: Vec<RawHit>,
    }
    #[derive(serde::Deserialize)]
    struct RawHit {
        id: Option<String>,
        name: Option<String>,
        installs: Option<i64>,
        source: Option<String>,
    }
    let text = resp.text().map_err(|e| format!("读取搜索结果失败：{e}"))?;
    let raw: Raw = serde_json::from_str(&text).map_err(|e| format!("解析搜索结果失败：{e}"))?;
    Ok(raw
        .skills
        .into_iter()
        .map(|h| SkillsSearchHit {
            id: h.id.unwrap_or_default(),
            name: h.name.unwrap_or_default(),
            installs: h.installs.unwrap_or(0),
            source: h.source.unwrap_or_default(),
        })
        .collect())
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Rebuild the index from on-disk truth (locks + agent dirs + hubs).
#[tauri::command(async)]
pub fn rebuild_skill_index(
    state: State<'_, AppState>,
    project_root: Option<String>,
) -> Result<IndexSummary, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let root = project_root_opt(project_root);
    index::rebuild(&db, root.as_deref()).map_err(|e| e.to_string())
}

/// List the cached index rows.
#[tauri::command]
pub fn get_skill_index(
    state: State<'_, AppState>,
    project_root: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<Vec<crate::db::index_store::SkillIndexRow>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_skill_index(&project_key(&project_root_opt(project_root)), scopes.as_deref())
        .map_err(|e| e.to_string())
}

/// Cheap mtime probe: has anything the index cares about changed since the
/// last rebuild? The auto tick calls this; the frontend calls it on focus.
#[tauri::command]
pub fn skill_index_stale(
    state: State<'_, AppState>,
    project_root: Option<String>,
) -> Result<bool, String> {
    let root = project_root_opt(project_root);
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let current = index::fingerprint(root.as_deref());
    let key = project_key(&root);
    Ok(match db.get_index_meta(&key).map_err(|e| e.to_string())? {
        Some((fp, _)) => fp != current,
        None => true,
    })
}

/// Read one indexed skill's SKILL.md for preview.
#[tauri::command]
pub fn read_skill_index_content(skill_md_path: String) -> Result<String, String> {
    std::fs::read_to_string(&skill_md_path).map_err(|e| format!("无法读取 {}：{e}", skill_md_path))
}

// ---------------------------------------------------------------------------
// Private hub
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn hub_create_skill(
    state: State<'_, AppState>,
    scope: String,
    project_root: Option<String>,
    name: String,
    description: String,
) -> Result<String, String> {
    let root = project_root_opt(project_root);
    let hub = hub::resolve_hub(&scope, root.as_deref()).map_err(|e| e.to_string())?;
    let dir = hub::create_skill(&hub, &scope, &name, &description).map_err(|e| e.to_string())?;
    let _ = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        index::rebuild(&db, root.as_deref())
    };
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn hub_collect_skill(
    state: State<'_, AppState>,
    scope: String,
    project_root: Option<String>,
    source: String,
    name: Option<String>,
) -> Result<String, String> {
    let root = project_root_opt(project_root);
    let hub = hub::resolve_hub(&scope, root.as_deref()).map_err(|e| e.to_string())?;
    let dir = hub::collect_skill(&hub, &scope, std::path::Path::new(&source), name.as_deref())
        .map_err(|e| e.to_string())?;
    let _ = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        index::rebuild(&db, root.as_deref())
    };
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn hub_status_cmd(scope: String, project_root: Option<String>) -> Result<HubStatus, String> {
    let hub = hub::resolve_hub(&scope, project_root_opt(project_root).as_deref()).map_err(|e| e.to_string())?;
    hub::hub_status(&hub, &scope).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hub_push(scope: String, project_root: Option<String>) -> Result<String, String> {
    let hub = hub::resolve_hub(&scope, project_root_opt(project_root).as_deref()).map_err(|e| e.to_string())?;
    hub::push(&hub).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hub_set_remote(
    scope: String,
    project_root: Option<String>,
    url: String,
) -> Result<(), String> {
    let hub = hub::resolve_hub(&scope, project_root_opt(project_root).as_deref()).map_err(|e| e.to_string())?;
    hub::set_remote(&hub, url.trim()).map_err(|e| e.to_string())
}

/// The hub path as an install source — `npx skills add <this>` is the whole
/// private-hub distribution story.
#[tauri::command]
pub fn hub_install_source(scope: String, project_root: Option<String>) -> Result<String, String> {
    let hub = hub::resolve_hub(&scope, project_root_opt(project_root).as_deref()).map_err(|e| e.to_string())?;
    if !hub.is_dir() {
        return Err("hub 尚未创建（新建一个 skill 时会自动创建）".into());
    }
    Ok(hub.to_string_lossy().to_string())
}

/// List hub skills (from the live directory, not the cached index).
#[tauri::command]
pub fn hub_list_skills(scope: String, project_root: Option<String>) -> Result<Vec<SkillIndexEntry>, String> {
    let hub = hub::resolve_hub(&scope, project_root_opt(project_root).as_deref()).map_err(|e| e.to_string())?;
    let scope_name = if scope == "global" { index::SCOPE_HUB_GLOBAL } else { index::SCOPE_HUB_PROJECT };
    Ok(index::scan_hub_dir(&hub, scope_name))
}
