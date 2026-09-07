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

    /// 共享的过滤口径：只统计用户真实输入（prompt_kind=user 且非 ACP 副产品）、
    /// 长度 >=12，并排除用户已忽略的分组。分组键 = substr(prompt_text,1,120)。
    const HIGH_VALUE_FILTER: &'static str = r#"FROM collected_prompts cp
               WHERE cp.prompt_text IS NOT NULL AND length(cp.prompt_text) >= 12
                 AND IFNULL(cp.prompt_kind, 'user') = 'user'
                 AND IFNULL(cp.origin, 'user') != 'skillmint_acp'
                 AND NOT EXISTS (
                     SELECT 1 FROM ignored_prompt_groups g
                     WHERE g.group_key = substr(cp.prompt_text, 1, 120)
                       AND IFNULL(g.source, '') = IFNULL(cp.source, '')
                 )"#;

    /// Find high-value prompts (repeated >= threshold times) for skill sedimentation.
    ///
    /// P6：接净化口径——只统计用户真实输入（prompt_kind=user 且非 ACP 副产品），
    /// 并把长度门槛从 >4 提到 >=12（挡 "hello"/"你好" 等零信息输入；
    /// discovery/high_value_prompt 检测器另有归一化后 >=10 的门槛）。
    /// 根因修复 2026-09-07：该 SQL 原先绕过了 S2.1.4 净化，
    /// TodoWrite 系统提醒以 4056 次霸榜「高价值 Prompt」。
    /// 2026-09-07 改版：阈值放宽到 >=2、真分页（LIMIT+OFFSET+总数）、过滤忽略分组。
    pub fn get_high_value_prompt_page(
        &self,
        min_repeat: i64,
        limit: i64,
        offset: i64,
    ) -> Result<crate::models::HighValuePromptPage> {
        let total: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM (
                     SELECT substr(cp.prompt_text, 1, 120) AS key, cp.source
                     {}
                     GROUP BY key, cp.source
                     HAVING COUNT(*) >= ?1
                 )",
                Self::HIGH_VALUE_FILTER
            ),
            params![min_repeat],
            |row| row.get(0),
        )?;

        let mut stmt = self.conn.prepare(&format!(
            r#"SELECT substr(cp.prompt_text, 1, 120) AS key,
                      cp.source,
                      COUNT(*) AS cnt,
                      MAX(cp.session_id) AS sample
               {}
               GROUP BY key, cp.source
               HAVING cnt >= ?1
               ORDER BY cnt DESC, key ASC
               LIMIT ?2 OFFSET ?3"#,
            Self::HIGH_VALUE_FILTER
        ))?;
        let rows = stmt.query_map(params![min_repeat, limit, offset], |row| {
            Ok(HighValuePrompt {
                prompt_text: row.get::<_, String>(0)?,
                source: row.get::<_, Option<String>>(1)?,
                repeat_count: row.get(2)?,
                sample_session_id: row.get(3)?,
            })
        })?;
        Ok(crate::models::HighValuePromptPage {
            items: rows.collect::<Result<Vec<_>, _>>()?,
            total,
        })
    }

    /// 用户忽略一个高频 Prompt 分组（永久生效；prompt_sample 存分组键原文供恢复列表展示）。
    pub fn ignore_prompt_group(
        &self,
        group_key: &str,
        source: &str,
        prompt_sample: Option<&str>,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn.execute(
            "INSERT OR REPLACE INTO ignored_prompt_groups (group_key, source, prompt_sample, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_key, source, prompt_sample, now],
        )?;
        Ok(())
    }

    /// 撤销忽略（恢复该分组重新参与聚合）。
    pub fn unignore_prompt_group(&self, group_key: &str, source: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ignored_prompt_groups WHERE group_key = ?1 AND IFNULL(source, '') = ?2",
            params![group_key, source],
        )?;
        Ok(())
    }

    /// 列出所有被忽略的分组（恢复列表数据源，最近忽略在前）。
    pub fn list_ignored_prompt_groups(&self) -> Result<Vec<crate::models::IgnoredPromptGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT group_key, source, prompt_sample, created_at
             FROM ignored_prompt_groups
             ORDER BY created_at DESC, group_key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::models::IgnoredPromptGroup {
                group_key: row.get(0)?,
                source: row.get(1)?,
                prompt_sample: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
