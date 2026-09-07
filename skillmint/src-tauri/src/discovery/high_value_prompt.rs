//! SPEC-I2: high-value prompt detector.
//!
//! Reuses the existing heuristic: short sessions with meaningful output and no
//! subsequent correction feel like one-shot successes worth sedimenting.

use sha2::{Digest, Sha256};

use super::{DetectContext, DiscoveryCandidate, DiscoveryKind};
use crate::discovery::config;

pub struct HighValuePromptDetector;

impl super::Detector for HighValuePromptDetector {
    fn kind(&self) -> DiscoveryKind {
        DiscoveryKind::HighValuePrompt
    }

    fn detect(&self, ctx: &DetectContext) -> anyhow::Result<Vec<DiscoveryCandidate>> {
        let mut stmt = ctx.db.conn_ref().prepare(
            r#"SELECT p.id, p.prompt_text, p.session_id,
                      COALESCE(s.message_count, 999),
                      COALESCE(tu.output_tokens, 0),
                      COALESCE(tu.input_tokens, 0),
                      IFNULL(p.prompt_kind, '')
               FROM collected_prompts p
               LEFT JOIN collected_sessions s ON s.id = p.session_id
               LEFT JOIN (
                   SELECT session_id, SUM(output_tokens) AS output_tokens, SUM(input_tokens) AS input_tokens
                   FROM collected_token_usage
                   GROUP BY session_id
               ) tu ON tu.session_id = p.session_id
               WHERE p.device_id = ?1
                 AND IFNULL(p.started_at, 0) >= ?2 AND IFNULL(p.started_at, 0) <= ?3
                 AND p.prompt_text IS NOT NULL AND length(p.prompt_text) >= ?4"#,
        )?;

        let rows = stmt.query_map(
            rusqlite::params![
                ctx.device_id,
                ctx.window_start,
                ctx.window_end,
                config::HIGH_VALUE_MIN_PROMPT_LENGTH as i64,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )?;

        // Exclude prompts already captured by repeat-pattern clusters.
        let repeat_dedups: std::collections::HashSet<String> =
            crate::discovery::repeat_pattern::detect_repeat_dedups(ctx)?;

        let mut candidates = Vec::new();
        for row in rows.filter_map(|r| r.ok()) {
            let (id, text, session_id, message_count, output_tokens, input_tokens, kind) = row;
            // E2-S2.1.4：非用户输入（系统提醒/Skill 脚手架）不构成高价值 Prompt。
            if !crate::prompt_kind::is_user_row(&kind, &text) {
                continue;
            }
            let normalized = crate::discovery::repeat_pattern::normalize(&text);
            if normalized.len() < config::HIGH_VALUE_MIN_PROMPT_LENGTH {
                continue;
            }

            let dedup = high_value_dedup_key(&text);
            if repeat_dedups.contains(&dedup) {
                continue;
            }

            let efficiency = if input_tokens > 0 {
                output_tokens as f64 / input_tokens as f64
            } else {
                0.0
            };

            let mut score = 0.0f64;
            if message_count <= config::HIGH_VALUE_MAX_MESSAGES {
                score += 0.3;
            }
            if efficiency >= config::HIGH_VALUE_MIN_TOKEN_EFFICIENCY {
                score += 0.3;
            }
            if output_tokens > 50 {
                score += 0.2;
            }
            // No correction signal currently available in prompt row; treat as clean.
            score += 0.2;

            if score < config::HIGH_VALUE_ONE_SHOT_CONFIDENCE {
                continue;
            }

            candidates.push(DiscoveryCandidate {
                kind: DiscoveryKind::HighValuePrompt,
                title: format!("高价值 Prompt：{}", truncate(&text, 80)),
                payload: serde_json::json!({
                    "prompt_id": id,
                    "session_id": session_id,
                    "prompt_text": text,
                    "token_efficiency": efficiency,
                    "message_count": message_count,
                }),
                confidence: score.min(1.0),
                dedup_key: dedup,
            });
        }

        Ok(candidates)
    }
}

pub fn high_value_dedup_key(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    format!("hvp:{}", hex::encode(&hash[..8]))
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect::<String>() + "…"
    }
}
