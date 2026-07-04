//! PRD-08: analysis engine — a 1:1 Rust port of AI-Digest's
//! `src/digest/metrics.py`.
//!
//! Two dimensions are computed here:
//! - **Token** (cost): how much was spent. Key signals: cache ratio, per-source
//!   breakdown, estimated reference cost, heavy sessions.
//! - **Prompt** (collaboration): how well AI is used. Key signals: the four
//!   semantic axes, agent-autonomy coefficient, 0-100 quality score, suggestions.
//!
//! The bridge is [`cost_efficiency_leverage`]: a single ROI number unifying the
//! two dimensions. Its denominator is **variable cost only** (subscription is a
//! fixed fee that does not move with tokens) — this is a PRD hard rule.
//!
//! Caliber is kept byte-aligned with the Python original so the two tools
//! produce comparable numbers. See `src/digest/metrics.py` for the reference.

use std::collections::BTreeMap;

use crate::models::{
    AgentCoefficientByTool, CostSplit, EntityMetrics, Leverage, PromptDimension, PromptMaturity,
    PromptPenetration, PromptQuality, PromptSemantics, PromptSample, RoleProfile, TokenCost,
    TokenDimension, TokenDiagnostics, TokenDistribution, TokenEfficiency, TokenScale, UsageSample,
};
use crate::pricing::Pricing;
use crate::settings::BillingMode;

// ---------------------------------------------------------------------------
// Token dimension
// ---------------------------------------------------------------------------

/// Aggregate the *cost* dimension over a list of usage samples. Mirrors
/// `metrics.token_metrics`. Returns the full token-dimension payload.
pub fn token_metrics(usages: &[UsageSample], pricing: &Pricing) -> TokenDimension {
    if usages.is_empty() {
        return TokenDimension::default();
    }

    let total_tokens: i64 = usages.iter().map(|u| u.total_tokens).sum();
    let total_input: i64 = usages.iter().map(|u| u.input_tokens).sum();
    let total_output: i64 = usages.iter().map(|u| u.output_tokens).sum();
    let total_cache_read: i64 = usages.iter().map(|u| u.cache_read_input_tokens).sum();
    let total_cache_create: i64 = usages.iter().map(|u| u.cache_creation_input_tokens).sum();
    let total_reasoning: i64 = usages.iter().map(|u| u.reasoning_tokens).sum();
    let total_calls: i64 = usages.iter().map(|u| u.model_calls).sum();
    let total_tool_calls: i64 = usages.iter().map(|u| u.tool_calls).sum();
    let total_duration_ms: i64 = usages.iter().filter_map(|u| u.duration_ms).sum();

    // cache_ratio relative to total_tokens (platform-consistent: total always
    // sums all categories, while input_tokens may or may not include cache).
    let cache_ratio = if total_tokens != 0 {
        total_cache_read as f64 / total_tokens as f64
    } else {
        0.0
    };
    // "Fresh" tokens = tokens that incurred real compute cost.
    let fresh_tokens = total_tokens - total_cache_read;

    let by_platform = group_sum(usages.iter().map(|u| (u.source.as_str(), u.total_tokens)));
    let by_project = top_n(
        group_sum(usages.iter().map(|u| (u.project_path.as_deref().unwrap_or("-"), u.total_tokens))),
        10,
    );
    let by_model = group_sum_ci(usages.iter().map(|u| (u.model_id.as_deref().unwrap_or("-"), u.total_tokens)));

    // Estimated reference cost.
    let est_cost: f64 = (usages.iter().map(|u| pricing.estimate_cost(u)).sum::<f64>() * 100.0).round() / 100.0;
    let mut by_platform_cny: BTreeMap<String, f64> = BTreeMap::new();
    for u in usages {
        let key = u.source.clone();
        let val = pricing.estimate_cost(u);
        *by_platform_cny.entry(key).or_insert(0.0) += val;
    }
    // round each to 2 decimals
    for v in by_platform_cny.values_mut() {
        *v = (*v * 100.0).round() / 100.0;
    }
    let billing_mix = group_count(
        usages
            .iter()
            .map(|u| pricing.billing_mode_for(&u.model_id.clone().unwrap_or_default()).to_string()),
    );

    // Heavy sessions (top 10% by tokens) — possible context bloat.
    let mut sorted: Vec<&UsageSample> = usages.iter().collect();
    sorted.sort_unstable_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    let heavy_n = (sorted.len() / 10).max(1);
    let heavy_sessions: Vec<crate::models::HeavySession> = sorted[..heavy_n]
        .iter()
        .map(|u| crate::models::HeavySession {
            session_id: u.session_id.clone(),
            project: u.project_path.clone(),
            model: u.model_id.clone(),
            total_tokens: u.total_tokens,
            cache_read: u.cache_read_input_tokens,
            model_calls: u.model_calls,
        })
        .collect();

    let n = usages.len() as i64;
    let duration_hours = (total_duration_ms as f64 / 3_600_000.0 * 10.0).round() / 10.0;
    TokenDimension {
        scale: TokenScale {
            total_tokens,
            fresh_tokens,
            input_tokens: total_input,
            output_tokens: total_output,
            reasoning_tokens: total_reasoning,
            cache_read_tokens: total_cache_read,
            cache_creation_tokens: total_cache_create,
            model_calls: total_calls,
            tool_calls: total_tool_calls,
            duration_hours,
        },
        distribution: TokenDistribution {
            by_platform,
            by_project,
            by_model,
        },
        efficiency: TokenEfficiency {
            tokens_per_session: total_tokens / n,
            fresh_tokens_per_session: fresh_tokens / n,
            tokens_per_call: if total_calls != 0 { total_tokens / total_calls } else { 0 },
            tokens_per_hour: if total_duration_ms != 0 {
                (total_tokens as f64 / (total_duration_ms as f64 / 3_600_000.0)) as i64
            } else {
                0
            },
        },
        cost: TokenCost {
            est_cost_cny: est_cost,
            by_platform_cny,
            billing_mix,
        },
        diagnostics: TokenDiagnostics {
            cache_ratio: (cache_ratio * 1e4).round() / 1e4,
            heavy_sessions,
        },
    }
}

