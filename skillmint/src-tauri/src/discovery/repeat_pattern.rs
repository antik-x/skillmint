//! SPEC-I2: repeat-pattern detector.
//!
//! Normalizes prompt text, clusters by 3-gram Jaccard similarity, and reports
//! clusters with >=3 occurrences in the window.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use super::{DetectContext, DiscoveryCandidate, DiscoveryKind};
use crate::discovery::config;

pub struct RepeatPatternDetector;

impl super::Detector for RepeatPatternDetector {
    fn kind(&self) -> DiscoveryKind {
        DiscoveryKind::RepeatPattern
    }

    fn detect(&self, ctx: &DetectContext) -> anyhow::Result<Vec<DiscoveryCandidate>> {
        let prompts = load_prompts(ctx)?;
        if prompts.len() < config::REPEAT_MIN_PROMPTS {
            return Ok(Vec::new());
        }

        // (id, entry, normalized)
        let normalized: Vec<(String, PromptEntry, String)> = prompts
            .into_iter()
            .map(|(id, text, ts, source)| {
                let normalized_text = normalize(&text);
                let entry = PromptEntry {
                    id,
                    normalized: normalized_text.clone(),
                    started_at: ts,
                    raw: text,
                    source,
                };
                let id = entry.id.clone();
                (id, entry, normalized_text)
            })
            .filter(|(_, e, _)| !e.normalized.is_empty())
            .collect();

        let mut clusters: Vec<Vec<PromptEntry>> = Vec::new();
        for (_, entry, norm) in normalized {
            let mut placed = false;
            for cluster in &mut clusters {
                let representative = &cluster[0].normalized;
                if jaccard_3gram(representative, &norm) >= config::REPEAT_JACCARD_THRESHOLD {
                    cluster.push(entry.clone());
                    placed = true;
                    break;
                }
            }
            if !placed {
                clusters.push(vec![entry]);
            }
        }

        let mut candidates = Vec::new();
        for cluster in clusters {
            if cluster.len() >= config::REPEAT_MIN_PROMPTS {
                // P6：标题用原文首行（normalize 会把多行压平、代码/路径替换成占位符，
                // 观感是"拼接的一坨"——真机反馈 2026-09-07）；归一化文本只用于去重键。
                let earliest = cluster.iter().min_by_key(|e| e.started_at).cloned().unwrap_or_else(|| cluster[0].clone());
                let title_text = truncate(&first_line(&earliest.raw), 80);
                let dedup = repeat_dedup_key(&earliest.normalized);
                let confidence = (cluster.len() as f64 / config::REPEAT_CONFIDENCE_DENOMINATOR).min(1.0);
                let prompt_ids: Vec<String> = cluster.iter().map(|e| e.id.clone()).collect();

                // P6：证据原文（最新 3 条），Inbox 卡默认展示前 2 条——
                // 此前后端从不写 payload.evidence，前端证据区恒为空（契约缺口）。
                let mut latest: Vec<&PromptEntry> = cluster.iter().collect();
                latest.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                let evidence: Vec<serde_json::Value> = latest
                    .into_iter()
                    .take(3)
                    .map(|e| {
                        serde_json::json!({
                            "prompt_text": truncate(&first_line(&e.raw), 200),
                            "started_at": e.started_at,
                            "source": e.source,
                            "session_id": e.id,
                        })
                    })
                    .collect();

                candidates.push(DiscoveryCandidate {
                    kind: DiscoveryKind::RepeatPattern,
                    title: format!("重复模式：{}", title_text),
                    payload: serde_json::json!({
                        "pattern": truncate(&earliest.normalized, 120),
                        "prompt_ids": prompt_ids,
                        "count": cluster.len(),
                        "evidence": evidence,
                    }),
                    confidence,
                    dedup_key: dedup,
                });
            }
        }

        Ok(candidates)
    }
}

/// 原文首行（trim 后）；空则回退全文。
fn first_line(raw: &str) -> String {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(raw);
    line.to_string()
}

#[derive(Clone)]
struct PromptEntry {
    id: String,
    normalized: String,
    started_at: i64,
    /// 原文（标题/证据用）。
    raw: String,
    /// 来源工具（证据行展示用）。
    source: String,
}

