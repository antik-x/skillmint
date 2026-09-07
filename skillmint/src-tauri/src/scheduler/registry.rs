use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::models::TaskKind;

/// The outcome of executing a scheduled task: a human-readable summary.
pub type TaskOutcome = Result<String, String>;

/// A scheduled-task executor receives the global AppState and returns a summary
/// or an error message. It must be Send + Sync because it lives in the registry
/// and is invoked from async tasks. Wrapped in Arc so it can be cloned into
/// the blocking execution thread.
pub type TaskExecutor = Arc<dyn Fn(&crate::AppState) -> TaskOutcome + Send + Sync>;

/// Build the registry mapping each supported task kind to its executor.
pub fn build_registry() -> HashMap<TaskKind, TaskExecutor> {
    let mut registry: HashMap<TaskKind, TaskExecutor> = HashMap::new();

    registry.insert(
        TaskKind::CollectUsageData,
        Arc::new(|state| {
            let device_id = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                settings.device_id.clone()
            };
            let all_stats =
                crate::commands::collect_usage_data_inner_with_progress(&state.db, &device_id, |_source, _stats| {}, || false)
                    .map_err(|e| e.to_string())?;
            let total_sessions: i64 = all_stats.iter().map(|s| s.sessions).sum();
            let total_prompts: i64 = all_stats.iter().map(|s| s.prompts).sum();
            Ok(format!(
                "采集 {} 个源，{} 会话 / {} prompt",
                all_stats.len(),
                total_sessions,
                total_prompts
            ))
        }),
    );

    registry.insert(
        TaskKind::ScanAgents,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let agents = crate::scan::scan_and_persist_agents(&db).map_err(|e| e.to_string())?;
            Ok(format!("发现 {} 个 Agent", agents.len()))
        }),
    );

    registry.insert(
        TaskKind::SyncAllSkills,
        Arc::new(|state| {
            let result = crate::commands::run_sync_core(state)?;
            Ok(format!(
                "同步 {} 个目标，导入 {} 个 Skill，{} 个冲突",
                result.targets.len(),
                result.imported_skills,
                result.import_conflicts
            ))
        }),
    );

    registry.insert(
        TaskKind::ImportAgentSkills,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let settings = state.settings.lock().map_err(|e| e.to_string())?;
            let (imported, conflicts) =
                crate::commands::import_all_agent_skills(&db, &settings).map_err(|e| e.to_string())?;
            if imported == 0 && conflicts == 0 {
                Ok("Center Repo 已存在 Skill，无需导入".to_string())
            } else {
                Ok(format!("导入 {} 个 Skill，{} 个冲突", imported, conflicts))
            }
        }),
    );

    registry.insert(
        TaskKind::ScanAgentDirectories,
        Arc::new(|state| {
            let mut db = state.db.lock().map_err(|e| e.to_string())?;
            let settings = state.settings.lock().map_err(|e| e.to_string())?;
            let agents = db.get_agents().map_err(|e| e.to_string())?;
            let mut scanned = 0usize;
            let mut total_items = 0usize;
            for agent in agents {
                let mut dirs: Vec<PathBuf> = vec![agent.skill_directory.clone()];
                let secondary: Vec<PathBuf> = db
                    .get_agent_directories(&agent.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|d| d.is_enabled)
                    .map(|d| d.path)
                    .collect();
                dirs.extend(secondary);

                for dir in dirs {
                    let items = crate::commands::scan_directory_skills_inner(
                        &mut db,
                        &settings,
                        &agent.id,
                        &dir.to_string_lossy(),
                        false,
                    )
                    .map_err(|e| e.to_string())?;
                    scanned += 1;
                    total_items += items.len();
                }
            }
            Ok(format!("扫描 {} 个目录，共 {} 个 Skill", scanned, total_items))
        }),
    );

    registry.insert(
        TaskKind::ScanProjects,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let device_id = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                settings.device_id.clone()
            };
            let count = db
                .link_sessions_to_projects(&device_id)
                .map(|n| n as usize)
                .map_err(|e| e.to_string())?;
            Ok(format!("扫描到 {} 个项目", count))
        }),
    );

    registry.insert(
        TaskKind::GenerateKnowledgeGraph,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let (device_id, center_repo) = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                (settings.device_id.clone(), settings.center_repo.clone())
            };
            let cfg = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                settings.ai.clone()
            };
            let (nodes, edges) =
                crate::kg::analyze_all_skills(&db, &device_id, &center_repo).map_err(|e| e.to_string())?;
            let classify = crate::classifier::classify_prompts(&db, None, &cfg).map_err(|e| e.to_string())?;
            Ok(format!(
                "生成 {} 个节点 / {} 条边，分类 {} / {} 条 prompt",
                nodes, edges, classify.classified, classify.eligible
            ))
        }),
    );

    registry.insert(
        TaskKind::GenerateDailySummary,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let cfg = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                settings.ai.clone()
            };
            let yesterday = {
                let d = chrono::Local::now() - chrono::Duration::days(1);
                d.format("%Y-%m-%d").to_string()
            };
            let outcome =
                crate::analyzer::generate_daily_summary(&db, &yesterday, &cfg).map_err(|e| e.to_string())?;
            match outcome {
                crate::analyzer::Outcome::Generated { summary } => {
                    Ok(format!("已生成 {} 摘要", summary.date))
                }
                crate::analyzer::Outcome::NoKey => Ok("未配置 AI，跳过摘要生成".to_string()),
                crate::analyzer::Outcome::NoSessions => Ok("昨日无会话，无需生成摘要".to_string()),
                crate::analyzer::Outcome::Failed { reason } => Err(reason),
            }
        }),
    );

    registry.insert(
        TaskKind::SyncRemoteSources,
        Arc::new(|state| {
            let settings = state.settings.lock().map_err(|e| e.to_string())?;
            if !settings.remote_enabled {
                return Err("远程功能已关闭，请在偏好设置中开启".to_string());
            }
            let center_repo = settings.center_repo.clone();
            drop(settings);

            let sources = {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                db.get_sources().map_err(|e| e.to_string())?
            };

            let mut refreshed = 0usize;
            let mut failed = 0usize;
            for source in sources {
                if crate::commands::refresh_source_inner(&state, &source.id).is_ok() {
                    refreshed += 1;
                } else {
                    failed += 1;
                }
            }

            if let Err(e) = crate::commands::git_pull_inner(&center_repo, None, None) {
                eprintln!("[scheduler] git pull failed: {}", e);
            }

            Ok(format!("刷新 {} 个远程源，{} 个失败", refreshed, failed))
        }),
    );

    registry.insert(
        TaskKind::BackupCenterRepo,
        Arc::new(|state| {
            let center_repo = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                settings.center_repo.clone()
            };
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let backup_dir = state.app_dir.join("backups").join(&today);
            std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
            let path = backup_dir.join(format!("skillmint-backup-{}.zip", today));
            crate::commands::backup_center_repo_inner(&center_repo, &path)
                .map_err(|e| e.to_string())?;
            Ok(format!("已备份到 {}", path.display()))
        }),
    );

    registry.insert(
        TaskKind::RunDiscoveryPipeline,
        Arc::new(|state| {
            let mut db = state.db.lock().map_err(|e| e.to_string())?;
            let (device_id, cfg) = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                (settings.device_id.clone(), settings.ai.clone())
            };
            let result = crate::discovery::run_pipeline(&mut db, &device_id, &cfg)
                .map_err(|e| e.to_string())?;
            Ok(format!("发现管线完成：新增 {} 条，过期 {} 条", result.inserted, result.expired))
        }),
    );

    registry.insert(
        TaskKind::GenerateWeeklyReport,
        Arc::new(|state| {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let (device_id, cfg) = {
                let settings = state.settings.lock().map_err(|e| e.to_string())?;
                (settings.device_id.clone(), settings.ai.clone())
            };
            let week = crate::discovery::this_monday_utc();
            let report = crate::discovery::generate_weekly_report(&db, &device_id, &cfg, &week)
                .map_err(|e| e.to_string())?;
            Ok(format!("已生成 {} 周报", report.week_start))
        }),
    );

    registry
}
