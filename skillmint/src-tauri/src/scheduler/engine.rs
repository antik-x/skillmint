use std::str::FromStr;
use std::time::Duration;

use chrono::TimeZone;
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

use crate::models::{RunStatus, ScheduledTask, ScheduleStrategy, TaskRun, TriggerSource};
use crate::scheduler::registry::build_registry;
use crate::AppState;

const TICK_INTERVAL: Duration = Duration::from_secs(10);
const RUNNING_TIMEOUT: Duration = Duration::from_secs(600);

pub struct Scheduler {
    stop_tx: watch::Sender<bool>,
}

impl Scheduler {
    fn new(stop_tx: watch::Sender<bool>) -> Self {
        Self { stop_tx }
    }
}

/// Start the scheduled-task engine. Idempotent: if a scheduler is already
/// running for this app, it is stopped and restarted.
pub fn start_scheduler(app: &AppHandle) -> Scheduler {
    stop_scheduler(app);

    let (stop_tx, mut stop_rx) = watch::channel(false);

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Ensure default tasks exist on first tick.
        let _ = ensure_defaults(&app_handle).await;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(TICK_INTERVAL) => {}
                _ = stop_rx.changed() => break,
            }

            if let Err(e) = tick(&app_handle).await {
                eprintln!("[scheduler] tick error: {}", e);
            }
        }

        // App shutting down: abort any in-flight scheduled tasks.
        let state = app_handle.state::<AppState>();
        let guard = state.scheduler_running.lock().await;
        for (_, handle) in guard.iter() {
            handle.abort();
        }
    });

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.scheduler.lock() {
            *guard = Some(Scheduler::new(stop_tx.clone()));
        }
    }

    Scheduler::new(stop_tx)
}

/// Stop any running scheduler loop for this app. Idempotent.
pub fn stop_scheduler(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.scheduler.lock() {
            if let Some(scheduler) = guard.take() {
                let _ = scheduler.stop_tx.send(true);
            }
        }
    }
}