fn load_prompts(ctx: &DetectContext) -> anyhow::Result<Vec<(String, String, i64, String)>> {
    let mut stmt = ctx.db.conn_ref().prepare(
        "SELECT id, prompt_text, IFNULL(started_at, 0), IFNULL(prompt_kind, ''), IFNULL(source, '')
         FROM collected_prompts
         WHERE device_id = ?1 AND IFNULL(started_at, 0) >= ?2 AND IFNULL(started_at, 0) <= ?3
           AND prompt_text IS NOT NULL AND length(prompt_text) > 0
           AND IFNULL(origin, 'user') != 'skillmint_acp'",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![ctx.device_id, ctx.window_start, ctx.window_end],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    // E2-S2.1.4：只统计用户真实输入。已打标行按标记过滤；老数据（无标记）按
    // 规则兜底再判一次，确保历史脏数据（系统提醒等）也被排除。
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|(_, text, _, kind, _)| crate::prompt_kind::is_user_row(kind, text))
        .map(|(id, text, at, _, source)| (id, text, at, source))
        .collect())
}

/// Lowercase, collapse whitespace, replace code blocks and paths with placeholders.
pub fn normalize(text: &str) -> String {
    let mut s = text.to_lowercase();
    s = replace_fenced_code(&s, " {code} ");
    s = replace_inline_code(&s, " {code} ");
    s = replace_paths(&s, " {path} ");
    // Keep only alphanumerics and whitespace.
    s = s.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' }).collect();
    // Collapse whitespace.
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn replace_fenced_code(text: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_block = !in_block;
            if !in_block {
                out.push_str(replacement);
            }
            continue;
        }
        if in_block {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn replace_inline_code(text: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let chars = text.chars().peekable();
    for c in chars {
        if c == '`' {
            in_code = !in_code;
            if !in_code {
                out.push_str(replacement);
            }
            continue;
        }
        if !in_code {
            out.push(c);
        }
    }
    out
}

fn replace_paths(text: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    for c in text.chars() {
        if is_path_char(c) {
            buf.push(c);
        } else {
            if looks_like_path(&buf) {
                out.push_str(replacement);
            } else {
                out.push_str(&buf);
            }
            buf.clear();
            out.push(c);
        }
    }
    if looks_like_path(&buf) {
        out.push_str(replacement);
    } else {
        out.push_str(&buf);
    }
    out
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || c == '/' || c == '\\' || c == '.' || c == '-' || c == '_' || c == '~'
}

fn looks_like_path(s: &str) -> bool {
    s.len() > 2 && (s.starts_with('/') || s.starts_with("~/") || s.starts_with("./") || s.starts_with("../") || s.starts_with('\\'))
}

fn jaccard_3gram(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_grams = ngrams(a, 3);
    let b_grams = ngrams(b, 3);
    if a_grams.is_empty() || b_grams.is_empty() {
        return 0.0;
    }
    let a_set: HashSet<&str> = a_grams.iter().map(|s| s.as_str()).collect();
    let b_set: HashSet<&str> = b_grams.iter().map(|s| s.as_str()).collect();
    let intersection: HashSet<&&str> = a_set.intersection(&b_set).collect();
    let union = a_set.len() + b_set.len() - intersection.len();
    if union == 0 {
        return 0.0;
    }
    intersection.len() as f64 / union as f64
}

fn ngrams(text: &str, n: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < n {
        return Vec::new();
    }
    words
        .windows(n)
        .map(|w| w.join(" "))
        .collect()
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect::<String>() + "…"
    }
}

pub fn repeat_dedup_key(normalized_text: &str) -> String {
    let hash = Sha256::digest(normalized_text.as_bytes());
    format!("repeat:{}", hex::encode(&hash[..8]))
}

/// Helper for the high-value detector: compute dedup keys of all repeat clusters
/// in the window so those prompts can be excluded.
pub fn detect_repeat_dedups(ctx: &DetectContext) -> anyhow::Result<std::collections::HashSet<String>> {
    let detector = RepeatPatternDetector;
    use crate::discovery::Detector;
    let candidates = detector.detect(ctx)?;
    Ok(candidates.into_iter().map(|c| c.dedup_key).collect())
}
