use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::watch;

mod acp;
mod agents_table;
pub mod commands;
mod classifier;
mod collector;
pub(crate) mod collectors_ext;
pub mod db;
mod discovery;
mod fs;
mod hub;
mod index;
mod kg;
mod llm;
mod metrics;
mod models;
mod npx;
mod openviking;
mod origin;
mod pricing;
mod prompt_kind;
mod remote;
mod scan;
mod scheduler;
pub mod settings;
mod skills_lock;
mod sync;
mod window;
mod analyzer;

#[cfg(test)]
mod tests;

pub struct AppState {
    pub db: Mutex<db::Db>,
    pub settings: Mutex<settings::Settings>,
    pub tray: Mutex<Option<tauri::tray::TrayIcon>>,
    /// P1-1: buffers deep-link routing events until the frontend signals
    /// "app-ready" (its listeners are registered), then replays them in order.
    pub deep_links: DeepLinkBuffer,
    /// Stop handle for the auto-sync background loop.
    /// `Some(tx)` = loop running; sending `true` (or dropping the sender) stops it.
    /// `None` = no loop running (interval == 0 or not yet started).
    pub sync_stop: Mutex<Option<watch::Sender<bool>>>,
    /// PRD-10: handle to the scheduled-task scheduler so it can be stopped/restarted.
    pub scheduler: Mutex<Option<scheduler::engine::Scheduler>>,
    /// PRD-10: in-memory abort handles for currently-running scheduled tasks.
    pub scheduler_running: tokio::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// P0: per-collection-job cancellation flags.
    pub collection_cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// P0: prevents concurrent manual sync_all_command runs.
    pub sync_all_running: AtomicBool,
    /// P-今天: in-memory throttle for `ensure_daily_summary` (date → last
    /// attempt, epoch secs). Bounds how often entering the Today page can
    /// trigger an LLM digest refresh.
    pub daily_summary_attempts: Mutex<HashMap<String, u64>>,
    /// The application's data directory (e.g. `~/Library/Application Support/com.skillmint.app`).
    /// Used for snapshots and other app-private storage.
    pub app_dir: PathBuf,
}

/// P1-1: deep links can arrive before the WebView has registered its event
/// listeners (cold start, or window not yet created) — emitting then means the
/// event is silently lost. Events are buffered until the frontend emits
/// "app-ready", then replayed in arrival order; later events go straight
/// through.
pub struct DeepLinkBuffer {
    ready: AtomicBool,
    pending: Mutex<Vec<(String, Option<String>)>>,
}

impl DeepLinkBuffer {
    pub fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Enqueue one routing event (`name` + optional string payload). Returns
    /// the events to emit right now: the event itself once the frontend is
    /// ready, nothing while it is still buffered.
    pub fn push(&self, name: &str, payload: Option<String>) -> Vec<(String, Option<String>)> {
        if self.ready.load(std::sync::atomic::Ordering::SeqCst) {
            return vec![(name.to_string(), payload)];
        }
        if let Ok(mut guard) = self.pending.lock() {
            guard.push((name.to_string(), payload));
        }
        Vec::new()
    }

    /// Mark the frontend ready and drain the backlog in arrival order.
    pub fn mark_ready(&self) -> Vec<(String, Option<String>)> {
        self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        match self.pending.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => Vec::new(),
        }
    }
}

/// Emit one deep-link routing event to the frontend. `deep-link-sync` carries
/// no payload; the rest carry a string payload.
fn emit_deep_link_event(app: &AppHandle, name: &str, payload: &Option<String>) {
    match payload {
        Some(p) => {
            let _ = app.emit::<String>(name, p.clone());
        }
        None => {
            let _ = app.emit::<()>(name, ());
        }
    }
}

/// P1-2: tray shortcut target — wake the window and route the frontend to a
/// page. Routed through the same reliability buffer as deep links so a click
/// landing before the frontend is ready is replayed rather than lost.
fn tray_navigate(app: &AppHandle, target: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(state) = app.try_state::<AppState>() {
        for (name, payload) in state
            .deep_links
            .push("tray-navigate", Some(target.to_string()))
        {
            emit_deep_link_event(app, &name, &payload);
        }
    }
}

/// (Re)start the auto-sync scheduler using the current `auto_sync_interval_minutes`.
/// Stops any running loop first, then starts a new one if the interval is > 0.
/// Called on app setup and whenever settings are saved (so a cadence change — or
/// setting it to 0 to disable — takes effect immediately). PRD-0 §4.7.
pub fn restart_sync_scheduler(app: &AppHandle) {
    stop_sync_scheduler(app);
    start_sync_scheduler(app);
}

