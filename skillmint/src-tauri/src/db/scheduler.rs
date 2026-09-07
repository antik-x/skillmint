use super::*;

impl Db {
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
            default_task(TaskKind::GenerateDailySummary, "每日 AI 摘要生成", "生成前一日的 AI 工作摘要。", ScheduleStrategy::Cron { expression: "0 8 * * *".to_string() }, true),
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
        // Must be a real upsert (ON CONFLICT DO UPDATE), NOT "INSERT OR
        // REPLACE": REPLACE is DELETE+INSERT, and the DELETE of the parent row
        // fires `task_runs.task_id ... ON DELETE CASCADE`, silently wiping the
        // task's entire run history on every finalize (found 2026-09-07 once
        // the engine actually started finishing runs).
        self.conn.execute(
            r#"INSERT INTO scheduled_tasks
               (id, task_kind, name, description, enabled, strategy_kind,
                strategy_value, strategy_unit, strategy_expression,
                created_at, updated_at, last_run_at, last_status,
                next_run_at, run_count, error_count)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
               ON CONFLICT(id) DO UPDATE SET
                 task_kind = excluded.task_kind,
                 name = excluded.name,
                 description = excluded.description,
                 enabled = excluded.enabled,
                 strategy_kind = excluded.strategy_kind,
                 strategy_value = excluded.strategy_value,
                 strategy_unit = excluded.strategy_unit,
                 strategy_expression = excluded.strategy_expression,
                 created_at = excluded.created_at,
                 updated_at = excluded.updated_at,
                 last_run_at = excluded.last_run_at,
                 last_status = excluded.last_status,
                 next_run_at = excluded.next_run_at,
                 run_count = excluded.run_count,
                 error_count = excluded.error_count"#,
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
