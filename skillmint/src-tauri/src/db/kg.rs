use super::*;

impl Db {
    pub fn upsert_kg_node(&self, n: &KgNode) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_nodes
               (id, label, type, source, description, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![n.id, n.label, n.node_type, n.source, n.description, now_secs(), now_secs()],
        )?;
        Ok(())
    }

    pub fn link_skill_node(&self, device_id: &str, skill_id: &str, node_id: &str, relevance: f64) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_skill_nodes
               (device_id, skill_id, node_id, relevance, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![device_id, skill_id, node_id, relevance, now_secs()],
        )?;
        Ok(())
    }

    pub fn upsert_kg_edge(&self, e: &KgEdge) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO kg_edges
               (id, device_id, source_id, target_id, relation, weight, reason,
                is_manual, is_rejected, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                e.id,
                e.device_id,
                e.source_id,
                e.target_id,
                e.relation,
                e.weight,
                e.reason,
                e.is_manual as i32,
                e.is_rejected as i32,
                now_secs(),
                now_secs(),
            ],
        )?;
        Ok(())
    }

    /// Reject an auto edge (user clicked ✗); sets is_rejected=1.
    pub fn reject_kg_edge(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE kg_edges SET is_rejected = 1, updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    /// Confirm an auto edge (user clicked ✓); sets is_manual=1.
    pub fn confirm_kg_edge(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE kg_edges SET is_manual = 1, is_rejected = 0, updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    /// Load the full graph (non-rejected edges, all nodes), capped at max_nodes.
    pub fn get_kg_graph(&self, max_nodes: i64) -> Result<KgGraph> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, type, source, description FROM kg_nodes ORDER BY label LIMIT ?1",
        )?;
        let nodes: Vec<KgNode> = stmt
            .query_map(params![max_nodes], |row| {
                Ok(KgNode {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    node_type: row.get(2)?,
                    source: row.get(3)?,
                    description: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        let edges = if node_ids.is_empty() {
            vec![]
        } else {
            // Collect edges among the loaded nodes, excluding rejected ones.
            let placeholders = node_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, device_id, source_id, target_id, relation, weight, reason, is_manual, is_rejected
                 FROM kg_edges
                 WHERE is_rejected = 0 AND source_id IN ({placeholders}) AND target_id IN ({placeholders})"
            );
            let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
            for id in &node_ids {
                params_vec.push(id);
            }
            for id in &node_ids {
                params_vec.push(id);
            }
            let mut stmt2 = self.conn.prepare(&sql)?;
            let edge_rows = stmt2
                .query_map(params_vec.as_slice(), |row| {
                    Ok(KgEdge {
                        id: row.get(0)?,
                        device_id: row.get(1)?,
                        source_id: row.get(2)?,
                        target_id: row.get(3)?,
                        relation: row.get(4)?,
                        weight: row.get(5)?,
                        reason: row.get(6)?,
                        is_manual: row.get::<_, i64>(7)? != 0,
                        is_rejected: row.get::<_, i64>(8)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            edge_rows
        };
        Ok(KgGraph { nodes, edges })
    }

    /// Get all skills (id, name) for graph recommendation lookups.
    pub fn get_skill_names(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM skills ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get the concept nodes linked to a given skill.
    pub fn get_skill_concepts(&self, device_id: &str, skill_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT n.id, n.label FROM kg_skill_nodes sn
               JOIN kg_nodes n ON sn.node_id = n.id
               WHERE sn.device_id = ?1 AND sn.skill_id = ?2 AND n.type = 'concept'"#,
        )?;
        let rows = stmt.query_map(params![device_id, skill_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get all (source_id, target_id) pairs the user rejected (for negative-record persistence).
    pub fn get_rejected_edge_pairs(&self) -> Result<HashSet<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_id FROM kg_edges WHERE is_rejected = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            let a: String = row.get(0)?;
            let b: String = row.get(1)?;
            // Normalize order so pair comparison is order-independent.
            Ok(if a < b { (a, b) } else { (b, a) })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get skill co-occurrence pairs from attribution data grouped by project.
    /// Returns (skill_id_a, skill_id_b, count) for skills used in the same project.
    pub fn get_skill_cooccurrence(&self, device_id: &str) -> Result<Vec<(String, String, i64)>> {
        // Find skills attributed within each project, then pair them.
        let mut stmt = self.conn.prepare(
            r#"SELECT skill_id, project_id FROM skill_usage_attributions
               WHERE device_id = ?1 AND project_id IS NOT NULL AND skill_id IS NOT NULL"#,
        )?;
        let mut project_skills: HashMap<String, Vec<String>> = HashMap::new();
        let rows = stmt.query_map(params![device_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows.filter_map(|r| r.ok()) {
            project_skills.entry(r.1).or_default().push(r.0);
        }
        // Build co-occurrence counts.
        let mut counts: HashMap<(String, String), i64> = HashMap::new();
        for skills in project_skills.values() {
            let deduped: HashSet<&String> = skills.iter().collect();
            let list: Vec<&String> = deduped.into_iter().collect();
            for i in 0..list.len() {
                for j in (i + 1)..list.len() {
                    let (a, b) = if list[i] < list[j] {
                        (list[i].clone(), list[j].clone())
                    } else {
                        (list[j].clone(), list[i].clone())
                    };
                    *counts.entry((a, b)).or_default() += 1;
                }
            }
        }
        Ok(counts.into_iter().map(|((a, b), c)| (a, b, c)).collect())
    }
}
