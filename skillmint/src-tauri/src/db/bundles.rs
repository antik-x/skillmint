use super::*;

impl Db {
    // ----- PRD-02 Phase 2: skill bundles --------------------------------------

    pub fn create_bundle(
        &self,
        device_id: &str,
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<SkillBundle> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO skill_bundles (id, device_id, name, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, device_id, name, description, now, now],
        )?;
        Ok(SkillBundle {
            id: id.to_string(),
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            skill_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn update_bundle(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<SkillBundle> {
        let now = now_secs();
        self.conn.execute(
            "UPDATE skill_bundles SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, description, now, id],
        )?;
        self.get_bundle(id)?.ok_or_else(|| anyhow::anyhow!("Bundle not found"))
    }

    pub fn delete_bundle(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM skill_bundles WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_bundle(&self, id: &str) -> Result<Option<SkillBundle>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                      (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) as skill_count
               FROM skill_bundles b
               WHERE b.id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SkillBundle {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
                skill_count: row.get(5)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_bundles(&self, device_id: &str) -> Result<Vec<SkillBundle>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                      (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) as skill_count
               FROM skill_bundles b
               WHERE b.device_id = ?1
               ORDER BY b.updated_at DESC"#,
        )?;
        let rows = stmt.query_map(params![device_id], |row| {
            Ok(SkillBundle {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
                skill_count: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_bundle_item(
        &self,
        device_id: &str,
        id: &str,
        bundle_id: &str,
        skill_id: &str,
        sort_order: i64,
    ) -> Result<SkillBundleItem> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO skill_bundle_items (id, device_id, bundle_id, skill_id, sort_order, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, device_id, bundle_id, skill_id, sort_order, now],
        )?;
        let skill_name = self
            .get_skill_by_id(skill_id)?
            .map(|s| s.name)
            .unwrap_or_default();
        Ok(SkillBundleItem {
            id: id.to_string(),
            skill_id: skill_id.to_string(),
            skill_name,
            sort_order,
        })
    }

    pub fn remove_bundle_item(&self, bundle_id: &str, skill_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skill_bundle_items WHERE bundle_id = ?1 AND skill_id = ?2",
            params![bundle_id, skill_id],
        )?;
        Ok(())
    }

    pub fn get_bundle_items(&self, bundle_id: &str) -> Result<Vec<SkillBundleItem>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT i.id, i.skill_id, COALESCE(s.name, i.skill_id), i.sort_order
               FROM skill_bundle_items i
               LEFT JOIN skills s ON i.skill_id = s.id
               WHERE i.bundle_id = ?1
               ORDER BY i.sort_order ASC, i.added_at ASC"#,
        )?;
        let rows = stmt.query_map(params![bundle_id], |row| {
            Ok(SkillBundleItem {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                skill_name: row.get(2)?,
                sort_order: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_bundle_item_order(&self, item_id: &str, sort_order: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_bundle_items SET sort_order = ?1 WHERE id = ?2",
            params![sort_order, item_id],
        )?;
        Ok(())
    }
}
