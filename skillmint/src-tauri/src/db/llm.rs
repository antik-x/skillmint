use super::*;

impl Db {
    /// P0: convenience helper to log a `llm::ChatOutcome` with request metadata.
    pub fn log_llm_request(
        &self,
        outcome: &crate::llm::ChatOutcome,
        request_kind: &str,
    ) {
        let log = crate::models::LlmRequestLog {
            id: crate::db::new_id(),
            requested_at: now_secs(),
            provider: outcome.provider.clone(),
            fallback: outcome.fallback,
            has_raw_text: true,
            error: outcome.error.clone(),
            metadata_json: serde_json::json!({"kind": request_kind}).to_string(),
        };
        let _ = self.insert_llm_request_log(&log);
    }

    /// P0: insert an LLM/ACP request audit log entry.
    pub fn insert_llm_request_log(&self, log: &crate::models::LlmRequestLog) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO llm_request_logs
               (id, requested_at, provider, fallback, has_raw_text, error, metadata_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                log.id,
                log.requested_at as i64,
                log.provider,
                log.fallback as i32,
                log.has_raw_text as i32,
                log.error,
                log.metadata_json,
            ],
        )?;
        Ok(())
    }

    /// P0: list recent LLM/ACP request audit logs, newest first.
    pub fn list_llm_request_logs(&self, limit: usize) -> Result<Vec<crate::models::LlmRequestLog>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, requested_at, provider, fallback, has_raw_text, error, metadata_json
               FROM llm_request_logs
               ORDER BY requested_at DESC
               LIMIT ?1"#,
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::models::LlmRequestLog {
                id: row.get(0)?,
                requested_at: row.get::<_, i64>(1)?.max(0) as u64,
                provider: row.get(2)?,
                fallback: row.get::<_, i32>(3)? != 0,
                has_raw_text: row.get::<_, i32>(4)? != 0,
                error: row.get(5)?,
                metadata_json: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }
}
