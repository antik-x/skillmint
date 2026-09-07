use super::*;

impl Db {
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
        // E2-S2.1.4：入库即打标（user / non_user），保留原文可回溯。NULL 仅存在于
        // 打标功能上线前的老数据，检测器读取时会按规则兜底再判一次。
        let prompt_kind = p
            .prompt_text
            .as_deref()
            .map(|t| {
                if crate::prompt_kind::is_user_prompt(t) {
                    crate::prompt_kind::PROMPT_KIND_USER
                } else {
                    crate::prompt_kind::PROMPT_KIND_NON_USER
                }
            })
            .unwrap_or(crate::prompt_kind::PROMPT_KIND_NON_USER);
        self.conn.execute(
            r#"INSERT OR REPLACE INTO collected_prompts
               (id, device_id, session_id, source, project_id, prompt_text, started_at,
                duration_ms, requested_action, target_object, interaction_state,
                interaction_mode, confidence, tool_calls, tool_errors, prompt_kind)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"#,
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
                prompt_kind,
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
}
