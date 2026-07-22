use super::*;

impl Db {
    // ----- SPEC-C3: trash_items ----------------------------------------------

    /// Insert a trash row pointing at the on-disk snapshot. Returns the row id
    /// so callers can delete the snapshot if a later step fails.
    pub fn insert_trash_item(
        &self,
        item_type: &str,
        original_id: &str,
        original_name: &str,
        snapshot_path: &str,
        metadata: &serde_json::Value,
        deleted_at: u64,
        expires_at: u64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO trash_items
             (item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                item_type,
                original_id,
                original_name,
                snapshot_path,
                metadata.to_string(),
                deleted_at as i64,
                expires_at as i64,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List all trash items, newest first.
    pub fn list_trash_items(&self) -> Result<Vec<crate::models::TrashItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items ORDER BY deleted_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Fetch a single trash item by id.
    pub fn get_trash_item(&self, id: i64) -> Result<Option<crate::models::TrashItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Delete a trash row by id. Does NOT touch the snapshot directory; callers
    /// are responsible for removing the on-disk snapshot.
    pub fn delete_trash_item(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM trash_items WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Return trash rows whose `expires_at` has passed (for startup cleanup).
    pub fn expired_trash_items(&self) -> Result<Vec<crate::models::TrashItem>> {
        let now = now_secs() as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at
             FROM trash_items WHERE expires_at < ?1 ORDER BY expires_at ASC",
        )?;
        let rows = stmt.query_map(params![now], |row| {
            let metadata_json: String = row.get(5)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::TrashItem {
                id: row.get(0)?,
                item_type: row.get(1)?,
                original_id: row.get(2)?,
                original_name: row.get(3)?,
                snapshot_path: row.get(4)?,
                metadata,
                deleted_at: row.get::<_, i64>(6)? as u64,
                expires_at: row.get::<_, i64>(7)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
