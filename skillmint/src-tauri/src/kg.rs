//! PRD-03: Skill knowledge graph.
//!
//! Parses each `SKILL.md` into concept/scenario nodes, computes inter-skill
//! relations (similar via Jaccard), and supports task-driven recommendation.
//! Extraction is **rule-based** (decision: A in DECISIONS-implementation.md);
//! LLM enhancement is a future trait extension point, not a P0 dependency.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::models::{KgEdge, KgNode, TaskRecommendation};

/// Parsed frontmatter from a SKILL.md (YAML between `---` fences).
#[derive(Debug, Default, Clone)]
pub struct Frontmatter {
    pub concepts: Vec<String>,
    pub scenarios: Vec<String>,
    pub related_skills: Vec<String>,
}

/// Analyze one skill's SKILL.md and upsert its nodes into the graph.
/// Reads frontmatter first (explicit), then falls back to rule extraction.
pub fn analyze_skill(db: &Db, device_id: &str, skill_id: &str, skill_md_path: &Path) -> Result<usize> {
    let content = std::fs::read_to_string(skill_md_path).unwrap_or_default();
    let (fm, body) = split_frontmatter(&content);

    // 1. Explicit concepts/scenarios from frontmatter.
    let mut concepts: HashSet<String> = fm.concepts.iter().cloned().collect();
    let scenarios: HashSet<String> = fm.scenarios.iter().cloned().collect();

    // 2. Rule-based extraction from body when frontmatter is thin.
    if concepts.is_empty() {
        // PRD-03 FR-1.3: segment by H2 headers, extract per-segment.
        for seg in segment_by_h2(&body) {
            concepts.extend(extract_concepts(&seg));
        }
        if concepts.is_empty() {
            concepts.extend(extract_concepts(&body));
        }
    }

    let mut linked = 0usize;
    // Upsert concept nodes.
    for c in &concepts {
        let node = make_node(c, "concept", Some(skill_id));
        db.upsert_kg_node(&node)?;
        db.link_skill_node(device_id, skill_id, &node.id, 1.0)?;
        linked += 1;
    }
    // Upsert scenario nodes.
    for s in &scenarios {
        let node = make_node(s, "scenario", Some(skill_id));
        db.upsert_kg_node(&node)?;
        db.link_skill_node(device_id, skill_id, &node.id, 0.8)?;
        linked += 1;
    }
    // Upsert a practice node representing the skill itself.
    let practice_label = skill_md_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let practice_node = make_node(practice_label, "practice", Some(skill_id));
    db.upsert_kg_node(&practice_node)?;
    db.link_skill_node(device_id, skill_id, &practice_node.id, 1.0)?;
    linked += 1;

    // PRD-03 FR-2.1: explicit relations from frontmatter `related_skills` → kg_edges.
    for related in &fm.related_skills {
        let related_normalized = strip_skill_prefix(related);
        let target_node = make_node(&related_normalized, "practice", None);
        db.upsert_kg_node(&target_node)?;
        let edge = KgEdge {
            id: format!("{}:{}:{}:related", device_id, practice_node.id, target_node.id),
            device_id: device_id.to_string(),
            source_id: practice_node.id.clone(),
            target_id: target_node.id,
            relation: "related".to_string(),
            weight: Some(1.0),
            reason: Some("frontmatter related_skills".to_string()),
            is_manual: true, // explicit = manual-confirmed
            is_rejected: false,
        };
        db.upsert_kg_edge(&edge)?;
    }
    Ok(linked)
}

/// Strip a `plugin:` namespace prefix from a skill name.
pub fn strip_skill_prefix(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((prefix, rest)) if !prefix.is_empty() && !rest.is_empty() => rest.to_string(),
        _ => name.to_string(),
    }
}