/// Stop the running auto-sync loop, if any. Idempotent.
fn stop_sync_scheduler(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.sync_stop.lock() {
            if let Some(tx) = guard.take() {
                // Signal stop; ignore error (receiver already dropped is fine).
                let _ = tx.send(true);
            }
        }
    }
}

/// Start the auto-sync background loop if `auto_sync_interval_minutes > 0`.
/// Does nothing (no-op) when the interval is 0 (disabled) or no AppState exists.
fn start_sync_scheduler(app: &AppHandle) {
    let interval_minutes = match app.try_state::<AppState>() {
        Some(state) => match state.settings.lock() {
            Ok(s) => s.auto_sync_interval_minutes,
            Err(_) => return,
        },
        None => return,
    };
    if interval_minutes == 0 {
        return; // disabled
    }

    let (tx, mut rx) = watch::channel(false);
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.sync_stop.lock() {
            *guard = Some(tx);
        }
    }

    let app_handle = app.clone();
    let period = Duration::from_secs(u64::from(interval_minutes) * 60);
    tauri::async_runtime::spawn(async move {
        // `tokio::select!` races the sleep against a stop signal so we can shut
        // down promptly even mid-wait.
        loop {
            tokio::select! {
                _ = tokio::time::sleep(period) => {}
                _ = rx.changed() => break, // stop requested
            }

            // Run one refresh tick (P3-3): cheap mtime fingerprint probe of the
            // npx locks / canonical stores / hubs; rebuild the index only when
            // something changed. A user-triggered operation holding the locks
            // is skipped rather than blocked.
            let state = match app_handle.try_state::<AppState>() {
                Some(s) => s,
                None => break, // app shutting down
            };
            let (db, settings) = match (state.db.try_lock(), state.settings.try_lock()) {
                (Ok(db), Ok(s)) => (db, s),
                _ => continue, // busy; wait for the next interval
            };
            match crate::index::rebuild_if_stale(&db, None) {
                Ok(Some(summary)) => {
                    let _ = update_tray_from_result(&app_handle, summary.modified);
                }
                Ok(None) => {}
                Err(e) => eprintln!("[auto-refresh] index rebuild failed: {e}"),
            }
            drop((db, settings));
        }
    });
}