// ---------------------------------------------------------------------------
// Prompt dimension
// ---------------------------------------------------------------------------

/// Aggregate the *collaboration* dimension over a list of prompt samples.
/// Mirrors `metrics.prompt_metrics`. Semantic distributions are computed only
/// over classified prompts; `classified_ratio` tells the UI how representative
/// they are.
pub fn prompt_metrics(prompts: &[PromptSample]) -> PromptDimension {
    if prompts.is_empty() {
        return PromptDimension::default();
    }

    let total = prompts.len();
    let total_duration_ms: i64 = prompts.iter().filter_map(|p| p.duration_ms).sum();
    let total_tool_calls: i64 = prompts.iter().filter_map(|p| p.tool_calls).sum();
    let agent_coeff = if total != 0 { total_duration_ms as f64 / total as f64 } else { 0.0 };

    let by_platform = group_count(prompts.iter().map(|p| p.source.clone()));
    let by_project = top_n(group_count(prompts.iter().map(|p| p.project_path.clone().unwrap_or_default())), 10);

    // Semantic distributions (only over prompts that have been classified).
    let classified: Vec<&PromptSample> = prompts.iter().filter(|p| p.requested_action.is_some()).collect();
    let classified_ratio = if total != 0 { classified.len() as f64 / total as f64 } else { 0.0 };

    let action_dist = group_count(classified.iter().map(|p| p.requested_action.clone().unwrap_or_default()));
    let object_dist = group_count(classified.iter().map(|p| p.target_object.clone().unwrap_or_default()));
    let state_dist = group_count(classified.iter().map(|p| p.interaction_state.clone().unwrap_or_default()));
    let mode_dist = group_count(classified.iter().map(|p| p.interaction_mode.clone().unwrap_or_default()));

    // Clarification/correction rate.
    let redo = classified
        .iter()
        .filter(|p| matches!(p.interaction_state.as_deref(), Some("补充澄清") | Some("纠偏修正")))
        .count();
    let redo_rate = if !classified.is_empty() { redo as f64 / classified.len() as f64 } else { 0.0 };

    // Planning ratio.
    let plan = classified.iter().filter(|p| p.requested_action.as_deref() == Some("规划")).count();
    let plan_ratio = if !classified.is_empty() { plan as f64 / classified.len() as f64 } else { 0.0 };

    // Test-object penetration.
    let test_obj = classified.iter().filter(|p| p.target_object.as_deref() == Some("测试")).count();
    let test_ratio = if !classified.is_empty() { test_obj as f64 / classified.len() as f64 } else { 0.0 };

    let agent_h = agent_coeff / 3_600_000.0; // hours
    let quality = quality_score(redo_rate, plan_ratio, agent_h, test_ratio, classified_ratio);
    let suggestions = improvement_suggestions(redo_rate, plan_ratio, test_ratio, agent_h);

    PromptDimension {
        penetration: PromptPenetration {
            total_prompts: total as i64,
            by_platform,
            by_project,
        },
        maturity: PromptMaturity {
            agent_coefficient_ms_per_prompt: agent_coeff as i64,
            agent_coefficient_min_per_prompt: (agent_coeff / 60_000.0 * 10.0).round() / 10.0,
            avg_tool_calls_per_prompt: ((total_tool_calls as f64 / total as f64) * 10.0).round() / 10.0,
            total_tool_calls,
        },
        semantics: PromptSemantics {
            classified_ratio: (classified_ratio * 1e4).round() / 1e4,
            requested_action: action_dist,
            target_object: object_dist,
            interaction_state: state_dist,
            interaction_mode: mode_dist,
        },
        quality: PromptQuality {
            score: quality,
            clarification_correction_rate: (redo_rate * 1e4).round() / 1e4,
            planning_ratio: (plan_ratio * 1e4).round() / 1e4,
            test_object_ratio: (test_ratio * 1e4).round() / 1e4,
            improvement_suggestions: suggestions,
        },
    }
}

