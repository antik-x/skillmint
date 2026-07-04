//! SPEC-I2: capability-gap detector.
//!
//! Uses the built-in keyword table to count prompts per category. If a category
//! has >=5 prompts and no Skill matches that category, report a gap.

use std::collections::{HashMap, HashSet};

use super::{DetectContext, DiscoveryCandidate, DiscoveryKind};
use crate::discovery::{config, keywords};

pub struct CapabilityGapDetector;

impl super::Detector for CapabilityGapDetector {
    fn kind(&self) -> DiscoveryKind {
        DiscoveryKind::CapabilityGap
    }

    fn detect(&self, ctx: &DetectContext) -> anyhow::Result<Vec<DiscoveryCandidate>> {
        // Load all prompts in window.
        let mut stmt = ctx.db.conn_ref().prepare(
            "SELECT prompt_text FROM collected_prompts
             WHERE device_id = ?1
               AND IFNULL(started_at, 0) >= ?2 AND IFNULL(started_at, 0) <= ?3
               AND prompt_text IS NOT NULL AND length(prompt_text) > 0",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![ctx.device_id, ctx.window_start, ctx.window_end],
            |row| row.get::<_, String>(0),
        )?;

        let mut category_counts: HashMap<&'static str, usize> = HashMap::new();
        for text in rows.filter_map(|r| r.ok()) {
            for key in keywords::match_categories(&text) {
                *category_counts.entry(key).or_insert(0) += 1;
            }
        }

        // Load existing skill names + descriptions for category matching.
        let skill_category_hits = skill_category_hits(ctx.db)?;

        let week_key = week_key(ctx.window_end);
        let mut candidates = Vec::new();
        for cat in keywords::categories() {
            let count = category_counts.get(cat.key).copied().unwrap_or(0);
            if count < config::CAPABILITY_GAP_MIN_PROMPTS {
                continue;
            }
            if skill_category_hits.contains(cat.key) {
                continue;
            }

            candidates.push(DiscoveryCandidate {
                kind: DiscoveryKind::CapabilityGap,
                title: format!("能力缺口：{}{} 次", cat.label, count),
                payload: serde_json::json!({
                    "category": cat.key,
                    "label": cat.label,
                    "prompt_count": count,
                }),
                confidence: (count as f64 / 10.0).min(1.0),
                dedup_key: format!("capability_gap:{}:{}", cat.key, week_key),
            });
        }

        Ok(candidates)
    }
}

pub fn skill_category_hits(db: &crate::db::Db) -> anyhow::Result<HashSet<&'static str>> {
    let mut hits = HashSet::new();
    let skills = db.get_skills()?;
    for skill in skills {
        let md_path = skill.repo_path.join("SKILL.md");
        if !md_path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&md_path).unwrap_or_default().to_lowercase();
        for cat in keywords::categories() {
            if cat.keywords.iter().any(|k| text.contains(&k.to_lowercase())) {
                hits.insert(cat.key);
            }
        }
        // Also match against skill name.
        let name_lower = skill.name.to_lowercase();
        for cat in keywords::categories() {
            if cat.keywords.iter().any(|k| name_lower.contains(&k.to_lowercase())) {
                hits.insert(cat.key);
            }
        }
    }
    Ok(hits)
}

fn week_key(epoch_secs: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(epoch_secs, 0).unwrap_or(chrono::DateTime::UNIX_EPOCH);
    dt.format("%Y-W%W").to_string()
}