/// Update the tray icon/tooltip from outside `commands` (avoids a circular
/// `State` borrow in the async scheduler). Mirrors `commands::update_tray_status`.
fn update_tray_from_result(app: &AppHandle, conflict_count: usize) -> Result<(), String> {
    let state = app.state::<AppState>();
    let tray_guard = state.tray.lock().map_err(|e| e.to_string())?;
    if let Some(tray) = tray_guard.as_ref() {
        let icon_name = if conflict_count > 0 {
            "tray-warning.png"
        } else {
            "tray-normal.png"
        };
        let icon_path = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default()
            .join("../Resources")
            .join(icon_name);
        if let Ok(image) = tauri::image::Image::from_path(&icon_path) {
            let _ = tray.set_icon(Some(image));
            let tooltip = if conflict_count > 0 {
                format!("SkillMint · {} 个冲突", conflict_count)
            } else {
                "SkillMint · 同步正常".to_string()
            };
            let _ = tray.set_tooltip(Some(&tooltip));
        }
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let app_dir = match app_handle.path().app_data_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[app] failed to get app data dir: {e}");
                    return Err(format!("无法获取应用数据目录：{e}").into());
                }
            };
            std::fs::create_dir_all(&app_dir)?;

            let settings = settings::Settings::load_or_default(&app_dir)?;
            let mut db = db::Db::new(&app_dir.join("skillmint.db"))?;
            db.init(&settings.device_id)?;

            // P0: reset any collection jobs left running from a previous crash.
            if let Err(e) = db.reset_stale_collection_jobs() {
                eprintln!("[app] failed to reset stale collection jobs: {e}");
            }

            // P3-6: the center repo is retired — no bootstrap, no recovery
            // banner. The skills inventory comes from the npx locks, the agent
            // directories, and the private hubs (rebuildable index).

            app.manage(AppState {
                db: Mutex::new(db),
                settings: Mutex::new(settings),
                tray: Mutex::new(None),
                deep_links: DeepLinkBuffer::new(),
                sync_stop: Mutex::new(None),
                scheduler: Mutex::new(None),
                scheduler_running: tokio::sync::Mutex::new(HashMap::new()),
                collection_cancel_flags: Mutex::new(HashMap::new()),
                sync_all_running: AtomicBool::new(false),
                daily_summary_attempts: Mutex::new(HashMap::new()),
                app_dir: app_dir.clone(),
            });

            // Build tray icon and menu
            let show_i = tauri::menu::MenuItem::with_id(
                &app_handle,
                "show",
                "打开 SkillMint",
                true,
                None::<&str>,
            )?;
            // P1-2: direct entries so the tray stays a usable navigation
            // entry point even when the window is hidden.
            let skills_i = tauri::menu::MenuItem::with_id(
                &app_handle,
                "open-skill-library",
                "Skill 库",
                true,
                None::<&str>,
            )?;
            let sync_i = tauri::menu::MenuItem::with_id(
                &app_handle,
                "open-sync-health",
                "同步健康",
                true,
                None::<&str>,
            )?;
            let quit_i =
                tauri::menu::MenuItem::with_id(&app_handle, "quit", "退出", true, None::<&str>)?;
            let menu =
                tauri::menu::Menu::with_items(&app_handle, &[&show_i, &skills_i, &sync_i, &quit_i])?;

            // F5: fall back to a bundled icon if the default window icon cannot be loaded.
            let tray_icon = match app_handle.default_window_icon() {
                Some(icon) => icon.clone(),
                None => {
                    eprintln!("[app] default window icon not found, falling back to bundled icon");
                    let fallback = std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_default()
                        .join("../Resources")
                        .join("tray-normal.png");
                    tauri::image::Image::from_path(&fallback).unwrap_or_else(|_| {
                        // 1x1 transparent PNG as last-resort empty icon.
                        const EMPTY_PNG: &[u8] = &[
                            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00,
                            0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
                            0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89,
                            0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63,
                            0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x73, 0x75, 0x01, 0x18,
                            0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
                            0x82,
                        ];
                        tauri::image::Image::from_bytes(EMPTY_PNG).unwrap_or_else(|e| {
                            panic!("failed to create empty tray icon: {e}")
                        })
                    })
                }
            };

            let tray = tauri::tray::TrayIconBuilder::new()
                .icon(tray_icon)
                .tooltip("SkillMint")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "open-skill-library" => tray_navigate(app, "skillLibrary"),
                    "open-sync-health" => tray_navigate(app, "today"),
                    _ => {}
                })
                .build(&app_handle)?;

            if let Some(state) = app.try_state::<AppState>() {
                *state.tray.lock().map_err(|e| e.to_string())? = Some(tray);
            }

            // P3-3: rebuild the skill index once at startup (background; the
            // fingerprint check makes it a no-op when nothing changed).
            // P-今天: the same thread also runs the yesterday-digest catch-up
            // (补偿「app 错过了夜间 08:00 调度窗口」——调度器只管 app 开着的时候).
            {
                let db_path = app_dir.join("skillmint.db");
                let device_id = {
                    let state = app_handle.state::<AppState>();
                    let guard = state.settings.lock();
                    match guard {
                        Ok(s) => s.device_id.clone(),
                        Err(_) => String::new(),
                    }
                };
                let ai_cfg = {
                    let state = app_handle.state::<AppState>();
                    let guard = state.settings.lock();
                    guard.ok().map(|s| s.ai.clone())
                };
                if !device_id.is_empty() {
                    std::thread::spawn(move || {
                        match crate::db::Db::new(&db_path) {
                            Ok(mut db) => {
                                if let Err(e) = db.init(&device_id) {
                                    eprintln!("[startup-index] failed to init db: {e}");
                                    return;
                                }
                                match crate::index::rebuild_if_stale(&db, None) {
                                    Ok(Some(summary)) => {
                                        eprintln!("[startup-index] rebuilt: {} entries", summary.total);
                                    }
                                    Ok(None) => {}
                                    Err(e) => eprintln!("[startup-index] rebuild failed: {e}"),
                                }
                                if let Some(cfg) = ai_cfg {
                                    let db = std::sync::Mutex::new(db);
                                    if let Some(msg) =
                                        commands::catch_up_yesterday_summary(&db, &cfg)
                                    {
                                        eprintln!("[startup-digest] {msg}");
                                    }
                                }
                            }
                            Err(e) => eprintln!("[startup-index] failed to open db: {e}"),
                        }
                    });
                }
            }

            // SPEC-F3: listen for skillmint:// deep links and forward to frontend.
            #[cfg(desktop)]
            {
                let app_handle_dpl = app_handle.clone();
                app_handle.listen("deep-link://new-url", move |event| {
                    // P1-1: wake the window FIRST — a deep link is an explicit
                    // user intent to interact with the app, and emitting into a
                    // hidden window was the ~50% failure mode.
                    if let Some(window) = app_handle_dpl.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    if let Ok(urls) = serde_json::from_str::<Vec<String>>(event.payload()) {
                        let state = app_handle_dpl.state::<AppState>();
                        for url_str in urls {
                            eprintln!("[deep-link] received {url_str}");
                            let mut events = vec![("deep-link".to_string(), Some(url_str.clone()))];
                            if let Some(path) = url_str.strip_prefix("skillmint://") {
                                if path == "sync" {
                                    events.push(("deep-link-sync".to_string(), None));
                                } else if let Some(skill_name) = path.strip_prefix("open/skill/") {
                                    events.push(("deep-link-open-skill".to_string(), Some(skill_name.to_string())));
                                }
                            }
                            // P1-1: buffer until the frontend signals app-ready.
                            for (name, payload) in events {
                                for (n, p) in state.deep_links.push(&name, payload) {
                                    emit_deep_link_event(&app_handle_dpl, &n, &p);
                                }
                            }
                        }
                    }
                });

                // P1-1: the frontend emits "app-ready" once its deep-link
                // listeners are registered; replay anything buffered so far.
                let app_handle_ready = app_handle.clone();
                app_handle.listen("app-ready", move |_| {
                    let state = app_handle_ready.state::<AppState>();
                    for (name, payload) in state.deep_links.mark_ready() {
                        emit_deep_link_event(&app_handle_ready, &name, &payload);
                    }
                });
            }

            // Start the auto-sync background loop if an interval is configured (PRD-0 §4.7).
            start_sync_scheduler(&app_handle);

            // PRD-10: start the scheduled-task engine.
            scheduler::start_scheduler(&app_handle);

            // SPEC-C3 T2: purge expired recycle-bin snapshots at startup. Runs
            // on a background thread with its own DB connection so it never
            // blocks window display or contends the AppState mutex.
            {
                let db_path = app_dir.join("skillmint.db");
                let device_id = {
                    let state = app_handle.state::<AppState>();
                    let guard = state.settings.lock();
                    match guard {
                        Ok(s) => s.device_id.clone(),
                        Err(_) => String::new(),
                    }
                };
                std::thread::spawn(move || {
                    match crate::db::Db::new(&db_path) {
                        Ok(mut db) => {
                            if let Err(e) = db.init(&device_id) {
                                eprintln!("[trash] failed to init db: {e}");
                                return;
                            }
                            match db.expired_trash_items() {
                                Ok(items) => {
                                    for it in items {
                                        let _ = std::fs::remove_dir_all(&it.snapshot_path);
                                        let _ = db.delete_trash_item(it.id);
                                    }
                                    eprintln!("[trash] startup cleanup done");
                                }
                                Err(e) => eprintln!("[trash] startup cleanup failed: {e}"),
                            }
                        }
                        Err(e) => eprintln!("[trash] failed to open db: {e}"),
                    }
                });
            }

            // P1-2: hide instead of destroy on close. Closing the window used
            // to destroy the WebView and lose all SPA state (e.g. a half-filled
            // import dialog); hiding keeps the scene intact for tray/deep-link
            // re-entry. Quit semantics are preserved: app.exit() (tray 退出)
            // and Cmd+Q go through RunEvent::ExitRequested, not CloseRequested.
            if let Some(window) = app.get_webview_window("main") {
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            // Show main window after setup
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::init_app,
            commands::get_skills,
            commands::get_agents,
            commands::save_agent,
            commands::scan_agents,
            commands::scan_agent_skills,
            commands::get_agent_skill_counts,
            commands::clear_collected_data,
            commands::reset_database,
            // PRD-10: scheduled tasks
            commands::get_scheduled_tasks,
            commands::save_scheduled_task,
            commands::delete_scheduled_task,
            commands::run_scheduled_task_now,
            commands::get_task_runs,
            commands::pause_scheduled_task,
            commands::resume_scheduled_task,
            // SPEC-C3: recycle bin
            commands::list_trash_items,
            commands::restore_trash_item,
            commands::purge_trash_item,
            commands::open_path_in_terminal,
            commands::open_skill_in_editor,
            commands::get_settings,
            commands::save_settings,
            commands::openviking_probe,
            commands::get_device_id,
            commands::collect_usage_data,
            commands::get_collection_status,
            commands::start_collection_job,
            commands::get_collection_job,
            commands::cancel_collection_job,
            commands::list_recent_collection_jobs,
            commands::test_acp_transport,
            commands::detect_local_agents,
            commands::get_acp_health,
            commands::list_llm_request_logs,
            commands::test_ai_model,
            commands::get_agent_usage,
            commands::get_agent_detail,
            // PRD-06 §3.3: agent directory (1:N) management.
            commands::add_agent_directory,
            commands::remove_agent_directory,
            commands::update_agent_directory,
            commands::scan_directory_skills,
            commands::get_projects_usage,
            commands::get_skill_usage,
            // PRD-01: project-level skills
            commands::get_projects,
            commands::scan_projects,
            commands::classify_projects,
            commands::open_project_in_finder,
            commands::open_project_in_terminal,
            commands::remove_project,
            // PRD-01 patch: project detail page + multi-version skills
            commands::get_project_detail,
            // PRD-02: prompt-driven generation + reports
            commands::get_high_value_prompts,
            commands::ignore_prompt_group,
            commands::unignore_prompt_group,
            commands::list_ignored_prompt_groups,
            commands::generate_skill_from_prompt,
            commands::preview_skill_from_prompt,
            commands::repair_skill_paths,
            commands::get_skill_suggestion_report,
            commands::export_report,
            // SPEC-I2: discovery inbox + weekly reports
            commands::run_discovery_pipeline,
            commands::list_discoveries,
            commands::decide_discovery,
            // SPEC-C1: gate-rejection ledger (low-confidence band etc.)
            commands::list_gate_rejections,
            // SPEC-C2: retry sync for a single skill (partial_synced recovery)
            commands::get_growth_metrics,
            commands::generate_weekly_report,
            commands::get_weekly_report,
            commands::get_usage_timeseries,
            commands::get_skill_health,
            // PRD-08: full analysis integration (window metrics + AI-Digest engine)
            commands::get_window_metrics,
            commands::classify_prompts,
            commands::generate_daily_summary,
            commands::get_daily_summary,
            // PRD-11: AI daily-summary value module
            commands::list_daily_summaries,
            commands::get_summary_value_metrics,
            // PRD-08 §3.8 (P2): AI-Digest digest.db one-time import
            commands::detect_digest_db,
            commands::import_digest_db,
            // PRD-08 §3.4 (P3): role-profile heatmap cell drill-down
            commands::get_prompts_for_cell,
            // PRD-03: knowledge graph
            commands::analyze_knowledge_graph,
            commands::get_knowledge_graph,
            commands::confirm_kg_edge,
            commands::reject_kg_edge,
            commands::recommend_skills_for_task,
            commands::get_related_skills,
            commands::export_knowledge_graph,
            commands::import_knowledge_graph,
            // PRD-0 §4.8: Center Repo zip backup / restore
            commands::backup_center_repo,
            commands::restore_center_repo,
            // PRD-07: remote sources, unified discovery, safety scan
            commands::add_source,
            commands::get_sources,
            commands::remove_source,
            commands::refresh_source,
            commands::list_source_skills,
            commands::scan_skill_safety,
            commands::search_all,
            commands::preview_remote_skill,
            // PRD-02 Phase 2: skill bundles
            commands::list_bundles,
            commands::get_bundle_detail,
            commands::create_bundle,
            commands::update_bundle,
            commands::delete_bundle,
            commands::add_skill_to_bundle,
            commands::remove_skill_from_bundle,
            commands::apply_bundle_to_project,
            commands::export_bundle,
            commands::export_bundle_directory,
            commands::import_bundle,
            // PRD-09 §4: snapshots + Git version control
            commands::create_snapshot,
            commands::list_snapshots,
            commands::restore_snapshot,
            commands::git_status_inited,
            commands::git_init_repo,
            commands::git_commit,
            commands::git_push,
            commands::git_pull,
            commands::git_versions,
            // P2-2: data dictionary + raw data export
            commands::export_data_dictionary,
            commands::export_raw_data,
            // P3: npx skills bridge — install/remove/update via the real CLI,
            // rebuildable index, private hubs, registry search.
            commands::npx_env,
            commands::get_agents_table,
            commands::npx_install,
            commands::npx_remove,
            commands::npx_update,
            commands::npx_list,
            commands::npx_preview_command,
            commands::skills_search,
            commands::rebuild_skill_index,
            commands::get_skill_index,
            commands::skill_index_stale,
            commands::read_skill_index_content,
            commands::clean_broken_skill_links,
            // P-今天: dual story cards (today live + yesterday) + active-day streak
            commands::ensure_daily_summary,
            commands::list_active_days,
            commands::hub_create_skill,
            commands::hub_collect_skill,
            commands::hub_status_cmd,
            commands::hub_push,
            commands::hub_set_remote,
            commands::hub_install_source,
            commands::hub_list_skills,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