/// Trigger a task immediately, regardless of its schedule. Returns the created
/// run record so the UI can track it.
pub async fn run_task_now(
    app: &AppHandle,
    task_id: &str,
    triggered_by: TriggerSource,
) -> Result<TaskRun, String> {
    let task = {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_scheduled_task(task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Task not found".to_string())?
    };

    let run = spawn_task_run(app, task, triggered_by).await?;
    Ok(run)
}

async fn ensure_defaults(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.ensure_default_tasks().map_err(|e| e.to_string())?;
    Ok(())
}

async fn tick(app: &AppHandle) -> Result<(), String> {
    let now = crate::commands::current_timestamp();

    // Recover any tasks that were marked running but whose process died.
    recover_orphaned_runs(app, now).await?;

    let tasks = {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_scheduled_tasks().map_err(|e| e.to_string())?
    };

    for task in tasks {
        if !task.enabled {
            continue;
        }
        if matches!(task.strategy, ScheduleStrategy::Manual) {
            continue;
        }

        // Check if already running in memory.
        {
            let state = app.state::<AppState>();
            let guard = state.scheduler_running.lock().await;
            if guard.contains_key(&task.id) {
                continue;
            }
        }

        // Due-ness must come from the persisted next_run_at (seeded on first
        // sight). Recomputing per tick yields the first occurrence strictly
        // after *now*, which can never satisfy a `<= now` check — that starved
        // every cron/interval task since launch (fixed 2026-09). A next_run_at
        // already in the past doubles as catch-up: a fire missed while the app
        // was closed runs once on the first tick after launch.
        let due = match task.next_run_at {
            Some(at) => at <= now,
            None => {
                let next_run = compute_next_run(&task, now);
                update_next_run(app, &task.id, next_run).await?;
                false
            }
        };
        if !due {
            continue;
        }

        // Also confirm no running DB row (defensive).
        let already_running = {
            let state = app.state::<AppState>();
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.get_running_task_run(&task.id).map_err(|e| e.to_string())?.is_some()
        };
        if already_running {
            continue;
        }

        spawn_task_run(app, task, TriggerSource::Schedule).await?;
    }

    Ok(())
}

async fn spawn_task_run(
    app: &AppHandle,
    task: ScheduledTask,
    triggered_by: TriggerSource,
) -> Result<TaskRun, String> {
    let run_id = crate::db::new_id();
    let started_at = crate::commands::current_timestamp();
    let run = TaskRun {
        id: run_id.clone(),
        task_id: task.id.clone(),
        status: RunStatus::Pending,
        started_at,
        finished_at: None,
        duration_ms: None,
        result_summary: String::new(),
        error_message: None,
        triggered_by,
    };

    {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.insert_task_run(&run).map_err(|e| e.to_string())?;
    }

    let app_handle = app.clone();
    let task_id = task.id.clone();
    let handle = tokio::spawn(async move {
        execute_task(&app_handle, task, run_id).await;
    });

    // Track the abort handle.
    {
        let state = app.state::<AppState>();
        let mut guard = state.scheduler_running.lock().await;
        guard.insert(task_id.clone(), handle.abort_handle());
    }

    Ok(run)
}

async fn execute_task(app: &AppHandle, task: ScheduledTask, run_id: String) {
    // Mark running.
    let now = crate::commands::current_timestamp();
    let _ = update_run_status(app, &run_id, RunStatus::Running, None, None, None).await;

    let registry = build_registry();
    let executor = registry.get(&task.task_kind).cloned();

    let (status, summary, error) = if let Some(exec) = executor {
        // Execute on the async runtime's blocking thread pool because some
        // tasks perform blocking I/O.
        let app_handle = app.clone();
        let result = tokio::task::spawn_blocking(move || {
            let state = app_handle.state::<AppState>();
            exec(&state)
        })
        .await;

        match result {
            Ok(Ok(msg)) => (RunStatus::Success, msg, None),
            Ok(Err(err)) => (RunStatus::Failed, String::new(), Some(err)),
            Err(join_err) => {
                let err = if join_err.is_panic() {
                    "任务执行 panic".to_string()
                } else {
                    "任务被取消".to_string()
                };
                (RunStatus::Failed, String::new(), Some(err))
            }
        }
    } else {
        (
            RunStatus::Failed,
            String::new(),
            Some(format!("未找到任务类型 {:?} 的执行器", task.task_kind)),
        )
    };

    let finished_at = crate::commands::current_timestamp();
    let duration_ms = finished_at.saturating_sub(now) * 1000;

    let _ = update_run_status(
        app,
        &run_id,
        status,
        Some(finished_at),
        Some(duration_ms),
        error.as_deref(),
    )
    .await;

    let _ = finalize_task_run(
        app,
        &task.id,
        finished_at,
        status,
        &summary,
        error.as_deref(),
    )
    .await;

    // Remove from in-memory running set.
    let _ = cleanup_running_handle(app, &task.id).await;
}

async fn update_run_status(
    app: &AppHandle,
    run_id: &str,
    status: RunStatus,
    finished_at: Option<u64>,
    duration_ms: Option<u64>,
    error_message: Option<&str>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let run = TaskRun {
        id: run_id.to_string(),
        task_id: String::new(), // not used in UPDATE WHERE id
        status,
        started_at: 0,
        finished_at,
        duration_ms,
        result_summary: String::new(),
        error_message: error_message.map(|s| s.to_string()),
        triggered_by: TriggerSource::Schedule,
    };
    db.update_task_run(&run).map_err(|e| e.to_string())
}

async fn finalize_task_run(
    app: &AppHandle,
    task_id: &str,
    finished_at: u64,
    status: RunStatus,
    summary: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|e| e.to_string())?;

    if let Some(mut task) = db.get_scheduled_task(task_id).map_err(|e| e.to_string())? {
        task.last_run_at = Some(finished_at);
        task.last_status = Some(status);
        task.run_count += 1;
        if status == RunStatus::Failed {
            task.error_count += 1;
        }
        task.next_run_at = compute_next_run(&task, finished_at);
        db.upsert_scheduled_task(&task).map_err(|e| e.to_string())?;
    }

    // Trim history.
    db.delete_old_task_runs(task_id, 50).map_err(|e| e.to_string())?;

    // Update the run record with summary/error. We re-read and patch because
    // `update_task_run` above only updates status/duration.
    if let Some(mut run) = db
        .get_task_runs(task_id, 1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
    {
        if run.started_at <= finished_at {
            run.result_summary = summary.to_string();
            run.error_message = error.map(|s| s.to_string());
            db.update_task_run(&run).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

async fn cleanup_running_handle(app: &AppHandle, task_id: &str) {
    let state = app.state::<AppState>();
    let mut guard = state.scheduler_running.lock().await;
    guard.remove(task_id);
}

async fn update_next_run(
    app: &AppHandle,
    task_id: &str,
    next_run_at: Option<u64>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    if let Some(mut task) = db.get_scheduled_task(task_id).map_err(|e| e.to_string())? {
        if task.next_run_at != next_run_at {
            task.next_run_at = next_run_at;
            db.upsert_scheduled_task(&task).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn recover_orphaned_runs(app: &AppHandle, now: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let tasks = db.get_scheduled_tasks().map_err(|e| e.to_string())?;

    for task in tasks {
        if let Some(run) = db.get_running_task_run(&task.id).map_err(|e| e.to_string())? {
            if now.saturating_sub(run.started_at) > RUNNING_TIMEOUT.as_secs() {
                let mut patched = run;
                patched.status = RunStatus::Failed;
                patched.finished_at = Some(now);
                patched.duration_ms = Some((now.saturating_sub(patched.started_at)) * 1000);
                patched.error_message = Some("任务执行超时（可能 App 崩溃过）".to_string());
                db.update_task_run(&patched).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Compute the next scheduled execution time for a task. Returns None for
/// manually-triggered tasks or when the strategy cannot produce a next time.
pub fn compute_next_run(task: &ScheduledTask, now: u64) -> Option<u64> {
    match &task.strategy {
        ScheduleStrategy::Manual => None,
        ScheduleStrategy::Interval { value, unit } => {
            let seconds = match unit {
                crate::models::IntervalUnit::Minutes => *value as u64 * 60,
                crate::models::IntervalUnit::Hours => *value as u64 * 3600,
                crate::models::IntervalUnit::Days => *value as u64 * 86400,
            };
            let base = task.last_run_at.unwrap_or(task.created_at);
            let next = base + seconds;
            if next <= now {
                // If we're already past the next run, schedule for the next
                // whole interval boundary after now to avoid instant re-runs.
                let elapsed = now.saturating_sub(base);
                let missed = elapsed / seconds + 1;
                Some(base + missed * seconds)
            } else {
                Some(next)
            }
        }
        ScheduleStrategy::Cron { expression } => {
            // The `cron` crate demands an explicit seconds field (6-7 fields),
            // but the app stores 5-field expressions ("0 8 * * *") — parsing
            // failed silently and left every cron task unseeded (second half
            // of the dead-scheduler bug, found by the local-clock unit test).
            let normalized = match expression.split_whitespace().count() {
                5 => format!("0 {expression}"),
                _ => expression.clone(),
            };
            let schedule = cron::Schedule::from_str(&normalized).ok()?;
            // Local wall clock: "0 8 * * *" means 08:00 local time, not UTC
            // (UTC evaluation silently turned the morning digest into 16:00 CST).
            let after = chrono::Local.timestamp_opt(now as i64, 0).single()?;
            schedule.after(&after).next().map(|dt| dt.timestamp() as u64)
        }
    }
}
