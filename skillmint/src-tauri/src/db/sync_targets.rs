use super::*;

impl Db {
    // Sync targets
    pub fn insert_sync_target(&self, target: &SyncTarget) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_targets (id, skill_id, agent_id, mode, last_sync_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                target.id,
                target.skill_id,
                target.agent_id,
                target.mode.to_string(),
                target.last_sync_at,
                target.status.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn get_sync_targets(&self) -> Result<Vec<SyncTarget>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT 
                t.id, t.skill_id, s.name as skill_name, t.agent_id, a.name as agent_name,
                t.mode, t.last_sync_at, t.status
            FROM sync_targets t
            JOIN skills s ON t.skill_id = s.id
            JOIN agents a ON t.agent_id = a.id
            ORDER BY s.name, a.name"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let mode_str: String = row.get(5)?;
            let status_str: String = row.get(7)?;
            Ok(SyncTarget {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                agent_id: row.get(3)?,
                agent_name: row.get(4)?,
                mode: parse_sync_mode(&mode_str),
                last_sync_at: row.get(6)?,
                status: parse_sync_status(&status_str),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_sync_target_status(&self, id: &str, status: SyncStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_targets SET status = ?1 WHERE id = ?2",
            params![status.to_string(), id],
        )?;
        Ok(())
    }

    /// Update both status and last_sync_at after a successful sync operation.
    pub fn update_sync_target_synced(&self, id: &str, status: SyncStatus, last_sync_at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_targets SET status = ?1, last_sync_at = ?2 WHERE id = ?3",
            params![status.to_string(), last_sync_at, id],
        )?;
        Ok(())
    }

    pub fn get_sync_target_by_id(&self, id: &str) -> Result<Option<SyncTarget>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT 
                t.id, t.skill_id, s.name as skill_name, t.agent_id, a.name as agent_name,
                t.mode, t.last_sync_at, t.status
            FROM sync_targets t
            JOIN skills s ON t.skill_id = s.id
            JOIN agents a ON t.agent_id = a.id
            WHERE t.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let mode_str: String = row.get(5)?;
            let status_str: String = row.get(7)?;
            Ok(SyncTarget {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                agent_id: row.get(3)?,
                agent_name: row.get(4)?,
                mode: parse_sync_mode(&mode_str),
                last_sync_at: row.get(6)?,
                status: parse_sync_status(&status_str),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Fetch raw sync_targets rows for a skill (used when snapshotting a skill
    /// for the trash bin). Returns serialized JSON so it survives a restore
    /// even if the schema evolves. Joins skills/agents for display names.
    pub fn raw_sync_targets_for_skill(&self, skill_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.skill_id, s.name, t.agent_id, a.name, t.mode, t.last_sync_at, t.status
             FROM sync_targets t
             LEFT JOIN skills s ON t.skill_id = s.id
             LEFT JOIN agents a ON t.agent_id = a.id
             WHERE t.skill_id = ?1",
        )?;
        let rows = stmt.query_map(params![skill_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "skill_id": row.get::<_, String>(1)?,
                "skill_name": row.get::<_, Option<String>>(2)?,
                "agent_id": row.get::<_, String>(3)?,
                "agent_name": row.get::<_, Option<String>>(4)?,
                "mode": row.get::<_, String>(5)?,
                "last_sync_at": row.get::<_, Option<i64>>(6)?,
                "status": row.get::<_, String>(7)?,
            }))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
