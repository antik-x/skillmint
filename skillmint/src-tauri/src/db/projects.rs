use super::*;

impl Db {
    /// Fetch raw skill_project_bindings rows for a skill.
    pub fn raw_bindings_for_skill(&self, skill_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, skill_id, project_id, agent_id, mode, local_path, is_enabled, pinned_version, updated_at
             FROM skill_project_bindings WHERE skill_id = ?1",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "device_id": row.get::<_, String>(1)?,
                "skill_id": row.get::<_, String>(2)?,
                "project_id": row.get::<_, Option<String>>(3)?,
                "agent_id": row.get::<_, Option<String>>(4)?,
                "mode": row.get::<_, String>(5)?,
                "local_path": row.get::<_, Option<String>>(6)?,
                "is_enabled": row.get::<_, i64>(7)? != 0,
                "pinned_version": row.get::<_, Option<String>>(8)?,
                "updated_at": row.get::<_, i64>(9)?,
            }))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ----- PRD-01: skill-project bindings ------------------------------------

    pub fn upsert_skill_project_binding(&self, b: &SkillProjectBinding) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO skill_project_bindings
               (id, device_id, skill_id, project_id, agent_id, mode, local_path,
                is_enabled, created_at, updated_at, pinned_version)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                b.id,
                b.device_id,
                b.skill_id,
                b.project_id,
                b.agent_id,
                b.mode,
                b.local_path,
                b.is_enabled as i32,
                now_secs(),
                now_secs(),
                b.pinned_version,
            ],
        )?;
        Ok(())
    }

    pub fn get_skill_project_bindings(&self, device_id: &str) -> Result<Vec<SkillProjectBinding>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.device_id, b.skill_id, s.name, b.project_id, p.name,
                      b.agent_id, b.mode, b.local_path, b.is_enabled, b.pinned_version
               FROM skill_project_bindings b
               LEFT JOIN skills s ON b.skill_id = s.id
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.device_id = ?1
               ORDER BY s.name"#,
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok(SkillProjectBinding {
                id: row.get(0)?,
                device_id: row.get(1)?,
                skill_id: row.get(2)?,
                skill_name: row.get(3)?,
                project_id: row.get(4)?,
                project_name: row.get(5)?,
                agent_id: row.get(6)?,
                mode: row.get(7)?,
                local_path: row.get(8)?,
                is_enabled: row.get::<_, i64>(9)? != 0,
                pinned_version: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 patch FR-F: look up a single binding by id (with joins for display names).
    pub fn get_skill_project_binding(&self, id: &str) -> Result<Option<SkillProjectBinding>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.device_id, b.skill_id, s.name, b.project_id, p.name,
                      b.agent_id, b.mode, b.local_path, b.is_enabled, b.pinned_version
               FROM skill_project_bindings b
               LEFT JOIN skills s ON b.skill_id = s.id
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SkillProjectBinding {
                id: row.get(0)?,
                device_id: row.get(1)?,
                skill_id: row.get(2)?,
                skill_name: row.get(3)?,
                project_id: row.get(4)?,
                project_name: row.get(5)?,
                agent_id: row.get(6)?,
                mode: row.get(7)?,
                local_path: row.get(8)?,
                is_enabled: row.get::<_, i64>(9)? != 0,
                pinned_version: row.get(10)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-01 patch FR-F: set the pinned version of a binding (None = follow latest).
    pub fn set_binding_pinned_version(&self, id: &str, version: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_project_bindings SET pinned_version = ?1, updated_at = ?2 WHERE id = ?3",
            params![version, now_secs(), id],
        )?;
        Ok(())
    }

    /// PRD-01 patch FR-F: which versions are pinned by any binding for a skill?
    pub fn get_pinned_versions_for_skill(&self, skill_id: &str) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT pinned_version FROM skill_project_bindings
             WHERE skill_id = ?1 AND pinned_version IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// PRD-01 patch FR-F §4.5d: for a given (skill, version), return the project
    /// names whose bindings pin to that version. Used to populate
    /// SkillVersion.pinned_by.
    pub fn get_pinned_projects_for_version(
        &self,
        skill_id: &str,
        version: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT p.name
               FROM skill_project_bindings b
               LEFT JOIN projects p ON b.project_id = p.id
               WHERE b.skill_id = ?1 AND b.pinned_version = ?2"#,
        )?;
        let rows = stmt.query_map(params![skill_id, version], |row| {
            row.get::<_, Option<String>>(0)
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .flatten()
            .collect())
    }

    pub fn delete_skill_project_binding(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM skill_project_bindings WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_projects(&self, device_id: &str) -> Result<Vec<(String, String, String, Option<u64>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, last_active_at FROM projects WHERE device_id = ?1 ORDER BY last_active_at DESC NULLS LAST",
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?.map(|t| t as u64),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// PRD-01 §3.2c: classify projects as stale (>30 days inactive) or path-invalid.
    /// Updates `is_stale` flag and returns the count flagged stale.
    pub fn classify_projects(&self) -> Result<i64> {
        let now = now_secs();
        let stale_threshold = now.saturating_sub(30 * 86400);
        // Mark stale: last_active older than 30 days (or never active).
        self.conn.execute(
            "UPDATE projects SET is_stale = 1 WHERE last_active_at IS NOT NULL AND last_active_at < ?1",
            params![stale_threshold],
        )?;
        // Count how many were flagged.
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM projects WHERE is_stale = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    /// Remove a project record (and its agent_instances).
    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM agent_instances WHERE project_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Promote a project-local skill to global: move its directory into center_repo
    /// and update the skill's repo_path. Returns the new repo_path.
    pub fn promote_skill_to_global(&self, skill_id: &str, center_repo: &std::path::Path) -> Result<String> {
        let skill = self
            .get_skill_by_id(skill_id)?
            .ok_or_else(|| anyhow::anyhow!("skill not found"))?;
        let dest = center_repo.join(&skill.name);
        if !dest.exists() {
            std::fs::rename(&skill.repo_path, &dest)
                .or_else(|_| {
                    // rename may fail across volumes; fall back to copy.
                    crate::fs::copy_dir_all(&skill.repo_path, &dest)?;
                    crate::fs::remove_path(&skill.repo_path).ok();
                    Ok::<(), anyhow::Error>(())
                })?;
        }
        self.conn.execute(
            "UPDATE skills SET repo_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![dest.to_string_lossy(), now_secs(), skill_id],
        )?;
        Ok(dest.to_string_lossy().to_string())
    }
}
