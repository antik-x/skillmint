use super::*;

impl Db {
    // Skills
    pub fn insert_skill(&self, skill: &Skill) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO skills (id, name, repo_path, created_at, updated_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![skill.id, skill.name, skill.repo_path.to_string_lossy().to_string(), skill.created_at, skill.updated_at, skill.status.to_string()],
        )?;
        Ok(())
    }

    pub fn get_skills(&self) -> Result<Vec<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete_skill(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sync_targets WHERE skill_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM skills WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_skill_by_name(&self, name: &str) -> Result<Option<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills WHERE name = ?1",
        )?;
        let mut rows = stmt.query_map(params![name], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get a skill by its primary key id.
    pub fn get_skill_by_id(&self, id: &str) -> Result<Option<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, created_at, updated_at, status FROM skills WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Skill {
                id: row.get(0)?,
                name: row.get(1)?,
                repo_path: PathBuf::from(row.get::<_, String>(2)?),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status: parse_skill_status(&row.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// PRD-09: update skill metadata after an in-app edit (name/path/timestamp).
    pub fn update_skill(&self, skill: &Skill) -> Result<()> {
        self.conn.execute(
            "UPDATE skills SET name = ?1, repo_path = ?2, updated_at = ?3, status = ?4 WHERE id = ?5",
            params![
                skill.name,
                skill.repo_path.to_string_lossy().to_string(),
                skill.updated_at,
                skill.status.to_string(),
                skill.id
            ],
        )?;
        Ok(())
    }

    /// P1-3: update only the lifecycle status of a skill.
    pub fn update_skill_status(&self, id: &str, status: SkillStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE skills SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.to_string(), now_secs(), id],
        )?;
        Ok(())
    }

    /// P1-4: rename a skill in ONE transaction — the row id is preserved so
    /// FK references (kg_skill_nodes, sync_targets) stay intact. The
    /// skill_name shown on sync targets comes from a JOIN with skills.name,
    /// so this single UPDATE refreshes every display automatically.
    pub fn rename_skill_tx(
        &self,
        skill_id: &str,
        new_name: &str,
        new_repo_path: &std::path::Path,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE skills SET name = ?1, repo_path = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                new_name,
                new_repo_path.to_string_lossy().to_string(),
                now_secs(),
                skill_id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}
