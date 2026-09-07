use super::*;

impl Db {
    // ----- PRD-01/02: project discovery + linking ----------------------------

    /// Get or create a project row for the given path, returning its id.
    /// Path is normalized (canonicalized) before lookup. Name is derived from the
    /// last path segment.
    pub fn ensure_project_by_path(&self, device_id: &str, raw_path: &str) -> Result<Option<String>> {
        Self::ensure_project_by_path_conn(&self.conn, device_id, raw_path)
    }

    /// Link collected sessions that carry a `project_path` to the projects table,
    /// and upsert agent_instances counts. Returns the number of sessions linked.
    pub fn link_sessions_to_projects(&self, device_id: &str) -> Result<i64> {
        Self::link_sessions_to_projects_conn(&self.conn, device_id)
    }

    /// Transaction-aware core of `link_sessions_to_projects` (P3-10): the
    /// hygiene migration re-runs the whole linking pass inside its transaction.
    pub fn link_sessions_to_projects_conn(
        conn: &rusqlite::Connection,
        device_id: &str,
    ) -> Result<i64> {
        // Read all (session_id, source, project_path, start_time) that are unlinked.
        let mut stmt = conn.prepare(
            "SELECT id, source, project_path, start_time FROM collected_sessions
             WHERE project_path IS NOT NULL AND project_path <> ''",
        )?;
        let rows: Vec<(String, String, String, Option<u64>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .map(|(id, src, path, ts)| (id, src, path, ts.map(|t| t as u64)))
            .collect();
        drop(stmt);

        let mut linked = 0i64;
        for (session_id, source, path, start_ts) in rows {
            let project_id = match Self::ensure_project_by_path_conn(conn, device_id, &path)? {
                Some(p) => p,
                None => continue,
            };
            // Update the session's project_id.
            conn.execute(
                "UPDATE collected_sessions SET project_id = ?1 WHERE id = ?2",
                params![project_id, session_id],
            )?;
            // Update token_usage rows for this session to carry the project_id too.
            conn.execute(
                "UPDATE collected_token_usage SET project_id = ?1 WHERE session_id = ?2",
                params![project_id, session_id],
            )?;
            // PRD-06 §3.2: attribute the session to an Agent via `agents.source`
            // (deterministic equality), replacing the old `derive_agent_id` hack.
            // If no Agent carries this source, skip attribution but still link the project.
            let agent_key: Option<String> = conn
                .query_row(
                    "SELECT id FROM agents WHERE source = ?1 LIMIT 1",
                    params![&source],
                    |row| row.get(0),
                )
                .ok();
            if let Some(agent_key) = agent_key {
                let ai_id = format!("{device_id}:{agent_key}:{project_id}");
                let now = now_secs();
                conn.execute(
                    r#"INSERT INTO agent_instances
                       (id, device_id, agent_id, project_id, last_session_at, session_count,
                        total_tokens, total_prompts, created_at, updated_at)
                       VALUES (?1, ?2, ?3, ?4, ?5, 1, 0, 0, ?6, ?7)
                       ON CONFLICT(device_id, agent_id, project_id) DO UPDATE SET
                         last_session_at = MAX(COALESCE(excluded.last_session_at, last_session_at), COALESCE(last_session_at, 0)),
                         session_count = agent_instances.session_count + 1,
                         updated_at = excluded.updated_at"#,
                    params![ai_id, device_id, agent_key, project_id, start_ts.unwrap_or(now), now, now],
                )?;
            }
            linked += 1;
        }

        // Populate agents.last_used_at and project_count from agent_instances (PRD-01 §5.1).
        // P3-10: update every agent — the old `WHERE EXISTS` guard left stale
        // counts on agents whose instances were wiped (e.g. by the hygiene
        // migration after their sessions turned out to be phantom links).
        let now = now_secs();
        conn.execute(
            r#"UPDATE agents SET
                 last_used_at = (
                    SELECT MAX(ai.last_session_at) FROM agent_instances ai
                    WHERE ai.agent_id = agents.id
                 ),
                 project_count = (
                    SELECT COUNT(DISTINCT ai.project_id) FROM agent_instances ai
                    WHERE ai.agent_id = agents.id
                 ),
                 updated_at = ?1"#,
            params![now],
        )?;
        Ok(linked)
    }

    /// P3-10: full rebuild of the agent↔project association data from
    /// `collected_sessions`. Drops phantom projects (relative cwds, tool data
    /// dirs — see `is_linkable_project_path`), merges duplicate spellings of the
    /// same path, then resets all session links and re-runs the linking pass so
    /// `agent_instances` / `agents.project_count` / `last_used_at` are derived
    /// from clean data. Runs inside the caller's transaction (hygiene migration).
    /// Returns (phantom_projects_removed, duplicates_merged, sessions_linked).
    pub fn rebuild_project_links_tx(
        tx: &rusqlite::Transaction<'_>,
        device_id: &str,
    ) -> Result<(usize, usize, i64)> {
        // 1. Wipe agent_instances — rebuilt from scratch below.
        tx.execute("DELETE FROM agent_instances", [])?;

        // 2. Classify existing projects.
        let mut stmt = tx.prepare("SELECT id, path FROM projects")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        let mut phantom: Vec<String> = Vec::new();
        // (device_id, normalized path) -> [(id, path, created_at)]
        let mut groups: std::collections::HashMap<(String, String), Vec<(String, String, i64)>> =
            std::collections::HashMap::new();
        for (id, path) in rows {
            if !is_linkable_project_path(&path) {
                phantom.push(id);
                continue;
            }
            let normalized = normalize_path(&path);
            let created: i64 = tx
                .query_row(
                    "SELECT COALESCE(created_at, 0) FROM projects WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            groups
                .entry((device_id.to_string(), normalized))
                .or_default()
                .push((id, path, created));
        }

        // 3. Drop phantom projects; detach child references first (the FKs on
        //    skill_project_bindings have no cascade).
        for id in &phantom {
            tx.execute(
                "UPDATE skill_project_bindings SET project_id = NULL WHERE project_id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        }

        // 4. Merge duplicate spellings of the same physical project. Keeper:
        //    the row already spelled exactly like the normalized path, else the
        //    earliest created. The keeper's path is rewritten to the normalized
        //    spelling so re-linking cannot resurrect the duplicates.
        let mut merged = 0usize;
        for ((_device, normalized), mut entries) in groups {
            if entries.len() <= 1 {
                continue;
            }
            entries.sort_by(|a, b| {
                let a_exact = a.1 == normalized;
                let b_exact = b.1 == normalized;
                b_exact.cmp(&a_exact).then_with(|| a.2.cmp(&b.2))
            });
            let (keeper_id, keeper_path, _) = entries[0].clone();
            if keeper_path != normalized {
                tx.execute(
                    "UPDATE projects SET path = ?1 WHERE id = ?2",
                    params![normalized, keeper_id],
                )?;
            }
            for (dup_id, _, _) in entries.iter().skip(1) {
                tx.execute(
                    "UPDATE collected_sessions SET project_id = ?1 WHERE project_id = ?2",
                    params![keeper_id, dup_id],
                )?;
                tx.execute(
                    "UPDATE collected_token_usage SET project_id = ?1 WHERE project_id = ?2",
                    params![keeper_id, dup_id],
                )?;
                // UPDATE OR IGNORE: a binding may already exist for the keeper
                // (UNIQUE device/skill/project/agent) — dropping the dup copy is
                // the desired dedup, not a failure.
                tx.execute(
                    "UPDATE OR IGNORE skill_project_bindings SET project_id = ?1 WHERE project_id = ?2",
                    params![keeper_id, dup_id],
                )?;
                tx.execute("DELETE FROM projects WHERE id = ?1", params![dup_id])?;
                merged += 1;
            }
        }

        // 5. Reset every session link and agent counter, then relink cleanly.
        //    Sessions whose path is unlinkable keep project_id NULL — that is
        //    the point: no more phantom rows.
        tx.execute("UPDATE collected_sessions SET project_id = NULL", [])?;
        tx.execute("UPDATE collected_token_usage SET project_id = NULL", [])?;
        tx.execute("UPDATE agents SET project_count = 0, last_used_at = NULL", [])?;
        let linked = Self::link_sessions_to_projects_conn(tx, device_id)?;
        Ok((phantom.len(), merged, linked))
    }

    /// Look up a skill id by name (case-insensitive). Returns None if no such skill.
    pub fn skill_id_by_name(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM skills WHERE lower(name) = lower(?1)",
                params![name],
                |row| row.get(0),
            )
            .ok())
    }

    // ----- PRD-02: skill usage attribution -----------------------------------

    pub fn upsert_skill_attribution(&self, a: &SkillUsageAttribution) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO skill_usage_attributions
               (id, device_id, skill_id, skill_name, source, session_id, project_id,
                agent_id, attribution_type, attributed_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                a.id,
                a.device_id,
                a.skill_id,
                a.skill_name,
                a.source,
                a.session_id,
                a.project_id,
                a.agent_id,
                a.attribution_type,
                a.attributed_at,
            ],
        )?;
        Ok(())
    }

