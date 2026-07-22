use super::*;

impl Db {
    // Agents
    pub fn insert_agent(&self, agent: &Agent) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO agents (id, name, skill_directory, is_enabled, discovery_rule, description, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![agent.id, agent.name, agent.skill_directory.to_string_lossy().to_string(), agent.is_enabled as i32, agent.discovery_rule, agent.description, agent.source],
        )?;
        Ok(())
    }

    pub fn get_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.conn.prepare("SELECT id, name, skill_directory, is_enabled, discovery_rule, description, source FROM agents ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(Agent {
                id: row.get(0)?,
                name: row.get(1)?,
                skill_directory: PathBuf::from(row.get::<_, String>(2)?),
                is_enabled: row.get::<_, i32>(3)? != 0,
                discovery_rule: row.get(4)?,
                description: row.get(5)?,
                source: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-06 §5.2: insert an agent directory sub-record.
    pub fn insert_agent_directory(&self, dir: &AgentDirectory) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_directories (id, agent_id, path, role, is_enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![dir.id, dir.agent_id, dir.path.to_string_lossy().to_string(), dir.role, dir.is_enabled as i32, dir.created_at],
        )?;
        Ok(())
    }

    /// PRD-06 §5.2: list all directories owned by an agent.
    pub fn get_agent_directories(&self, agent_id: &str) -> Result<Vec<AgentDirectory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, path, role, is_enabled, created_at FROM agent_directories WHERE agent_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(AgentDirectory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                role: row.get(3)?,
                is_enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Look up a single agent directory by id.
    pub fn get_agent_directory_by_id(&self, directory_id: &str) -> Result<Option<AgentDirectory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, path, role, is_enabled, created_at FROM agent_directories WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![directory_id], |row| {
            Ok(AgentDirectory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                role: row.get(3)?,
                is_enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-06 §3.3: delete an agent directory by id. Caller owns any disk-side
    /// decisions (we only remove the row + its sync_targets here, never the files).
    /// Returns the count of sync_targets orphaned by the deletion, so the caller
    /// can warn the user / offer cleanup.
    pub fn delete_agent_directory(&self, directory_id: &str) -> Result<usize> {
        let orphaned = self.conn.execute(
            "DELETE FROM sync_targets WHERE agent_directory_id = ?1",
            params![directory_id],
        )?;
        let deleted = self.conn.execute(
            "DELETE FROM agent_directories WHERE id = ?1",
            params![directory_id],
        )?;
        if deleted == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(orphaned)
    }

    /// PRD-06 §3.3: rename the role label of an agent directory (free-form string).
    pub fn update_agent_directory_role(
        &self,
        directory_id: &str,
        role: Option<&str>,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE agent_directories SET role = ?1 WHERE id = ?2",
            params![role, directory_id],
        )?;
        if updated == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(())
    }

    /// PRD-06 §3.3: toggle whether a directory participates in scan/sync.
    pub fn update_agent_directory_enabled(
        &self,
        directory_id: &str,
        is_enabled: bool,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE agent_directories SET is_enabled = ?1 WHERE id = ?2",
            params![is_enabled as i32, directory_id],
        )?;
        if updated == 0 {
            anyhow::bail!("agent directory not found: {directory_id}");
        }
        Ok(())
    }

    /// Read cached skill scan results for one agent directory.
    pub fn get_directory_skills(
        &self,
        agent_id: &str,
        path: &std::path::Path,
    ) -> Result<Vec<AgentSkillItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, exists_in_center, content_match FROM agent_directory_skills
             WHERE agent_id = ?1 AND path = ?2 ORDER BY name",
        )?;
        let rows = stmt.query_map(
            params![agent_id, path.to_string_lossy().to_string()],
            |row| {
                Ok(AgentSkillItem {
                    name: row.get(0)?,
                    exists_in_center: row.get::<_, i64>(1)? != 0,
                    content_match: row.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Persist skill scan results for one agent directory, replacing any prior rows.
    pub fn replace_directory_skills(
        &mut self,
        agent_id: &str,
        path: &std::path::Path,
        items: &[AgentSkillItem],
        scanned_at: u64,
    ) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
            params![agent_id, &path_str],
        )?;
        let mut stmt = tx.prepare(
            "INSERT INTO agent_directory_skills
             (agent_id, path, name, exists_in_center, content_match, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for item in items {
            stmt.execute(params![
                agent_id,
                &path_str,
                &item.name,
                item.exists_in_center as i32,
                item.content_match.map(|v| v as i32),
                scanned_at as i64,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    /// Drop cached scan results for one agent directory.
    pub fn delete_directory_skills(
        &self,
        agent_id: &str,
        path: &std::path::Path,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
            params![agent_id, path.to_string_lossy().to_string()],
        )?;
        Ok(())
    }

    /// Drop cached scan results for an entire agent (used when the agent is removed/rescanned).
    pub fn delete_agent_directory_skills(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_directory_skills WHERE agent_id = ?1",
            params![agent_id],
        )?;
        Ok(())
    }

    /// Drop all cached directory-skill scan results. Infrequent: used after global
    /// sync or center-repo changes that could affect every agent's content_match.
    pub fn clear_directory_skill_cache(&self) -> Result<()> {
        self.conn.execute("DELETE FROM agent_directory_skills", [])?;
        Ok(())
    }

    /// Batch count of cached skills per agent, scoped to each agent's primary skill_directory.
    pub fn count_agent_directory_skills(
        &self,
        agent_id_to_path: &[(String, PathBuf)],
    ) -> Result<HashMap<String, usize>> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        if agent_id_to_path.is_empty() {
            return Ok(counts);
        }
        // SQLite does not support tuple IN, so query per agent. The batch is typically small
        // (tens of agents) and this keeps the query simple and indexed.
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*) FROM agent_directory_skills WHERE agent_id = ?1 AND path = ?2",
        )?;
        for (agent_id, path) in agent_id_to_path {
            let n: i64 = stmt.query_row(
                params![agent_id, path.to_string_lossy().to_string()],
                |row| row.get(0),
            )?;
            counts.insert(agent_id.clone(), n as usize);
        }
        Ok(counts)
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sync_targets WHERE agent_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM agents WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Get a single agent by id (with last_used_at / project_count).
    pub fn get_agent_by_id(&self, id: &str) -> Result<Option<Agent>> {        let mut stmt = self.conn.prepare(
            "SELECT id, name, skill_directory, is_enabled, discovery_rule, description, source FROM agents WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Agent {
                id: row.get(0)?,
                name: row.get(1)?,
                skill_directory: PathBuf::from(row.get::<_, String>(2)?),
                is_enabled: row.get::<_, i64>(3)? != 0,
                discovery_rule: row.get(4)?,
                description: row.get(5)?,
                source: row.get(6)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get last_used_at and project_count for an agent.
    pub fn get_agent_usage_meta(&self, id: &str) -> Result<(Option<u64>, i64)> {
        let row: (Option<i64>, Option<i64>) = self.conn.query_row(
            "SELECT last_used_at, project_count FROM agents WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((row.0.map(|t| t as u64), row.1.unwrap_or(0)))
    }
}
