//! P3-3: storage for the rebuildable skill index (`skill_index` +
//! `skill_index_meta`). The DB is a disposable read model — `replace_skill_index`
//! swaps the whole per-project-key set in one transaction and computes the
//! `modified` status by diffing content hashes against the previous scan.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::index::SkillIndexEntry;

/// One row as returned to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillIndexRow {
    pub name: String,
    pub scope: String,
    pub managed_by: String,
    pub path: String,
    pub skill_md_path: String,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub source_type: Option<String>,
    pub ref_spec: Option<String>,
    pub hash: String,
    pub content_hash: String,
    pub status: String,
    pub agents: Vec<String>,
    pub description: Option<String>,
    pub updated_at: i64,
}

fn row_from_entry(e: &SkillIndexEntry, content_hash: &str, status: &str) -> SkillIndexRow {
    SkillIndexRow {
        name: e.name.clone(),
        scope: e.scope.clone(),
        managed_by: e.managed_by.clone(),
        path: e.path.clone(),
        skill_md_path: e.skill_md_path.clone(),
        source: e.source.clone(),
        source_url: e.source_url.clone(),
        source_type: e.source_type.clone(),
        ref_spec: e.ref_spec.clone(),
        hash: e.hash.clone(),
        content_hash: content_hash.to_string(),
        status: status.to_string(),
        agents: e.agents.clone(),
        description: e.description.clone(),
        updated_at: chrono::Utc::now().timestamp_millis(),
    }
}

impl super::Db {
    /// Replace the whole index for `project_key`. Returns how many entries came
    /// out `modified` (npx-managed skills whose on-disk content changed since
    /// the previous scan — the design's "已被本地修改" status).
    pub fn replace_skill_index(&self, project_key: &str, entries: &[SkillIndexEntry]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;

        let mut prev: BTreeMap<(String, String, String), String> = BTreeMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT scope, name, path, content_hash FROM skill_index WHERE project_key = ?1",
            )?;
            let rows = stmt.query_map(params![project_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })?;
            for r in rows {
                let (scope, name, path, hash) = r?;
                prev.insert((scope, name, path), hash);
            }
        }

        tx.execute("DELETE FROM skill_index WHERE project_key = ?1", params![project_key])?;
        let mut modified = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO skill_index
                 (project_key, scope, name, managed_by, path, skill_md_path, source, source_url,
                  source_type, ref_spec, hash, content_hash, prev_content_hash, status,
                  agents_json, description, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            )?;
            for e in entries {
                let content = crate::index::content_hash(Path::new(&e.path));
                let key = (e.scope.clone(), e.name.clone(), e.path.clone());
                let prev_hash = prev.get(&key).cloned().unwrap_or_default();
                let status = if e.status == "broken" {
                    "broken".to_string()
                } else if e.managed_by == "npx"
                    && !prev_hash.is_empty()
                    && prev_hash != content
                {
                    "modified".to_string()
                } else {
                    "ok".to_string()
                };
                if status == "modified" {
                    modified += 1;
                }
                let row = row_from_entry(e, &content, &status);
                stmt.execute(params![
                    project_key,
                    row.scope,
                    row.name,
                    row.managed_by,
                    row.path,
                    row.skill_md_path,
                    row.source,
                    row.source_url,
                    row.source_type,
                    row.ref_spec,
                    row.hash,
                    row.content_hash,
                    prev_hash,
                    row.status,
                    serde_json::to_string(&row.agents)?,
                    row.description,
                    row.updated_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(modified)
    }

    /// List the index for a project key, optionally filtered to scopes
    /// (e.g. `["global", "hub-global"]`).
    pub fn list_skill_index(&self, project_key: &str, scopes: Option<&[String]>) -> Result<Vec<SkillIndexRow>> {
        let mut sql = String::from(
            "SELECT scope, name, managed_by, path, skill_md_path, source, source_url, source_type,
                    ref_spec, hash, content_hash, status, agents_json, description, updated_at
             FROM skill_index WHERE project_key = ?1",
        );
        if let Some(ss) = scopes {
            if !ss.is_empty() {
                let placeholders = ss.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                sql.push_str(&format!(" AND scope IN ({placeholders})"));
            }
        }
        sql.push_str(" ORDER BY scope, name");
        let mut stmt = self.conn.prepare(&sql)?;
        let bind = |stmt: &mut rusqlite::Statement<'_>| -> rusqlite::Result<Vec<SkillIndexRow>> {
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(project_key.to_string())];
            if let Some(ss) = scopes {
                for s in ss {
                    params_vec.push(Box::new(s.clone()));
                }
            }
            let pref = rusqlite::params_from_iter(params_vec.iter().map(|b| b.as_ref()));
            let rows = stmt.query_map(pref, |row| {
                let agents_json: String = row.get(12)?;
                Ok(SkillIndexRow {
                    scope: row.get(0)?,
                    name: row.get(1)?,
                    managed_by: row.get(2)?,
                    path: row.get(3)?,
                    skill_md_path: row.get(4)?,
                    source: row.get(5)?,
                    source_url: row.get(6)?,
                    source_type: row.get(7)?,
                    ref_spec: row.get(8)?,
                    hash: row.get(9)?,
                    content_hash: row.get(10)?,
                    status: row.get(11)?,
                    agents: serde_json::from_str(&agents_json).unwrap_or_default(),
                    description: row.get(13)?,
                    updated_at: row.get(14)?,
                })
            })?;
            rows.collect()
        };
        bind(&mut stmt).map_err(Into::into)
    }

    pub fn get_index_meta(&self, project_key: &str) -> Result<Option<(String, i64)>> {
        self.conn
            .query_row(
                "SELECT fingerprint, rebuilt_at FROM skill_index_meta WHERE project_key = ?1",
                params![project_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_index_meta(&self, project_key: &str, fingerprint: &str, rebuilt_at: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_index_meta (project_key, fingerprint, rebuilt_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(project_key) DO UPDATE SET fingerprint = ?2, rebuilt_at = ?3",
            params![project_key, fingerprint, rebuilt_at as i64],
        )?;
        Ok(())
    }
}