    /// Aggregate skill usage over the last N days (days==0 => all time).
    pub fn get_skill_usage_summary(&self, days: u32, now: u64) -> Result<Vec<SkillUsageSummary>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT skill_name,
                      MAX(skill_id),
                      COUNT(*) AS usage_count,
                      COUNT(DISTINCT session_id) AS session_count,
                      COUNT(DISTINCT project_id) AS project_count
               FROM skill_usage_attributions
               WHERE IFNULL(attributed_at, 0) >= ?1
               GROUP BY skill_name
               ORDER BY usage_count DESC"#,
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok(SkillUsageSummary {
                skill_name: row.get(0)?,
                skill_id: row.get(1)?,
                usage_count: row.get(2)?,
                session_count: row.get(3)?,
                project_count: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-09 §3.3c: per-skill discovery metrics used to rank the Discover page.
    ///
    /// Returns a map keyed by lowercased skill name. `usage_count` comes from
    /// `skill_usage_attributions`; `correction_count` counts prompts in the
    /// same sessions whose `interaction_state` is "补充澄清" or "纠偏修正".
    /// Days==0 means all time.
    pub fn get_skill_discovery_metrics(
        &self,
        days: u32,
        now: u64,
    ) -> Result<std::collections::HashMap<String, (i64, i64)>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT
                 a.skill_name,
                 COUNT(*) AS usage_count,
                 COALESCE(SUM(CASE
                   WHEN p.interaction_state IN ('补充澄清', '纠偏修正') THEN 1
                   ELSE 0
                 END), 0) AS correction_count
               FROM skill_usage_attributions a
               LEFT JOIN collected_prompts p
                 ON a.device_id = p.device_id AND a.session_id = p.session_id
               WHERE IFNULL(a.attributed_at, 0) >= ?1
               GROUP BY a.skill_name"#,
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (name, usage, correction) = row?;
            map.insert(name.to_lowercase(), (usage, correction));
        }
        Ok(map)
    }

