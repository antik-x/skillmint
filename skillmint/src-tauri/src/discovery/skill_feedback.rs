//! SPEC-I2: skill-effect feedback detector.
//!
//! Compares sessions that used a specific Skill vs sessions that didn't, within
//! the same project scope when possible.

use super::{DetectContext, DiscoveryCandidate, DiscoveryKind};
use crate::discovery::config;

pub struct SkillFeedbackDetector;

impl super::Detector for SkillFeedbackDetector {
    fn kind(&self) -> DiscoveryKind {
        DiscoveryKind::SkillFeedback
    }

    fn detect(&self, ctx: &DetectContext) -> anyhow::Result<Vec<DiscoveryCandidate>> {
        // Find all skills used in the window.
        let mut stmt = ctx.db.conn_ref().prepare(
            r#"SELECT DISTINCT skill_id, skill_name
               FROM skill_usage_attributions
               WHERE device_id = ?1
                 AND IFNULL(attributed_at, 0) >= ?2 AND IFNULL(attributed_at, 0) <= ?3
                 AND skill_id IS NOT NULL"#,
        )?;
        let rows = stmt.query_map(
            rusqlite::params![ctx.device_id, ctx.window_start, ctx.window_end],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let skills: Vec<(String, String)> = rows.filter_map(|r| r.ok()).collect();

        let mut candidates = Vec::new();
        let month_key = month_key(ctx.window_end);

        for (skill_id, skill_name) in skills {
            let (with_sessions, without_sessions) = session_samples(ctx, &skill_id)?;
            if with_sessions.len() < config::SKILL_FEEDBACK_MIN_SAMPLES
                || without_sessions.len() < config::SKILL_FEEDBACK_MIN_SAMPLES
            {
                continue;
            }

            let with_avg = avg_messages(&with_sessions);
            let without_avg = avg_messages(&without_sessions);
            if without_avg == 0.0 {
                continue;
            }

            let diff = (without_avg - with_avg).abs() / without_avg;
            if diff < config::SKILL_FEEDBACK_DIFF_THRESHOLD {
                continue;
            }

            let positive = with_avg < without_avg;
            let title = if positive {
                format!("Skill「{}」显著提升了效率", skill_name)
            } else {
                format!("Skill「{}」可能需要改进", skill_name)
            };

            candidates.push(DiscoveryCandidate {
                kind: DiscoveryKind::SkillFeedback,
                title,
                payload: serde_json::json!({
                    "skill_id": skill_id,
                    "skill_name": skill_name,
                    "with_avg_messages": with_avg,
                    "without_avg_messages": without_avg,
                    "with_samples": with_sessions.len(),
                    "without_samples": without_sessions.len(),
                }),
                confidence: diff.min(1.0),
                dedup_key: format!("skill_feedback:{}:{}", skill_id, month_key),
            });
        }

        Ok(candidates)
    }
}

fn session_samples(ctx: &DetectContext, skill_id: &str) -> anyhow::Result<(Vec<i64>, Vec<i64>)> {
    // Sessions with the skill.
    let mut stmt = ctx.db.conn_ref().prepare(
        r#"SELECT DISTINCT s.id, s.message_count
           FROM collected_sessions s
           JOIN skill_usage_attributions a
             ON a.device_id = s.device_id AND a.session_id = s.id
           WHERE s.device_id = ?1
             AND IFNULL(s.start_time, 0) >= ?2 AND IFNULL(s.start_time, 0) <= ?3
             AND IFNULL(s.origin, 'user') != 'skillmint_acp'
             AND a.skill_id = ?4"#,
    )?;
    let rows = stmt.query_map(
        rusqlite::params![ctx.device_id, ctx.window_start, ctx.window_end, skill_id],
        |row| Ok(row.get::<_, i64>(1).unwrap_or(0)),
    )?;
    let with_sessions: Vec<i64> = rows.filter_map(|r| r.ok()).collect();

    // Sessions without the skill in the same window.
    let mut stmt = ctx.db.conn_ref().prepare(
        r#"SELECT s.message_count
           FROM collected_sessions s
           WHERE s.device_id = ?1
             AND IFNULL(s.start_time, 0) >= ?2 AND IFNULL(s.start_time, 0) <= ?3
             AND IFNULL(s.origin, 'user') != 'skillmint_acp'
             AND NOT EXISTS (
                 SELECT 1 FROM skill_usage_attributions a
                 WHERE a.device_id = s.device_id AND a.session_id = s.id AND a.skill_id = ?4
             )"#,
    )?;
    let rows = stmt.query_map(
        rusqlite::params![ctx.device_id, ctx.window_start, ctx.window_end, skill_id],
        |row| Ok(row.get::<_, i64>(0).unwrap_or(0)),
    )?;
    let without_sessions: Vec<i64> = rows.filter_map(|r| r.ok()).collect();

    Ok((with_sessions, without_sessions))
}

fn avg_messages(values: &[i64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<i64>() as f64 / values.len() as f64
}

fn month_key(epoch_secs: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(epoch_secs, 0).unwrap_or(chrono::DateTime::UNIX_EPOCH);
    dt.format("%Y-%m").to_string()
}