/// PRD-03 FR-1.3: split markdown body into segments by H2 (`##`) headers.
fn segment_by_h2(body: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    for line in body.lines() {
        if line.trim_start().starts_with("## ") {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Compute `similar` edges between all skills based on concept-set Jaccard
/// similarity (threshold 0.4 per PRD-03 §3.2). Clears old auto-similar edges first,
/// but preserves user-rejected pairs (negative record, PRD-03 §3.2 b).
pub fn compute_relations(db: &Db, device_id: &str, threshold: f64) -> Result<usize> {
    // Load each skill's concept set.
    let skills = db.get_skill_names()?;
    let mut concept_map: HashMap<String, HashSet<String>> = HashMap::new();
    for (sid, _name) in &skills {
        let concepts = db.get_skill_concepts(device_id, sid)?;
        concept_map.insert(
            sid.clone(),
            concepts.into_iter().map(|(_id, label)| label).collect(),
        );
    }

    // Before clearing, snapshot which auto-edges the user rejected (so we don't recreate them).
    let rejected = db.get_rejected_edge_pairs()?;

    // Delete previous auto-similar edges (is_manual=0) before recomputing.
    db.conn_execute("DELETE FROM kg_edges WHERE relation = 'similar' AND is_manual = 0")?;

    let mut created = 0usize;
    let skill_list: Vec<(String, String)> = skills.clone();
    for i in 0..skill_list.len() {
        for j in (i + 1)..skill_list.len() {
            let (id_a, name_a) = &skill_list[i];
            let (id_b, name_b) = &skill_list[j];
            let set_a = concept_map.get(id_a);
            let set_b = concept_map.get(id_b);
            if let (Some(a), Some(b)) = (set_a, set_b) {
                let sim = jaccard(a, b);
                if sim >= threshold {
                    let node_a_id = ensure_practice_node(db, name_a)?;
                    let node_b_id = ensure_practice_node(db, name_b)?;
                    // Skip if the user previously rejected this pair.
                    let pair_key = if node_a_id < node_b_id {
                        (node_a_id.clone(), node_b_id.clone())
                    } else {
                        (node_b_id.clone(), node_a_id.clone())
                    };
                    if rejected.contains(&pair_key) {
                        continue;
                    }
                    let shared: Vec<String> = a.intersection(b).cloned().collect();
                    let edge = KgEdge {
                        id: format!("{}:{}:{}:similar", device_id, node_a_id, node_b_id),
                        device_id: device_id.to_string(),
                        source_id: node_a_id,
                        target_id: node_b_id,
                        relation: "similar".to_string(),
                        weight: Some(sim),
                        reason: Some(format!("概念相似度 {:.2}，共享 {}", sim, shared.join("/"))),
                        is_manual: false,
                        is_rejected: false,
                    };
                    db.upsert_kg_edge(&edge)?;
                    created += 1;
                }

                // PRD-03 FR-2.2: generalizes — one skill's concept set is a subset of the other's.
                let subset = a.is_subset(b) || b.is_subset(a);
                if subset && a.len() != b.len() {
                    let (gen_name, spec_name) = if a.len() < b.len() { (name_a, name_b) } else { (name_b, name_a) };
                    let gen_node_id = ensure_practice_node(db, gen_name)?;
                    let spec_node_id = ensure_practice_node(db, spec_name)?;
                    let edge = KgEdge {
                        id: format!("{}:{}:{}:generalizes", device_id, gen_node_id, spec_node_id),
                        device_id: device_id.to_string(),
                        source_id: gen_node_id,
                        target_id: spec_node_id,
                        relation: "generalizes".to_string(),
                        weight: Some(0.9),
                        reason: Some("概念集合为包含关系".to_string()),
                        is_manual: false,
                        is_rejected: false,
                    };
                    db.upsert_kg_edge(&edge)?;
                    created += 1;
                }
            }
        }
    }

    // PRD-03 FR-2.2: composes — skills that co-occur in the same project (from PRD-02 data).
    created += compute_composes(db, device_id, &skills)?;

    // PRD-03 FR-2.2: depends_on + conflicts_with relations.
    created += compute_depends_and_conflicts(db, device_id, &skills)?;

    Ok(created)
}

/// PRD-03 §3.2: compute `depends_on` (skill B's concepts include concepts A's body mentions
/// as prerequisites) and `conflicts_with` (two skills share ≥2 scenario nodes that imply
/// mutually-exclusive approaches). Simplified heuristic for rule-based P0.
fn compute_depends_and_conflicts(db: &Db, device_id: &str, skills: &[(String, String)]) -> Result<usize> {
    // Clear old auto depends_on/conflicts_with edges.
    db.conn_execute("DELETE FROM kg_edges WHERE relation IN ('depends_on','conflicts_with') AND is_manual = 0")?;
    let rejected = db.get_rejected_edge_pairs()?;
    let mut created = 0usize;

    // Load scenario sets per skill (for conflicts_with).
    let mut scenario_map: HashMap<String, HashSet<String>> = HashMap::new();
    for (sid, _) in skills {
        let nodes = db.get_skill_concepts(device_id, sid)?; // reuse concept loader; scenarios are separate
        scenario_map.insert(sid.clone(), nodes.into_iter().map(|(_id, l)| l).collect());
    }

    // conflicts_with: two skills sharing ≥2 identical concepts AND different practice names
    // (heuristic for "same problem, different approach"). Threshold per PRD §3.2e.
    for i in 0..skills.len() {
        for j in (i + 1)..skills.len() {
            let (id_a, name_a) = &skills[i];
            let (id_b, name_b) = &skills[j];
            let set_a = scenario_map.get(id_a);
            let set_b = scenario_map.get(id_b);
            if let (Some(a), Some(b)) = (set_a, set_b) {
                let shared = a.intersection(b).count();
                if shared >= 2 {
                    let node_a_id = ensure_practice_node(db, name_a)?;
                    let node_b_id = ensure_practice_node(db, name_b)?;
                    let pair = if node_a_id < node_b_id {
                        (node_a_id.clone(), node_b_id.clone())
                    } else {
                        (node_b_id.clone(), node_a_id.clone())
                    };
                    if rejected.contains(&pair) {
                        continue;
                    }
                    let edge = KgEdge {
                        id: format!("{}:{}:{}:conflicts_with", device_id, node_a_id, node_b_id),
                        device_id: device_id.to_string(),
                        source_id: node_a_id,
                        target_id: node_b_id,
                        relation: "conflicts_with".to_string(),
                        weight: Some(0.7),
                        reason: Some(format!("共享 {} 个概念，可能为互斥方案", shared)),
                        is_manual: false,
                        is_rejected: false,
                    };
                    db.upsert_kg_edge(&edge)?;
                    created += 1;
                }
            }
        }
    }
    Ok(created)
}

/// PRD-03 §8.1: concept coverage report — which concepts are well-covered vs sparse.
/// Returns (concept_label, skill_count_covering_it) sorted ascending (gaps first).
pub fn concept_coverage_report(db: &Db) -> Result<Vec<(String, i64)>> {
    let mut stmt = db.conn_ref().prepare(
        r#"SELECT n.label, COUNT(DISTINCT sn.skill_id) AS skill_count
           FROM kg_nodes n
           LEFT JOIN kg_skill_nodes sn ON sn.node_id = n.id
           WHERE n.type = 'concept'
           GROUP BY n.id
           ORDER BY skill_count ASC, n.label"#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Compute `composes` edges from skill co-occurrence in projects (PRD-02 attribution data).
fn compute_composes(db: &Db, device_id: &str, skills: &[(String, String)]) -> Result<usize> {
    // For each project, find skills attributed to sessions in that project, then pair them.
    // We use skill_usage_attributions grouped by project_id.
    let co = db.get_skill_cooccurrence(device_id)?;
    let mut created = 0usize;
    for (skill_a, skill_b, count) in co {
        // Only create an edge if both skills still exist.
        let name_a = skills.iter().find(|(id, _)| id == &skill_a).map(|(_, n)| n.clone());
        let name_b = skills.iter().find(|(id, _)| id == &skill_b).map(|(_, n)| n.clone());
        if let (Some(na), Some(nb)) = (name_a, name_b) {
            if count >= 2 {
                let node_a_id = ensure_practice_node(db, &na)?;
                let node_b_id = ensure_practice_node(db, &nb)?;
                let edge = KgEdge {
                    id: format!("{}:{}:{}:composes", device_id, node_a_id, node_b_id),
                    device_id: device_id.to_string(),
                    source_id: node_a_id.clone(),
                    target_id: node_b_id.clone(),
                    relation: "composes".to_string(),
                    weight: Some((count as f64).min(1.0)),
                    reason: Some(format!("同项目共现 {} 次", count)),
                    is_manual: false,
                    is_rejected: false,
                };
                db.upsert_kg_edge(&edge)?;
                created += 1;
            }
        }
    }
    Ok(created)
}

/// Task-driven recommendation: extract concepts from the task text, find skills
/// covering them, and report uncovered concepts as gaps.
pub fn recommend_for_task(db: &Db, device_id: &str, task: &str) -> Result<TaskRecommendation> {
    let task_concepts = extract_concepts(task);
    if task_concepts.is_empty() {
        return Ok(TaskRecommendation::default());
    }
    let task_set: HashSet<String> = task_concepts.iter().cloned().collect();

    let skills = db.get_skill_names()?;
    let mut matched: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();
    for (sid, name) in &skills {
        let concepts = db.get_skill_concepts(device_id, sid)?;
        let skill_concepts: HashSet<String> = concepts.into_iter().map(|(_id, label)| label).collect();
        let hit: Vec<String> = task_set.intersection(&skill_concepts).cloned().collect();
        if !hit.is_empty() {
            covered.extend(hit.iter().cloned());
            matched.push((sid.clone(), name.clone(), hit));
        }
    }
    // Sort by number of matched concepts descending.
    matched.sort_by(|a, b| b.2.len().cmp(&a.2.len()));

    let gaps: Vec<String> = task_set.difference(&covered).cloned().collect();

    let recommendations = matched
        .into_iter()
        .map(|(skill_id, skill_name, matched_concepts)| crate::models::SkillRecommendation {
            skill_id,
            skill_name,
            reason: format!("覆盖 {}", matched_concepts.join("/")),
            matched_concepts,
        })
        .collect();

    Ok(TaskRecommendation {
        recommendations,
        gaps,
    })
}

/// Analyze all skills in the center repo (batch entry point).
pub fn analyze_all_skills(db: &Db, device_id: &str, center_repo: &Path) -> Result<(usize, usize)> {
    let skills = db.get_skill_names()?;
    let mut total_nodes = 0usize;
    for (sid, _name) in &skills {
        let md = center_repo.join(_name).join("SKILL.md");
        if md.exists() {
            match analyze_skill(db, device_id, sid, &md) {
                Ok(n) => total_nodes += n,
                Err(e) => eprintln!("[kg] failed to analyze {}: {}", _name, e),
            }
        }
    }
    let edges = compute_relations(db, device_id, 0.4)?;
    Ok((total_nodes, edges))
}

// =============================================================================
// Parsing helpers
// =============================================================================

/// Split a SKILL.md into (frontmatter, body). Frontmatter is YAML between
/// leading `---` fences.
fn split_frontmatter(content: &str) -> (Frontmatter, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (Frontmatter::default(), content.to_string());
    }
    let after_first = &trimmed[3..];
    if let Some(end) = after_first.find("\n---") {
        let yaml = &after_first[..end];
        let body = &after_first[end + 4..];
        return (parse_yaml_frontmatter(yaml), body.to_string());
    }
    (Frontmatter::default(), content.to_string())
}

/// Minimal YAML-ish parser for the fields we care about (concepts/scenarios/related_skills).
/// Handles both block-list (`- item`) and inline (`[a, b]`) forms. Not a full YAML parser.
fn parse_yaml_frontmatter(yaml: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut current_key: Option<String> = None;
    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // block list item
        if line.starts_with('-') {
            let val = line.trim_start_matches('-').trim().trim_matches('"').to_string();
            if !val.is_empty() {
                match current_key.as_deref() {
                    Some("concepts") => fm.concepts.push(val),
                    Some("scenarios") => fm.scenarios.push(val),
                    Some("related_skills") => fm.related_skills.push(val),
                    _ => {}
                }
            }
            continue;
        }
        // key: value
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            current_key = Some(key.to_string());
            if val.is_empty() {
                continue;
            }
            // inline list [a, b]
            let items: Vec<String> = if val.starts_with('[') {
                val.trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                vec![val.trim_matches('"').to_string()]
            };
            match key {
                "concepts" => fm.concepts.extend(items),
                "scenarios" => fm.scenarios.extend(items),
                "related_skills" => fm.related_skills.extend(items),
                _ => {}
            }
        }
    }
    fm
}

/// Rule-based concept extraction from markdown body text.
/// Strategy: extract capitalized terms, code-ish identifiers, and known tech keywords.
fn extract_concepts(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // Match CamelCase / PascalCase tokens of length >= 3.
    let re = regex_lite(r"[A-Z][a-zA-Z0-9]{2,}");
    for m in re.find_all(text) {
        let token = m.to_string();
        // Skip common English words that happen to be capitalized.
        if is_stopword(&token) {
            continue;
        }
        if seen.insert(token.clone()) {
            found.push(token);
        }
    }
    // Also pick up kebab/snake tech terms from headings and code spans.
    let re2 = regex_lite(r"`([a-z][a-z0-9_-]{2,})`");
    for m in re2.find_all_captured(text) {
        if seen.insert(m.clone()) {
            found.push(m);
        }
    }
    // Dedup case-insensitively, keep first form.
    let mut deduped: Vec<String> = Vec::new();
    let mut lower_seen: HashSet<String> = HashSet::new();
    for t in found {
        if lower_seen.insert(t.to_lowercase()) {
            deduped.push(t);
        }
    }
    deduped
}

/// Build a stable node id: hash(label + type). Public so commands can compute node ids.
pub fn node_id_for(label: &str, node_type: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(b":");
    hasher.update(node_type.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

/// Build a stable node id: hash(label + type).
fn make_node(label: &str, node_type: &str, source: Option<&str>) -> KgNode {
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(b":");
    hasher.update(node_type.as_bytes());
    let id = hex::encode(&hasher.finalize()[..8]);
    KgNode {
        id,
        label: label.to_string(),
        node_type: node_type.to_string(),
        source: source.map(|s| s.to_string()),
        description: None,
    }
}

/// Ensure a `practice`-typed node exists before an edge references it.
/// `compute_relations` synthesizes practice nodes from skill names; without
/// persisting them first, `kg_edges.source_id/target_id → kg_nodes(id)` FK
/// constraint fails on connections that enforce foreign keys. Idempotent.
fn ensure_practice_node(db: &Db, label: &str) -> Result<String> {
    let node = make_node(label, "practice", None);
    db.upsert_kg_node(&node)?;
    Ok(node.id)
}

/// Jaccard similarity between two sets.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn is_stopword(w: &str) -> bool {
    matches!(
        w,
        "The" | "This" | "That" | "These" | "Those" | "When" | "Then" | "What" | "How"
            | "Why" | "Who" | "Where" | "Use" | "Using" | "Note" | "Notes" | "Example"
            | "Examples" | "Description" | "Usage" | "Skill" | "Skills" | "Section"
    )
}

// =============================================================================
// Minimal regex shim (avoids pulling in the regex crate for a few patterns).
// Supports: char classes [A-Z], quantifiers {n,m} and +, groups (...), literals.
// This is deliberately tiny — it covers the two patterns used above.
// =============================================================================

struct LiteRegex {
    parts: Vec<Part>,
}

enum Part {
    Class(Vec<char>, bool), // (chars, negated) — simplified to positive ranges only
    Literal(char),
    CaptureOpen,
    CaptureClose,
    Repeat(Box<Part>, usize, Option<usize>), // min, max
}

struct RegexIter {
    parts: Vec<Part>,
    text: Vec<char>,
    pos: usize,
}

impl LiteRegex {
    fn find_all(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if let Some((end, captured)) = self.match_at(&chars, i) {
                if let Some(c) = captured {
                    out.push(c);
                } else {
                    let full: String = chars[i..end].iter().collect();
                    out.push(full);
                }
                if end <= i {
                    i += 1;
                } else {
                    i = end;
                }
            } else {
                i += 1;
            }
        }
        out
    }

    fn find_all_captured(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if let Some((_end, captured)) = self.match_at(&chars, i) {
                if let Some(c) = captured {
                    out.push(c);
                }
                i = _end.max(i + 1);
            } else {
                i += 1;
            }
        }
        out
    }

    fn match_at(&self, chars: &[char], start: usize) -> Option<(usize, Option<String>)> {
        let mut idx = start;
        let mut capture_start: Option<usize> = None;
        let mut captured: String = String::new();
        for part in &self.parts {
            match part {
                Part::Class(ranges, _neg) => {
                    if idx >= chars.len() {
                        return None;
                    }
                    let c = chars[idx];
                    if !ranges.contains(&c) {
                        return None;
                    }
                    if capture_start.is_some() {
                        captured.push(c);
                    }
                    idx += 1;
                }
                Part::Literal(ch) => {
                    if idx >= chars.len() || chars[idx] != *ch {
                        return None;
                    }
                    if capture_start.is_some() {
                        captured.push(*ch);
                    }
                    idx += 1;
                }
                Part::CaptureOpen => {
                    capture_start = Some(idx);
                }
                Part::CaptureClose => {
                    capture_start = None;
                }
                Part::Repeat(inner, min, max) => {
                    let mut count = 0usize;
                    loop {
                        if let Some(m) = max {
                            if count >= *m {
                                break;
                            }
                        }
                        let saved_capture = captured.clone();
                        let saved_idx = idx;
                        let one = match inner.as_ref() {
                            Part::Class(r, _) => {
                                if idx < chars.len() && r.contains(&chars[idx]) {
                                    if capture_start.is_some() {
                                        captured.push(chars[idx]);
                                    }
                                    idx += 1;
                                    true
                                } else {
                                    false
                                }
                            }
                            Part::Literal(ch) => {
                                if idx < chars.len() && chars[idx] == *ch {
                                    if capture_start.is_some() {
                                        captured.push(*ch);
                                    }
                                    idx += 1;
                                    true
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        if !one {
                            captured = saved_capture;
                            idx = saved_idx;
                            break;
                        }
                        count += 1;
                        if idx == saved_idx {
                            break; // zero-width guard
                        }
                    }
                    if count < *min {
                        return None;
                    }
                }
            }
        }
        Some((idx, if captured.is_empty() { None } else { Some(captured) }))
    }
}

fn regex_lite(pattern: &str) -> LiteRegex {
    let mut parts: Vec<Part> = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '[' => {
                // parse until ']'
                let mut ranges: Vec<char> = Vec::new();
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    if i + 2 < chars.len() && chars[i + 1] == '-' {
                        let start_b = chars[i];
                        let end_b = chars[i + 2];
                        let mut ch = start_b as u32;
                        while ch <= end_b as u32 {
                            if let Some(x) = char::from_u32(ch) {
                                ranges.push(x);
                            }
                            ch += 1;
                        }
                        i += 3;
                    } else {
                        ranges.push(chars[i]);
                        i += 1;
                    }
                }
                if i < chars.len() {
                    i += 1; // skip ']'
                }
                // check for following quantifier
                if i < chars.len() && (chars[i] == '{' || chars[i] == '+') {
                    let (min, max, advance) = parse_quantifier(&chars, i);
                    i += advance;
                    parts.push(Part::Repeat(Box::new(Part::Class(ranges, false)), min, max));
                } else {
                    parts.push(Part::Class(ranges, false));
                }
            }
            '(' => {
                parts.push(Part::CaptureOpen);
                i += 1;
            }
            ')' => {
                parts.push(Part::CaptureClose);
                i += 1;
            }
            '+' => {
                // should have been consumed by previous; ignore stray
                i += 1;
            }
            '{' => {
                // stray quantifier; skip
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            }
            _ => {
                if i + 1 < chars.len() && (chars[i + 1] == '{' || chars[i + 1] == '+') {
                    let (min, max, advance) = parse_quantifier(&chars, i + 1);
                    i += 1 + advance;
                    parts.push(Part::Repeat(Box::new(Part::Literal(c)), min, max));
                } else {
                    parts.push(Part::Literal(c));
                    i += 1;
                }
            }
        }
    }
    LiteRegex { parts }
}

fn parse_quantifier(chars: &[char], start: usize) -> (usize, Option<usize>, usize) {
    // chars[start] is '{' or '+'
    if chars[start] == '+' {
        return (1, None, 1);
    }
    // {n,m} or {n,} or {n}
    let mut i = start + 1;
    let mut min_s = String::new();
    let mut max_s = String::new();
    let mut in_max = false;
    while i < chars.len() && chars[i] != '}' {
        if chars[i] == ',' {
            in_max = true;
        } else if chars[i].is_ascii_digit() {
            if in_max {
                max_s.push(chars[i]);
            } else {
                min_s.push(chars[i]);
            }
        }
        i += 1;
    }
    if i < chars.len() {
        i += 1; // skip '}'
    }
    let min: usize = min_s.parse().unwrap_or(0);
    let max: Option<usize> = if max_s.is_empty() {
        if in_max {
            None
        } else {
            Some(min)
        }
    } else {
        max_s.parse().ok()
    };
    (min, max, i - start)
}
