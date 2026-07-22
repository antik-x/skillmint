use super::*;

impl Db {
    /// P2-2: return the local SQLite path and a human-readable data dictionary.
    pub fn export_data_dictionary(&self) -> Result<DataDictionary> {
        let tables = vec![
            TableInfo {
                name: "skills".to_string(),
                description: "中心仓库中的 Skill 元数据".to_string(),
                columns: vec!["id", "name", "repo_path", "created_at", "updated_at", "status"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "collected_sessions".to_string(),
                description: "从各 Agent 采集的会话记录".to_string(),
                columns: vec!["id", "device_id", "source", "project_id", "agent_id", "start_time", "end_time", "message_count", "title_or_prompt", "cached_at"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "skill_versions".to_string(),
                description: "Skill 的历史版本快照".to_string(),
                columns: vec!["id", "device_id", "skill_id", "version", "content", "created_at", "created_by"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "skill_bundles".to_string(),
                description: "用户创建的 Skill 组合".to_string(),
                columns: vec!["id", "device_id", "name", "description", "created_at", "updated_at"].into_iter().map(String::from).collect(),
            },
            TableInfo {
                name: "projects".to_string(),
                description: "从会话中识别的项目".to_string(),
                columns: vec!["id", "device_id", "name", "path", "first_seen_at", "last_active_at"].into_iter().map(String::from).collect(),
            },
        ];
        Ok(DataDictionary {
            db_path: self.path.to_string_lossy().to_string(),
            tables,
        })
    }

    /// P2-2: export the requested tables as a single JSON object.
    pub fn export_raw_data(&self, tables: &[String]) -> Result<RawDataExport> {
        let allowed: std::collections::HashSet<&str> = [
            "skills", "agents", "sync_targets", "collected_sessions", "collected_prompts",
            "skill_versions", "skill_bundles", "skill_bundle_items", "projects",
            "skill_project_bindings", "llm_request_logs", "collection_jobs",
        ]
        .iter()
        .cloned()
        .collect();

        let mut data = serde_json::Map::new();
        for table in tables {
            if !allowed.contains(table.as_str()) {
                anyhow::bail!("unsupported table: {table}");
            }
            let rows = self.dump_table_as_json(table)?;
            data.insert(table.clone(), serde_json::Value::Array(rows));
        }

        Ok(RawDataExport {
            format: "json".to_string(),
            exported_at: now_secs(),
            tables: tables.to_vec(),
            data: serde_json::Value::Object(data),
        })
    }

    fn dump_table_as_json(&self, table: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(&format!("SELECT * FROM {table}"))?;
        let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let mut rows = Vec::new();
        let mut result = stmt.query([])?;
        while let Some(row) = result.next()? {
            let mut obj = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => serde_json::Value::Number(n.into()),
                    rusqlite::types::ValueRef::Real(n) => serde_json::json!(n),
                    rusqlite::types::ValueRef::Text(s) => serde_json::Value::String(
                        std::str::from_utf8(s).unwrap_or("").to_string()
                    ),
                    rusqlite::types::ValueRef::Blob(b) => serde_json::Value::String(format!("<blob {} bytes>", b.len())),
                };
                obj.insert(col.clone(), value);
            }
            rows.push(serde_json::Value::Object(obj));
        }
        Ok(rows)
    }

    /// SPEC-F3: delete all collected data while preserving skills, agents, settings,
    /// sync targets and project bindings. Runs inside a transaction.
    pub fn clear_collected_data(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let tables = [
            "collected_sources",
            "collected_sessions",
            "collected_prompts",
            "collected_token_usage",
            "collected_code_contributions",
            "collector_file_states",
            "collection_jobs",
            "analysis_window_cache",
            "digest_summary",
            "llm_request_logs",
        ];
        for table in tables {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// SPEC-F3: reset the entire database except for the center repo files on disk.
    /// Drops all user tables and re-creates the schema. Runs inside a transaction.
    pub fn reset_database(&mut self, device_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let tables = [
            "agents",
            "agent_directories",
            "agent_instances",
            "skills",
            "sync_targets",
            "projects",
            "skill_project_bindings",
            "skill_bundles",
            "skill_bundle_items",
            "collected_sources",
            "collected_sessions",
            "collected_prompts",
            "collected_token_usage",
            "collected_code_contributions",
            "collector_file_states",
            "collection_jobs",
            "kg_nodes",
            "kg_edges",
            "kg_skill_nodes",
            "sources",
            "digest_model",
            "digest_tool",
            "digest_summary",
            "analysis_window_cache",
            "discoveries",
            "weekly_reports",
            "scheduled_tasks",
            "task_runs",
            "llm_request_logs",
            "install_audit",
        ];
        for table in tables {
            let _ = tx.execute(&format!("DROP TABLE IF EXISTS {table}"), []);
        }
        tx.commit()?;
        self.init(device_id)?;
        Ok(())
    }
}
