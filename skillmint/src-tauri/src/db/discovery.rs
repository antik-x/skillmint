use super::*;

impl Db {
    // ----- SPEC-I2: discovery inbox + weekly reports -------------------------

    /// Insert or skip discovery rows. For each candidate, if a row with the same
    /// dedup_key already exists in `pending` status, skip it. Otherwise insert as
    /// pending. Returns the number actually inserted.
    pub fn insert_discoveries(&mut self, discoveries: &[crate::models::Discovery]) -> Result<usize> {
        let mut inserted = 0usize;
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO discoveries
                 (id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                 WHERE NOT EXISTS (
                     SELECT 1 FROM discoveries WHERE dedup_key = ?6 AND status = 'pending'
                 )",
            )?;
            for d in discoveries {
                let rows = stmt.execute(params![
                    d.id,
                    d.kind.to_string(),
                    d.title,
                    d.payload.to_string(),
                    d.confidence,
                    d.dedup_key,
                    d.status.to_string(),
                    d.created_at as i64,
                    d.decided_at.map(|t| t as i64),
                    d.resulting_skill_id,
                ])?;
                inserted += rows;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// List discoveries filtered by status (empty string = all).
    pub fn list_discoveries(&self, status: Option<&str>) -> Result<Vec<crate::models::Discovery>> {
        match status {
            None | Some("") => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
                     FROM discoveries ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map([], map_discovery_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
            Some(s) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
                     FROM discoveries WHERE status = ?1 ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(params![s], map_discovery_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
        }
    }

    /// Update a discovery's status, decided_at, and optional resulting_skill_id.
    pub fn update_discovery_status(
        &self,
        id: &str,
        status: crate::models::DiscoveryStatus,
        decided_at: Option<u64>,
        resulting_skill_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE discoveries
             SET status = ?1, decided_at = ?2, resulting_skill_id = ?3
             WHERE id = ?4",
            params![
                status.to_string(),
                decided_at.map(|t| t as i64),
                resulting_skill_id,
                id,
            ],
        )?;
        Ok(())
    }

    /// Get a single discovery by id.
    pub fn get_discovery_by_id(&self, id: &str) -> Result<Option<crate::models::Discovery>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at, resulting_skill_id
             FROM discoveries WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_discovery_row)?;
        Ok(rows.next().transpose()?)
    }

    /// True if the dedup_key was dismissed within `days`.
    pub fn is_dedup_cooling(&self, dedup_key: &str, days: u32) -> Result<bool> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM discoveries
             WHERE dedup_key = ?1 AND status = 'dismissed' AND decided_at >= ?2",
            params![dedup_key, cutoff],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Mark pending discoveries older than `days` as expired.
    pub fn expire_stale_discoveries(&self, days: u32) -> Result<usize> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let updated = self.conn.execute(
            "UPDATE discoveries
             SET status = 'expired'
             WHERE status = 'pending' AND created_at < ?1",
            params![cutoff],
        )?;
        Ok(updated)
    }

    // ----- SPEC-C1: adoption_events ------------------------------------------

    /// Record an adoption decision. Called inside the same logical step as the
    /// discoveries status update so the ledger is always consistent with the
    /// inbox (callers wrap both in a transaction when they need atomicity).
    pub fn insert_adoption_event(
        &self,
        discovery_id: &str,
        decision: &str,
        reject_reason: Option<&str>,
        resulting_skill_id: Option<&str>,
        decided_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO adoption_events
             (discovery_id, decision, reject_reason, resulting_skill_id, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                discovery_id,
                decision,
                reject_reason,
                resulting_skill_id,
                decided_at as i64,
            ],
        )?;
        Ok(())
    }

    /// Most recent adoption_event for a discovery (by id), if any. Used to pick
    /// the cooling tier after a rejection.
    pub fn latest_adoption_event(&self, discovery_id: &str) -> Result<Option<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT decision, reject_reason FROM adoption_events
             WHERE discovery_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![discovery_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Most recent `rejected` reason for a dedup_key. Drives the tiered cooling
    /// (SPEC-C1 T3): duplicate=90d, trivial=30d, wrong=14d. Returns None when
    /// there is no recorded rejection (historical dismissed items fall back to
    /// the legacy uniform window).
    pub fn latest_reject_reason_for_dedup(&self, dedup_key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT ae.reject_reason FROM adoption_events ae
             JOIN discoveries d ON d.id = ae.discovery_id
             WHERE d.dedup_key = ?1 AND ae.decision = 'rejected'
               AND ae.reject_reason IS NOT NULL
             ORDER BY ae.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![dedup_key], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    // ----- SPEC-C1: gate_rejections ------------------------------------------

    /// Record a gate rejection. The pipeline calls this at the three discard
    /// points (below threshold / daily limit / cooling) plus the rule gate.
    pub fn insert_gate_rejection(
        &self,
        reason: &str,
        kind: &str,
        confidence: f64,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let now = now_secs() as i64;
        self.conn.execute(
            "INSERT INTO gate_rejections (reason, kind, confidence, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![reason, kind, confidence, payload.to_string(), now],
        )?;
        Ok(())
    }

    /// List gate rejections, optionally filtered by reason. Newest first.
    pub fn list_gate_rejections(
        &self,
        reason: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::models::GateRejection>> {
        let sql = match reason {
            Some(r) if !r.is_empty() => "SELECT id, reason, kind, confidence, payload, created_at
                 FROM gate_rejections WHERE reason = ?1
                 ORDER BY created_at DESC LIMIT ?2",
            _ => "SELECT id, reason, kind, confidence, payload, created_at
                 FROM gate_rejections ORDER BY created_at DESC LIMIT ?1",
        };
        let map_row = |row: &rusqlite::Row| {
            let payload_json: String = row.get(4)?;
            let payload = serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
            Ok(crate::models::GateRejection {
                id: row.get(0)?,
                reason: row.get(1)?,
                kind: row.get(2)?,
                confidence: row.get(3)?,
                payload,
                created_at: row.get::<_, i64>(5)? as u64,
            })
        };
        match reason {
            Some(r) if !r.is_empty() => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map(params![r, limit as i64], map_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
            _ => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map(params![limit as i64], map_row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            }
        }
    }

    /// Delete gate rejections older than `days`. Called from the pipeline run
    /// so the table never grows unbounded (SPEC-C1 T4 cleanup).
    pub fn prune_gate_rejections(&self, days: u32) -> Result<usize> {
        let cutoff = now_secs().saturating_sub((days as u64) * 86400) as i64;
        let deleted = self.conn.execute(
            "DELETE FROM gate_rejections WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }
}
