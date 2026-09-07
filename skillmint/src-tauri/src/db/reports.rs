use super::*;

impl Db {
    /// Upsert a weekly report.
    pub fn upsert_weekly_report(&self, report: &crate::models::WeeklyReport) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO weekly_reports
             (week_start, content, generated_at, model, provider)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                report.week_start,
                serde_json::to_string(&report.content)?,
                report.generated_at as i64,
                report.model,
                report.provider,
            ],
        )?;
        Ok(())
    }

    /// Load a weekly report by its Monday-start date.
    pub fn get_weekly_report(&self, week_start: &str) -> Result<Option<crate::models::WeeklyReport>> {
        let mut stmt = self.conn.prepare(
            "SELECT week_start, content, generated_at, model, provider
             FROM weekly_reports WHERE week_start = ?1",
        )?;
        let mut rows = stmt.query_map(params![week_start], |row| {
            let content_json: String = row.get(1)?;
            let content = serde_json::from_str(&content_json).unwrap_or_default();
            Ok(crate::models::WeeklyReport {
                week_start: row.get(0)?,
                content,
                generated_at: row.get::<_, i64>(2)? as u64,
                model: row.get(3)?,
                provider: row.get(4)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    // ----- PRD-02: high-value prompts + suggestion report --------------------

    /// Find high-value prompts (repeated >= threshold times) for skill sedimentation.
    ///
    /// P6：接净化口径——只统计用户真实输入（prompt_kind=user 且非 ACP 副产品），
    /// 并把长度门槛从 >4 提到 >=12（挡 "hello"/"你好" 等零信息输入；
    /// discovery/high_value_prompt 检测器另有归一化后 >=10 的门槛）。
    /// 根因修复 2026-09-07：该 SQL 原先绕过了 S2.1.4 净化，
    /// TodoWrite 系统提醒以 4056 次霸榜「高价值 Prompt」。
    pub fn get_high_value_prompts(&self, min_repeat: i64, limit: i64) -> Result<Vec<HighValuePrompt>> {
        // Group by normalized prompt text prefix; count occurrences.
        let mut stmt = self.conn.prepare(
            r#"SELECT substr(prompt_text, 1, 120) AS key,
                      source,
                      COUNT(*) AS cnt,
                      MAX(session_id) AS sample
               FROM collected_prompts
               WHERE prompt_text IS NOT NULL AND length(prompt_text) >= 12
                 AND IFNULL(prompt_kind, 'user') = 'user'
                 AND IFNULL(origin, 'user') != 'skillmint_acp'
               GROUP BY key, source
               HAVING cnt >= ?1
               ORDER BY cnt DESC
               LIMIT ?2"#,
        )?;
        let rows = stmt.query_map(params![min_repeat, limit], |row| {
            Ok(HighValuePrompt {
                prompt_text: row.get::<_, String>(0)?,
                source: row.get::<_, Option<String>>(1)?,
                repeat_count: row.get(2)?,
                sample_session_id: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