/// Heuristic 0-100 quality score. Each of four components contributes up to 25.
/// Mirrors `metrics._quality_score`.
fn quality_score(redo_rate: f64, plan_ratio: f64, agent_h: f64, test_ratio: f64, classified_ratio: f64) -> i64 {
    // C1: low redo rate is good (redo < 20% → full marks).
    let c1 = (25.0 - (redo_rate - 0.20).max(0.0) * 100.0).max(0.0);
    // C2: some planning is good (plan > 10% → full marks).
    let c2 = (plan_ratio / 0.10 * 25.0).min(25.0);
    // C3: agent autonomy (drives > 5 min → full marks).
    let c3 = (agent_h / (5.0 / 60.0) * 25.0).min(25.0);
    // C4: AI entering test chain + classification coverage.
    let c4 = (test_ratio / 0.05 * 15.0).min(15.0) + (classified_ratio * 10.0).min(10.0);
    let score = (c1 + c2 + c3 + c4).max(0.0).min(100.0);
    score as i64
}

/// Rule-driven Chinese improvement suggestions. Mirrors
/// `metrics._improvement_suggestions`. Pure rules, no LLM.
fn improvement_suggestions(redo_rate: f64, plan_ratio: f64, test_ratio: f64, agent_h: f64) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if redo_rate > 0.30 {
        out.push(format!(
            "澄清/纠偏率偏高 ({:.0}%)：首轮 Prompt 质量待提升，建议补充上下文、约束与验收标准，减少返工",
            redo_rate * 100.0
        ));
    }
    if plan_ratio < 0.10 {
        out.push("规划类 Prompt 占比偏低：建议复杂任务先让 AI 出方案/计划，再进入实施".to_string());
    }
    if test_ratio < 0.05 {
        out.push("AI 较少进入测试/质量链路：建议让 AI 参与测试编写与检查，提升渗透深度".to_string());
    }
    if agent_h < (5.0 / 60.0) {
        out.push(format!(
            "Agent 化系数偏低 ({:.1} 分钟/prompt)：交互偏碎片化，可尝试长链路 Agent 任务，沉淀可复用 Skill/命令",
            agent_h * 60.0
        ));
    }
    if out.is_empty() {
        out.push("各维度均衡，协作状态健康，保持当前节奏并持续沉淀 Skill".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Bridge: cost-efficiency leverage
// ---------------------------------------------------------------------------

/// Bridge the two dimensions via a leverage ratio. Mirrors
/// `metrics.cost_efficiency_leverage`. **Denominator = variable cost only**
/// (PRD hard requirement): subscription is a fixed fee that does not move with
/// tokens, so mixing it in dilutes the marginal-efficiency signal.
pub fn cost_efficiency_leverage(usages: &[UsageSample], prompts: &[PromptSample], pricing: &Pricing) -> Leverage {
    let mut variable_cost = 0.0_f64;
    let mut subscription_cost = 0.0_f64;
    for u in usages {
        let c = pricing.estimate_cost(u);
        if pricing.billing_mode_for(&u.model_id.clone().unwrap_or_default()) == BillingMode::PayAsYouGo {
            variable_cost += c;
        } else {
            subscription_cost += c;
        }
    }
    let total_cost = variable_cost + subscription_cost;
    let total_prompts = prompts.len() as i64;
    let total_tool_calls: i64 = prompts.iter().filter_map(|p| p.tool_calls).sum::<i64>()
        + usages.iter().map(|u| u.tool_calls).sum::<i64>();
    let fresh_tokens: i64 = usages.iter().map(|u| u.total_tokens - u.cache_read_input_tokens).sum();

    // Output proxy: tool calls (real side effects) + prompts (intent density).
    let output_proxy = total_tool_calls + total_prompts;
    let leverage = if variable_cost != 0.0 { output_proxy as f64 / variable_cost } else { 0.0 };
    let cost_per_prompt = if total_prompts != 0 { total_cost / total_prompts as f64 } else { 0.0 };

    Leverage {
        est_cost_cny: (total_cost * 100.0).round() / 100.0,
        variable_cost_cny: (variable_cost * 100.0).round() / 100.0,
        subscription_cost_cny: (subscription_cost * 100.0).round() / 100.0,
        fresh_tokens,
        output_proxy,
        leverage_per_cny: (leverage * 100.0).round() / 100.0,
        cost_per_prompt_cny: (cost_per_prompt * 1e4).round() / 1e4,
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Sum values by a string key; "-" is used for missing keys (mirrors Python).
fn group_sum<'a, I: Iterator<Item = (&'a str, i64)>>(iter: I) -> BTreeMap<String, i64> {
    let mut m: BTreeMap<String, i64> = BTreeMap::new();
    for (k, v) in iter {
        *m.entry(k.to_string()).or_insert(0) += v;
    }
    m
}

/// Case-insensitive grouping by a model id, keeping the last-seen spelling for
/// display. Mirrors `_group_sum_ci`.
fn group_sum_ci<'a, I: Iterator<Item = (&'a str, i64)>>(iter: I) -> BTreeMap<String, i64> {
    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    let mut display: BTreeMap<String, String> = BTreeMap::new();
    for (raw, v) in iter {
        let key = raw.to_lowercase();
        *totals.entry(key.clone()).or_insert(0) += v;
        display.insert(key, raw.to_string()); // last-wins is fine
    }
    display.into_iter().map(|(k, d)| (d, totals.remove(&k).unwrap_or(0))).collect()
}

/// Count occurrences by string key.
fn group_count<I: Iterator<Item = String>>(iter: I) -> BTreeMap<String, i64> {
    let mut m: BTreeMap<String, i64> = BTreeMap::new();
    for k in iter {
        *m.entry(k).or_insert(0) += 1;
    }
    m
}

/// Keep the top N entries by value (ties broken by key for determinism).
fn top_n(m: BTreeMap<String, i64>, n: usize) -> BTreeMap<String, i64> {
    let mut pairs: Vec<(String, i64)> = m.into_iter().collect();
    pairs.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs.truncate(n);
    pairs.into_iter().collect()
}

// ---------------------------------------------------------------------------
// v2 entity-derived metrics (role profile, cost split, by-tool maturity).
// Ported from metrics.py:286-360. Computed directly from collected_* query
// rows — no separate entity tables.
// ---------------------------------------------------------------------------

/// Goal 1: each tool's main role, as tool × action row-normalized shares.
/// `role_rows` is (source, requested_action, count). Mirrors `metrics.role_profile`.
pub fn role_profile(role_rows: &[(String, String, i64)]) -> RoleProfile {
    // source -> action -> count
    let mut by_tool: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for (tool, action, n) in role_rows {
        *by_tool
            .entry(tool.clone())
            .or_default()
            .entry(action.clone())
            .or_insert(0) += *n;
    }
    let mut tools: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for (tool, counts) in &by_tool {
        let total: i64 = counts.values().sum();
        if total <= 0 {
            continue;
        }
        let shares: BTreeMap<String, f64> = counts
            .iter()
            .map(|(k, v)| (k.clone(), (*v as f64 / total as f64 * 1e4).round() / 1e4))
            .collect();
        tools.insert(tool.clone(), shares);
    }
    RoleProfile { tools, degraded: false }
}

/// Goal 3: split cost into variable (pay-as-you-go) vs subscription. Variable
/// is real money and alone feeds the leverage denominator. Subscription reports
/// call intensity (not a fictitious amount). Mirrors `cost_by_billing_mode`.
pub fn cost_by_billing_mode(usages: &[UsageSample], pricing: &Pricing) -> CostSplit {
    let mut variable_cny = 0.0_f64;
    let mut variable_calls = 0i64;
    let mut subscription_calls = 0i64;
    for u in usages {
        let amt = pricing.estimate_cost(u);
        if pricing.billing_mode_for(&u.model_id.clone().unwrap_or_default()) == BillingMode::PayAsYouGo {
            variable_cny += amt;
            variable_calls += 1;
        } else {
            subscription_calls += 1;
        }
    }
    let subscription_intensity = if subscription_calls > 50 {
        "high"
    } else if subscription_calls > 10 {
        "medium"
    } else {
        "low"
    }
    .to_string();
    CostSplit {
        variable_cny: (variable_cny * 100.0).round() / 100.0,
        variable_calls,
        subscription_calls,
        subscription_intensity,
    }
}

/// Goal 2: per-tool agent coefficient (avg prompt duration in minutes), only
/// over sources that persist trustworthy duration. Sources without trusted
/// duration are omitted (not 0). `rows` is (source, duration_ms).
pub fn agent_coefficient_by_tool(rows: &[(String, i64)]) -> AgentCoefficientByTool {
    // source -> Vec<duration_ms>
    let mut by_tool: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (src, dur) in rows {
        by_tool.entry(src.clone()).or_default().push(*dur);
    }
    let mut tools: BTreeMap<String, f64> = BTreeMap::new();
    for (src, durs) in &by_tool {
        if durs.is_empty() {
            continue;
        }
        let avg_ms = durs.iter().sum::<i64>() as f64 / durs.len() as f64;
        tools.insert(src.clone(), (avg_ms / 60_000.0 * 10.0).round() / 10.0);
    }
    AgentCoefficientByTool { tools }
}

/// Build the full entity-metrics bundle for a window. `degraded` reflects
/// whether the role profile fell back from the ≥0.6 set to all labels.
pub fn entity_metrics(
    role_rows_high_conf: &[(String, String, i64)],
    role_rows_all: &[(String, String, i64)],
    usage_samples: &[UsageSample],
    prompt_dur_rows: &[(String, i64)],
    pricing: &Pricing,
) -> EntityMetrics {
    let mut profile = role_profile(role_rows_high_conf);
    let mut degraded = false;
    if profile.tools.is_empty() {
        // Fall back to unfiltered labels; flag the degradation.
        profile = role_profile(role_rows_all);
        degraded = !profile.tools.is_empty();
    }
    profile.degraded = degraded;
    EntityMetrics {
        role_profile: profile,
        cost_split: cost_by_billing_mode(usage_samples, pricing),
        agent_coefficient_by_tool: agent_coefficient_by_tool(prompt_dur_rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(model: &str, input: i64, cache_read: i64, cache_create: i64, output: i64, reasoning: i64) -> UsageSample {
        UsageSample {
            session_id: "s".into(),
            source: "claude-code".into(),
            project_path: None,
            model_id: Some(model.into()),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: reasoning,
            cache_creation_input_tokens: cache_create,
            cache_read_input_tokens: cache_read,
            total_tokens: input + output + reasoning + cache_read + cache_create,
            model_calls: 1,
            tool_calls: 0,
            duration_ms: None,
        }
    }

    fn p(action: Option<&str>, state: Option<&str>, object: Option<&str>) -> PromptSample {
        PromptSample {
            session_id: "s".into(),
            source: "claude-code".into(),
            project_path: None,
            duration_ms: Some(300_000), // 5 min
            tool_calls: Some(3),
            requested_action: action.map(String::from),
            target_object: object.map(String::from),
            interaction_state: state.map(String::from),
            interaction_mode: None,
        }
    }

    #[test]
    fn quality_score_full_marks_for_healthy_values() {
        // redo 0%, plan 10%, agent 5min/prompt, test 5%, full classification.
        let score = quality_score(0.0, 0.10, 5.0 / 60.0, 0.05, 1.0);
        assert_eq!(score, 100);
    }

    #[test]
    fn quality_score_zero_for_terrible_values() {
        let score = quality_score(0.9, 0.0, 0.0, 0.0, 0.0);
        // redo 0.9 → c1 = 25 - 0.7*100 = -45 → clamped 0; others 0.
        assert_eq!(score, 0);
    }

    #[test]
    fn prompt_metrics_classified_ratio_and_distributions() {
        let prompts = vec![
            p(Some("规划"), Some("新任务"), Some("代码")),
            p(Some("生成"), Some("继续推进"), Some("代码")),
            p(None, None, None), // unclassified
        ];
        let m = prompt_metrics(&prompts);
        assert_eq!(m.penetration.total_prompts, 3);
        assert!((m.semantics.classified_ratio - 0.6667).abs() < 0.001);
        // 2/3 of classified are 代码
        assert_eq!(m.semantics.target_object.get("代码"), Some(&2));
    }

    #[test]
    fn suggestions_fire_on_weak_signals() {
        // redo 0.4 > 0.3, plan 0.0 < 0.1, test 0.0 < 0.05, agent 0 < 5min
        let s = improvement_suggestions(0.4, 0.0, 0.0, 0.0);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn suggestions_healthy_fallback() {
        let s = improvement_suggestions(0.1, 0.2, 0.1, 1.0);
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("健康"));
    }

    #[test]
    fn role_profile_row_normalizes_to_shares() {
        // claude-code: 修改×8, 规划×2 → 修改=0.8, 规划=0.2; zcode: 规划×4 → 规划=1.0.
        let rows = vec![
            ("claude-code".into(), "修改".into(), 8),
            ("claude-code".into(), "规划".into(), 2),
            ("zcode".into(), "规划".into(), 4),
        ];
        let profile = role_profile(&rows);
        let cc = profile.tools.get("claude-code").unwrap();
        assert!((cc.get("修改").unwrap() - 0.8).abs() < 1e-6);
        assert!((cc.get("规划").unwrap() - 0.2).abs() < 1e-6);
        let zc = profile.tools.get("zcode").unwrap();
        assert!((zc.get("规划").unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cost_by_billing_mode_splits_variable_vs_subscription() {
        let pricing = Pricing::from_db(vec![]);
        // deepseek-chat = pay_as_you_go; GLM-5.2 = subscription.
        let usages = vec![
            sample("deepseek-chat", 1_000_000, 0, 0, 0, 0), // 2.0 CNY
            sample("GLM-5.2", 1_000_000, 0, 0, 0, 0),        // would be 2.0 but subscription
        ];
        let split = cost_by_billing_mode(&usages, &pricing);
        assert!((split.variable_cny - 2.0).abs() < 1e-6, "variable was {}", split.variable_cny);
        assert_eq!(split.variable_calls, 1);
        assert_eq!(split.subscription_calls, 1);
    }

    #[test]
    fn agent_coefficient_by_tool_omits_zero_duration_sources() {
        // Only sources with at least one duration row appear.
        let rows = vec![
            ("zcode".into(), 300_000), // 5 min
            ("zcode".into(), 600_000), // 10 min → avg 7.5
        ];
        let coeff = agent_coefficient_by_tool(&rows);
        assert!((coeff.tools.get("zcode").unwrap() - 7.5).abs() < 1e-6);
        // A source with no duration rows is simply absent (not 0).
        assert!(coeff.tools.get("claude-code").is_none());
    }

    #[test]
    fn entity_metrics_degrades_to_all_labels_when_high_conf_empty() {
        let pricing = Pricing::from_db(vec![]);
        // high-conf set empty; all-labels set has one row.
        let all_rows = vec![("claude-code".into(), "修改".into(), 1)];
        let em = entity_metrics(&[], &all_rows, &[], &[], &pricing);
        assert!(em.role_profile.degraded);
        assert!(em.role_profile.tools.contains_key("claude-code"));
    }
}
