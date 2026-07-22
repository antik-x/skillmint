use super::*;

impl Db {
    // ----- PRD-07: remote sources -------------------------------------------

    pub fn insert_source(&self, s: &Source) -> Result<()> {
        self.conn.execute(
            r#"INSERT OR REPLACE INTO sources
               (id, name, source_type, url, ref_spec, subpath, cache_path,
                commit_sha, added_at, last_fetched_at, pull_policy, remote_revision)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                s.id,
                s.name,
                s.source_type.to_string(),
                s.url,
                s.ref_spec,
                s.subpath,
                s.cache_path,
                s.commit_sha,
                s.added_at,
                s.last_fetched_at,
                s.pull_policy,
                s.remote_revision,
            ],
        )?;
        Ok(())
    }

    pub fn get_sources(&self) -> Result<Vec<Source>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, name, source_type, url, ref_spec, subpath, cache_path,
                      commit_sha, added_at, last_fetched_at, pull_policy, remote_revision
               FROM sources ORDER BY name"#,
        )?;
        let rows = stmt.query_map([], map_source_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_source_by_id(&self, id: &str) -> Result<Option<Source>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, name, source_type, url, ref_spec, subpath, cache_path,
                      commit_sha, added_at, last_fetched_at, pull_policy, remote_revision
               FROM sources WHERE id = ?1"#,
        )?;
        let mut rows = stmt.query_map(params![id], map_source_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn delete_source(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sources WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// P2-3: seed a built-in official example source when no remote/local sources exist.
    pub fn ensure_official_example_source(
        &self,
        center_repo: &std::path::Path,
    ) -> Result<Option<Source>> {
        let existing: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
        if existing > 0 {
            return Ok(None);
        }

        let examples_dir = center_repo.join("examples");
        let sample_skill_dir = examples_dir.join("hello-skillmint");
        let skill_md = sample_skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            std::fs::create_dir_all(&sample_skill_dir)?;
            std::fs::write(
                &skill_md,
                r#"---
name: hello-skillmint
description: SkillMint 官方示例 Skill，演示 frontmatter 格式。
author: SkillMint
---

# hello-skillmint

这是一个官方示例 Skill。

## Usage

在任意项目中让 Agent 使用本 Skill，观察 SkillMint 如何把它同步到本地 Agent 目录。
"#,
            )?;
        }

        let source = Source {
            id: new_id(),
            source_type: SourceType::Local,
            name: "SkillMint 官方示例".to_string(),
            url: examples_dir.to_string_lossy().to_string(),
            ref_spec: String::new(),
            subpath: String::new(),
            cache_path: String::new(),
            commit_sha: String::new(),
            added_at: now_secs(),
            last_fetched_at: None,
            pull_policy: "manual".to_string(),
            remote_revision: String::new(),
        };
        self.insert_source(&source)?;
        Ok(Some(source))
    }
}
