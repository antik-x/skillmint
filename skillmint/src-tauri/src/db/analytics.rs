use super::*;

impl Db {
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
                      AND (?3 = '' OR tu.source = ?3)
                      AND IFNULL(tu.origin, 'user') != 'skillmint_acp'"#;
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
                 AND (?3 = '' OR p.source = ?3)
                 AND IFNULL(p.origin, 'user') != 'skillmint_acp'
                 AND IFNULL(p.prompt_kind, 'user') = 'user'"#;
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
    /// P5/Q8：只统计用户真实输入（origin=user 且 prompt_kind=user）——系统注入
    /// 行与 ACP 副产品行永不进 LLM。
    pub fn count_unclassified_prompts(&self, source: Option<&str>) -> Result<usize> {
        let sql = "SELECT COUNT(*) FROM collected_prompts
                   WHERE IFNULL(prompt_text,'') != '' AND requested_action IS NULL
                     AND (?1 = '' OR source = ?1)
                     AND IFNULL(prompt_kind, 'user') = 'user'
                     AND IFNULL(origin, 'user') = 'user'";
        let src = source.unwrap_or("");
        let n: i64 = self.conn.query_row(sql, params![src], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// PRD-08 §3.3 (P1): load prompts awaiting classification (text present,
    /// no label yet). Returns (id, prompt_text). Same user-only caliber as
    /// `count_unclassified_prompts`（P5/Q8）.
    pub fn load_unclassified_prompts(
        &self,
        source: Option<&str>,
    ) -> Result<Vec<crate::classifier::UnclassifiedPrompt>> {
        let sql = "SELECT id, prompt_text FROM collected_prompts
                   WHERE IFNULL(prompt_text,'') != '' AND requested_action IS NULL
                     AND (?1 = '' OR source = ?1)
                     AND IFNULL(prompt_kind, 'user') = 'user'
                     AND IFNULL(origin, 'user') = 'user'";
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
               AND IFNULL(origin, 'user') != 'skillmint_acp'
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
               AND IFNULL(origin, 'user') != 'skillmint_acp'
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
    /// P5：provider/model 反映真实执行者（acp:<连接名> / cloud:<模型id> /
    /// local），不再硬编码 'skillmint'。
    pub fn save_daily_summary(
        &self,
        date: &str,
        summary: &crate::analyzer::DailySummary,
        model: &str,
        provider: &str,
    ) -> Result<()> {
        let highlights = serde_json::to_string(&summary.highlights)?;
        let activities = serde_json::to_string(&summary.activities)?;
        let now = now_secs();
        self.conn.execute(
            "INSERT OR REPLACE INTO digest_summary
             (date, highlights, activities, model, provider, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![date, highlights, activities, model, provider, now],
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
                 AND IFNULL(p.prompt_kind, 'user') = 'user'
                 AND IFNULL(p.origin, 'user') != 'skillmint_acp'
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
                       AND IFNULL(p.prompt_kind, 'user') = 'user'
                       AND IFNULL(p.origin, 'user') != 'skillmint_acp'
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
                 AND p.duration_ms IS NOT NULL
                 AND IFNULL(p.prompt_kind, 'user') = 'user'
                 AND IFNULL(p.origin, 'user') != 'skillmint_acp'"#,
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
}