    /// PRD-02 §3.2: per-day timeseries of token usage for a source.
    /// Returns Vec<(date_string, token_count)> bucketed by day.
    pub fn get_usage_timeseries(&self, source: &str, days: u32) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT date(s.start_time, 'unixepoch', 'localtime') AS day,
                      SUM(COALESCE(tu.tokens, 0)) AS tokens
               FROM collected_sessions s
               LEFT JOIN (
                 SELECT session_id, SUM(total_tokens) AS tokens
                 FROM collected_token_usage
                 GROUP BY session_id
               ) tu ON tu.session_id = s.id
               WHERE s.source = ?1 AND s.start_time IS NOT NULL
               GROUP BY day
               ORDER BY day"#,
        )?;
        let rows = stmt.query_map(params![source], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let all: Vec<(String, i64)> = rows.filter_map(|r| r.ok()).collect();
        if days == 0 {
            return Ok(all);
        }
        // Trim to last N days.
        let len = all.len();
        let start = len.saturating_sub(days as usize);
        Ok(all[start..].to_vec())
    }

    /// PRD-02 §3.3c: skill health score (0-100) based on usage, coverage, and correction rate.
    /// Returns (score, suggestion) for a single skill.
    pub fn get_skill_health(&self, skill_name: &str, days: u32, now: u64) -> Result<(f64, String)> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let usage: Option<(i64, i64, i64)> = self.conn.query_row(
            r#"SELECT COUNT(*), COUNT(DISTINCT session_id), COUNT(DISTINCT project_id)
               FROM skill_usage_attributions
               WHERE skill_name = ?1 AND IFNULL(attributed_at, 0) >= ?2"#,
            params![skill_name, cutoff],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).ok();
        let (count, sessions, projects) = usage.unwrap_or((0, 0, 0));

        // Health = usage density (40pts) + coverage breadth (40pts) + recency bonus (20pts).
        let density = (count as f64 * 5.0).min(40.0);
        let coverage = ((sessions as f64 * 4.0) + (projects as f64 * 8.0)).min(40.0);
        let recency = if count > 0 { 20.0 } else { 0.0 };
        let score = (density + coverage + recency).round();

        let suggestion = if score < 30.0 {
            format!("「{}」使用率低，建议检查描述是否清晰或是否已被弃用。", skill_name)
        } else if score < 60.0 && projects <= 1 {
            format!("「{}」覆盖项目少（{}），考虑推广到更多项目。", skill_name, projects)
        } else {
            String::new()
        };
        Ok((score, suggestion))
    }

    /// Per-project usage over the last N days (project profile).
    pub fn get_project_usage_summary(
        &self,
        device_id: &str,
        days: u32,
        now: u64,
    ) -> Result<Vec<ProjectUsageSummary>> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };
        let mut stmt = self.conn.prepare(
            r#"SELECT p.id, p.name, p.path,
                      COUNT(DISTINCT s.id) AS session_count,
                      COALESCE(SUM(t.total_tokens), 0) AS total_tokens
               FROM projects p
               LEFT JOIN collected_sessions s
                 ON s.project_id = p.id AND IFNULL(s.start_time, 0) >= ?1
                    AND IFNULL(s.origin, 'user') != 'skillmint_acp'
               LEFT JOIN collected_token_usage t
                 ON t.session_id = s.id
               WHERE p.device_id = ?2
               GROUP BY p.id
               ORDER BY total_tokens DESC"#,
        )?;
        let rows = stmt.query_map(params![cutoff, device_id], |row| {
            Ok(ProjectUsageSummary {
                project_id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                session_count: row.get(3)?,
                total_tokens: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Aggregate usage for a source over the last `days` days (P0 data-tab payload).
    /// `days == 0` means "all time".
    pub fn get_agent_usage_summary(
        &self,
        source: &str,
        days: u32,
        now: u64,
    ) -> Result<AgentUsageSummary> {
        let cutoff = if days == 0 { 0 } else { now.saturating_sub((days as u64) * 86400) };

        let session_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collected_sessions WHERE source = ?1 AND IFNULL(start_time, 0) >= ?2
             AND IFNULL(origin, 'user') != 'skillmint_acp'",
            params![source, cutoff],
            |row| row.get(0),
        )?;
        let prompt_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM collected_prompts WHERE source = ?1 AND IFNULL(started_at, 0) >= ?2
             AND IFNULL(origin, 'user') != 'skillmint_acp'",
            params![source, cutoff],
            |row| row.get(0),
        )?;
        let totals = self.conn.query_row(
            r#"SELECT
                  COALESCE(SUM(input_tokens), 0),
                  COALESCE(SUM(output_tokens), 0),
                  COALESCE(SUM(cache_read_input_tokens), 0),
                  COALESCE(SUM(cache_creation_input_tokens), 0),
                  COALESCE(SUM(total_tokens), 0)
               FROM collected_token_usage
               WHERE source = ?1
                 AND IFNULL(origin, 'user') != 'skillmint_acp'
                 AND session_id IN (SELECT id FROM collected_sessions WHERE IFNULL(start_time,0) >= ?2)"#,
            params![source, cutoff],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?, // input
                    row.get::<_, i64>(1)?, // output
                    row.get::<_, i64>(2)?, // cache_read
                    row.get::<_, i64>(3)?, // cache_creation
                    row.get::<_, i64>(4)?, // total
                ))
            },
        )?;

        Ok(AgentUsageSummary {
            source: source.to_string(),
            days,
            session_count,
            prompt_count,
            input_tokens: totals.0,
            output_tokens: totals.1,
            cache_read_input_tokens: totals.2,
            cache_creation_input_tokens: totals.3,
            total_tokens: totals.4,
        })
    }

    /// Get projects associated with an agent (via agent_instances).
    pub fn get_agent_projects(&self, device_id: &str, agent_id: &str) -> Result<Vec<ProjectUsageSummary>> {
        // PRD-06 §3.2: join collected sessions by `agents.source` (deterministic),
        // replacing the old `CASE WHEN name LIKE 'Claude%'` string-guessing hack.
        let mut stmt = self.conn.prepare(
            r#"SELECT p.id, p.name, p.path, COUNT(DISTINCT s.id), COALESCE(SUM(t.total_tokens),0)
               FROM agent_instances ai
               JOIN projects p ON ai.project_id = p.id
               LEFT JOIN collected_sessions s
                 ON s.project_id = p.id
                AND s.source = (SELECT a.source FROM agents a WHERE a.id = ai.agent_id)
               LEFT JOIN collected_token_usage t ON t.session_id = s.id
               WHERE ai.device_id = ?1 AND ai.agent_id = ?2
               GROUP BY p.id ORDER BY p.last_active_at DESC"#,
        )?;
        let rows = stmt.query_map(params![device_id, agent_id], |row| {
            Ok(ProjectUsageSummary {
                project_id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                session_count: row.get(3)?,
                total_tokens: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-A/B: all agents active in a single project (with usage stats).
    pub fn get_project_agents(
        &self,
        device_id: &str,
        project_id: &str,
    ) -> Result<Vec<ProjectAgentEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT a.id, a.name, a.skill_directory, a.is_enabled,
                      ai.last_session_at, ai.session_count, ai.total_tokens
               FROM agent_instances ai
               JOIN agents a ON ai.agent_id = a.id
               WHERE ai.device_id = ?1 AND ai.project_id = ?2
               ORDER BY ai.total_tokens DESC, ai.last_session_at DESC NULLS LAST"#,
        )?;
        let rows = stmt.query_map(params![device_id, project_id], |row| {
            Ok(ProjectAgentEntry {
                agent_id: row.get(0)?,
                agent_name: row.get(1)?,
                skill_directory: row.get(2)?,
                is_enabled: row.get::<_, i64>(3)? != 0,
                last_session_at: row.get::<_, Option<i64>>(4)?.map(|t| t as u64),
                session_count: row.get(5)?,
                total_tokens: row.get(6)?,
                skill_count: None,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-A: a project's aggregate usage (sessions + tokens).
    pub fn get_project_aggregate(&self, device_id: &str, project_id: &str) -> Result<(i64, i64)> {
        let row: (i64, i64) = self.conn.query_row(
            r#"SELECT COUNT(DISTINCT s.id), COALESCE(SUM(t.total_tokens), 0)
               FROM collected_sessions s
               LEFT JOIN collected_token_usage t ON t.session_id = s.id
               WHERE s.device_id = ?1 AND s.project_id = ?2"#,
            params![device_id, project_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(row)
    }
}
