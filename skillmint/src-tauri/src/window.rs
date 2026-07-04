//! PRD-08: window metrics with 环比 (period-over-period) & 同比 (year-over-year).
//!
//! A 1:1 Rust port of the orchestration part of AI-Digest's
//! `metrics.window_metrics` / `_window_dates` / `_summary` / `_delta`.
//!
//! Given a window kind (day/week/month) and a reference date, this module:
//!   1. computes the current, previous, and year-over-year date windows;
//!   2. reads the usage/prompt samples for each window from the DB;
//!   3. reduces each window to a 9-scalar summary;
//!   4. builds the comparison table (current + optional mom/yoy delta);
//!   5. attaches the full token/prompt dimensions + leverage for the current
//!      window.
//!
//! Time storage note: `collected_sessions.start_time` is **epoch seconds**
//! (not ms). Window bounds here are therefore computed and compared in
//! epoch seconds, consistent with `get_agent_usage_summary`.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Weekday};

use crate::db::Db;
use crate::metrics::{
    cost_efficiency_leverage, entity_metrics, prompt_metrics, token_metrics,
};
use crate::models::{ComparisonEntry, Delta, WindowMetrics, WindowRange};
use crate::pricing::Pricing;

/// The window kind: `day`, `week` (Mon-Sun), or `month`.
pub fn normalize_kind(kind: &str) -> Result<&'static str, String> {
    match kind {
        "day" => Ok("day"),
        "week" => Ok("week"),
        "month" => Ok("month"),
        _ => Err(format!("unknown window kind: {kind} (expected day|week|month)")),
    }
}

/// The date windows for a comparison: current, previous (环比), YoY (同比).
/// Each is an inclusive `[start, end]` pair of `NaiveDate`.
struct Windows {
    cs: NaiveDate,
    ce: NaiveDate,
    ps: NaiveDate,
    pe: NaiveDate,
    yys: NaiveDate,
    yye: NaiveDate,
}

/// Compute the three windows. Mirrors `metrics._window_dates`.
fn window_dates(kind: &str, ref_date: NaiveDate) -> Windows {
    match kind {
        "day" => {
            let cs = ref_date;
            Windows {
                cs,
                ce: cs,
                ps: cs.pred_opt().unwrap(),
                pe: cs.pred_opt().unwrap(),
                yys: cs.with_year(cs.year() - 1).unwrap_or(cs),
                yye: cs.with_year(cs.year() - 1).unwrap_or(cs),
            }
        }
        "week" => {
            // Monday-based start of the ref week.
            let offset = match ref_date.weekday() {
                Weekday::Mon => 0,
                Weekday::Tue => 1,
                Weekday::Wed => 2,
                Weekday::Thu => 3,
                Weekday::Fri => 4,
                Weekday::Sat => 5,
                Weekday::Sun => 6,
            };
            let cs = ref_date - chrono::Duration::days(offset);
            let ce = cs + chrono::Duration::days(6);
            let ps = cs - chrono::Duration::days(7);
            let pe = ps + chrono::Duration::days(6);
            Windows {
                cs,
                ce,
                ps,
                pe,
                yys: cs.with_year(cs.year() - 1).unwrap_or(cs),
                yye: ce.with_year(ce.year() - 1).unwrap_or(ce),
            }
        }
        "month" => {
            let cs = ref_date.with_day(1).unwrap_or(ref_date);
            let ce = last_day_of_month(cs);
            let ps = first_day_of_prev_month(cs);
            let pe = last_day_of_month(ps);
            Windows {
                cs,
                ce,
                ps,
                pe,
                yys: cs.with_year(cs.year() - 1).unwrap_or(cs),
                yye: ce.with_year(ce.year() - 1).unwrap_or(ce),
            }
        }
        _ => unreachable!("normalize_kind guards this"),
    }
}

fn last_day_of_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .unwrap_or(d)
        .pred_opt()
        .unwrap_or(d)
}

fn first_day_of_prev_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 1 {
        NaiveDate::from_ymd_opt(d.year() - 1, 12, 1).unwrap_or(d)
    } else {
        NaiveDate::from_ymd_opt(d.year(), d.month() - 1, 1).unwrap_or(d)
    }
}

/// Convert a date to the epoch-seconds timestamp of its local start-of-day.
/// Window bounds are compared against `start_time` (epoch secs).
/// Convert a calendar date to the UTC epoch-seconds timestamp of that date's
/// **local** midnight. This must match the frame `query_sessions_for_day` uses
/// (`date(start_time,'unixepoch','localtime')`): sessions are stored as UTC
/// epoch seconds and bucketed to a *local* date, so window bounds must also be
/// local-midnight-derived or day/week/month windows will be off by the tz offset.
fn day_start_secs(d: NaiveDate) -> i64 {
    use chrono::offset::Local;
    use chrono::{LocalResult, NaiveTime, TimeZone};
    // Local midnight on `d` → its UTC timestamp. `Local` resolves the machine tz.
    // `and_hms_opt` is normally infallible for 00:00:00, but be defensive.
    let naive = d.and_time(
        NaiveTime::from_hms_opt(0, 0, 0)
            .expect("00:00:00 is always valid"),
    );
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.timestamp(),
        // Ambiguous: prefer the earlier occurrence (standard-time side).
        LocalResult::Ambiguous(earliest, _latest) => earliest.timestamp(),
        // Non-existent local midnight (DST spring-forward gap). Use the start
        // of the next valid local second as a stable fallback.
        LocalResult::None => Local
            .from_local_datetime(&(naive + chrono::Duration::seconds(1)))
            .latest()
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp()),
    }
}

fn window_bounds(start: NaiveDate, end: NaiveDate) -> (i64, i64) {
    // end-of-day = start of (end+1) to make the bound inclusive in seconds.
    (day_start_secs(start), day_start_secs(end + chrono::Duration::days(1)) - 1)
}

/// PRD-08 §3.4 (P3): public helper that computes the current-window epoch-second
/// bounds [start, end] for a (kind, ref_date). Used by the cell-drill-down
/// command so it shares the exact windowing frame with window_metrics.
pub fn window_bounds_for(kind: &str, ref_date: NaiveDate) -> (i64, i64) {
    let w = window_dates(kind, ref_date);
    window_bounds(w.cs, w.ce)
}

/// The compact 9-scalar summary of one window, used for comparison. Mirrors
/// `metrics._summary`. Each field must appear in [`SUMMARY_KEYS`] in order.
#[derive(Debug, Clone, Default)]
struct Summary {
    sessions: f64,
    total_tokens: f64,
    fresh_tokens: f64,
    cache_ratio: f64,
    est_cost_cny: f64,
    prompts: f64,
    agent_coeff_min: f64,
    quality_score: f64,
    tool_calls: f64,
}

/// The ordered comparison-table keys (mirrors the Python `cur` dict order).
const SUMMARY_KEYS: &[&str] = &[
    "sessions",
    "total_tokens",
    "fresh_tokens",
    "cache_ratio",
    "est_cost_cny",
    "prompts",
    "agent_coeff_min",
    "quality_score",
    "tool_calls",
];

fn reduce_window(
    db: &Db,
    start: NaiveDate,
    end: NaiveDate,
    source: Option<&str>,
    pricing: &Pricing,
) -> anyhow::Result<(Summary, bool)> {
    let (s, e) = window_bounds(start, end);
    let usages = db.query_usage_samples(s, e, source)?;
    let prompts = db.query_prompt_samples(s, e, source)?;
    let has_data = !usages.is_empty() || !prompts.is_empty();

    let tm = token_metrics(&usages, pricing);
    let pm = prompt_metrics(&prompts);

    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    for u in &usages {
        sessions.insert(u.session_id.clone());
    }
    for p in &prompts {
        sessions.insert(p.session_id.clone());
    }

    let tool_calls = tm.scale.tool_calls as f64
        + prompts.iter().filter_map(|p| p.tool_calls).sum::<i64>() as f64;

    Ok((
        Summary {
            sessions: sessions.len() as f64,
            total_tokens: tm.scale.total_tokens as f64,
            fresh_tokens: tm.scale.fresh_tokens as f64,
            cache_ratio: tm.diagnostics.cache_ratio,
            est_cost_cny: tm.cost.est_cost_cny,
            prompts: pm.penetration.total_prompts as f64,
            agent_coeff_min: pm.maturity.agent_coefficient_min_per_prompt,
            quality_score: pm.quality.score as f64,
            tool_calls,
        },
        has_data,
    ))
}

/// Compute change vs a baseline. `pct` is `None` when the baseline is 0
/// (cannot divide) — mirrors `_delta`'s "基线为0" note.
fn delta(cur: f64, prev: f64) -> Delta {
    if prev == 0.0 {
        return Delta {
            current: cur,
            previous: prev,
            delta: cur,
            pct: None,
            note: Some("基线为0".into()),
        };
    }
    let pct = (cur - prev) / prev;
    Delta {
        current: cur,
        previous: prev,
        delta: cur - prev,
        pct: Some((pct * 1e4).round() / 1e4),
        note: None,
    }
}

/// The top-level entry point. Reads the DB for all three windows and assembles
/// the comparison table plus the current window's full dimensions + leverage.
/// Wraps [`compute_window_metrics`] with a memoization cache (analysis_window_cache)
/// so repeated window switches are instant until new data is collected.
/// Stable, cross-platform cache key for a (kind, ref_date, source) triple.
fn stable_hash_key(kind: &str, ref_date: NaiveDate, source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(ref_date.to_string().as_bytes());
    hasher.update(source.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

pub fn window_metrics(
    db: &Db,
    kind: &str,
    ref_date: NaiveDate,
    source: Option<&str>,
    pricing: &Pricing,
) -> anyhow::Result<WindowMetrics> {
    let src = source.unwrap_or("");
    let cache_key = stable_hash_key(kind, ref_date, src);

    // Cache hit?
    if let Some(payload) = db.get_cached_window_metrics(&cache_key) {
        if let Ok(m) = serde_json::from_str::<WindowMetrics>(&payload) {
            return Ok(m);
        }
    }

    let metrics = compute_window_metrics(db, kind, ref_date, source, pricing)?;
    // Store (best-effort; a serialization failure just skips caching).
    if let Ok(payload) = serde_json::to_string(&metrics) {
        let _ = db.set_cached_window_metrics(&cache_key, &payload);
    }
    Ok(metrics)
}

/// The uncached computation. Reads the DB for all three windows and assembles
/// the comparison table plus the current window's full dimensions + leverage.
fn compute_window_metrics(
    db: &Db,
    kind: &str,
    ref_date: NaiveDate,
    source: Option<&str>,
    pricing: &Pricing,
) -> anyhow::Result<WindowMetrics> {
    let w = window_dates(kind, ref_date);

    let (cur, _) = reduce_window(db, w.cs, w.ce, source, pricing)?;
    let (prev, has_prev) = reduce_window(db, w.ps, w.pe, source, pricing)?;
    let (yoy, has_yoy) = reduce_window(db, w.yys, w.yye, source, pricing)?;

    let cur_arr = [
        cur.sessions,
        cur.total_tokens,
        cur.fresh_tokens,
        cur.cache_ratio,
        cur.est_cost_cny,
        cur.prompts,
        cur.agent_coeff_min,
        cur.quality_score,
        cur.tool_calls,
    ];
    let prev_arr = [
        prev.sessions,
        prev.total_tokens,
        prev.fresh_tokens,
        prev.cache_ratio,
        prev.est_cost_cny,
        prev.prompts,
        prev.agent_coeff_min,
        prev.quality_score,
        prev.tool_calls,
    ];
    let yoy_arr = [
        yoy.sessions,
        yoy.total_tokens,
        yoy.fresh_tokens,
        yoy.cache_ratio,
        yoy.est_cost_cny,
        yoy.prompts,
        yoy.agent_coeff_min,
        yoy.quality_score,
        yoy.tool_calls,
    ];

    let mut comparison: BTreeMap<String, ComparisonEntry> = BTreeMap::new();
    for (i, key) in SUMMARY_KEYS.iter().enumerate() {
        comparison.insert(
            (*key).to_string(),
            ComparisonEntry {
                current: cur_arr[i],
                mom: if has_prev { Some(delta(cur_arr[i], prev_arr[i])) } else { None },
                yoy: if has_yoy { Some(delta(cur_arr[i], yoy_arr[i])) } else { None },
            },
        );
    }

    // Recompute the full dimensions for the current window only (the comparison
    // used a compact summary; the detail views need the full payload).
    let (cs_start, cs_end) = window_bounds(w.cs, w.ce);
    let cur_usages = db.query_usage_samples(cs_start, cs_end, source)?;
    let cur_prompts = db.query_prompt_samples(cs_start, cs_end, source)?;
    let token_dimension = token_metrics(&cur_usages, pricing);
    let prompt_dimension = prompt_metrics(&cur_prompts);
    let leverage = cost_efficiency_leverage(&cur_usages, &cur_prompts, pricing);

    // PRD-08 §3.4/§3.5 (P1): entity-derived metrics (role profile + cost split +
    // agent coefficient). Wrapped so a missing/empty result never breaks the
    // report — the entity view simply renders empty.
    let em = (|| -> anyhow::Result<crate::models::EntityMetrics> {
        let role_hi = db.query_role_profile(cs_start, cs_end, 0.6)?;
        let role_all = db.query_role_profile(cs_start, cs_end, 0.0)?;
        let dur_rows = db.query_prompt_durations_by_source(cs_start, cs_end)?;
        Ok(entity_metrics(&role_hi, &role_all, &cur_usages, &dur_rows, pricing))
    })()
    .unwrap_or_default();

    Ok(WindowMetrics {
        kind: kind.to_string(),
        ref_date: ref_date.to_string(),
        current_window: WindowRange { start: w.cs.to_string(), end: w.ce.to_string() },
        previous_window: WindowRange { start: w.ps.to_string(), end: w.pe.to_string() },
        yoy_window: WindowRange { start: w.yys.to_string(), end: w.yye.to_string() },
        has_previous_baseline: has_prev,
        has_yoy_baseline: has_yoy,
        comparison,
        token_dimension,
        prompt_dimension,
        leverage,
        entity_metrics: em,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_window_prev_is_yesterday() {
        let ref_date = NaiveDate::from_ymd_opt(2026, 6, 28).unwrap();
        let w = window_dates("day", ref_date);
        assert_eq!(w.cs, ref_date);
        assert_eq!(w.ce, ref_date);
        assert_eq!(w.ps, NaiveDate::from_ymd_opt(2026, 6, 27).unwrap());
        assert_eq!(w.yys, NaiveDate::from_ymd_opt(2025, 6, 28).unwrap());
    }

    #[test]
    fn week_window_is_monday_based() {
        // 2026-06-28 is a Sunday → week starts Mon 2026-06-22.
        let ref_date = NaiveDate::from_ymd_opt(2026, 6, 28).unwrap();
        let w = window_dates("week", ref_date);
        assert_eq!(w.cs, NaiveDate::from_ymd_opt(2026, 6, 22).unwrap());
        assert_eq!(w.ce, NaiveDate::from_ymd_opt(2026, 6, 28).unwrap());
        assert_eq!(w.ps, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
    }

    #[test]
    fn month_window_handles_december() {
        let ref_date = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        let w = window_dates("month", ref_date);
        assert_eq!(w.cs, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(w.ce, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        // previous month = November, even though naive month+1 would overflow.
        assert_eq!(w.ps, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
    }

    #[test]
    fn delta_handles_zero_baseline() {
        let d = delta(5.0, 0.0);
        assert_eq!(d.pct, None);
        assert_eq!(d.note.as_deref(), Some("基线为0"));
        let d2 = delta(6.0, 2.0);
        assert!((d2.pct.unwrap() - 2.0).abs() < 1e-9); // +200%
    }

    // ── End-to-end: seed a DB, run window_metrics, assert the numbers. ────────

    fn fresh_db() -> crate::db::Db {
        let tmp = tempfile::tempdir().unwrap();
        // `keep()` detaches the tempdir from its cleanup guard so it survives the
        // function return. Leaking a test scratch dir is acceptable.
        let dir = tmp.keep();
        let path = dir.join("test.db");
        let mut db = crate::db::Db::new(&path).unwrap();
        db.init("test-device").unwrap();
        db
    }

    fn seed_session(
        db: &crate::db::Db,
        device_id: &str,
        source: &str,
        sid: &str,
        start_secs: i64,
        model: &str,
        input: i64,
        output: i64,
        cache_read: i64,
    ) {
        let session_pk = format!("{device_id}:{source}:{sid}");
        db.upsert_collected_session(&crate::models::CollectedSession {
            id: session_pk.clone(),
            device_id: device_id.to_string(),
            source: source.to_string(),
            project_id: None,
            agent_id: None,
            start_time: Some(start_secs as u64),
            end_time: Some(start_secs as u64),
            message_count: 5,
            title_or_prompt: Some("test".into()),
            cached_at: 1,
            project_path: Some("proj".into()),
            quality_score: Some(50.0),
        })
        .unwrap();
        db.upsert_collected_token_usage(&crate::models::CollectedTokenUsage {
            id: format!("{session_pk}:{model}"),
            device_id: device_id.to_string(),
            session_id: session_pk,
            source: source.to_string(),
            project_id: None,
            model_id: Some(model.to_string()),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: cache_read,
            total_tokens: input + output + cache_read,
            model_calls: 1,
            tool_calls: 2,
            duration_ms: None,
        })
        .unwrap();
    }

    #[test]
    fn window_metrics_aggregates_and_compares() {
        let db = fresh_db();
        let pricing = crate::pricing::Pricing::from_db(vec![]);
        // "Today" relative to the test: use a fixed date so the test is stable.
        let ref_date = NaiveDate::from_ymd_opt(2026, 6, 29).unwrap();
        let today_secs = day_start_secs(ref_date) + 3600; // 01:00 local
        let yesterday_secs = day_start_secs(ref_date.pred_opt().unwrap()) + 3600;

        // Today: two sessions, deepseek-chat (pay_as_you_go).
        seed_session(&db, "test-device", "claude-code", "a1", today_secs, "deepseek-chat", 1_000_000, 500_000, 0);
        seed_session(&db, "test-device", "claude-code", "a2", today_secs, "deepseek-chat", 500_000, 250_000, 0);
        // Yesterday: one session (for the 环比 baseline).
        seed_session(&db, "test-device", "claude-code", "b1", yesterday_secs, "deepseek-chat", 800_000, 400_000, 0);

        let m = window_metrics(&db, "day", ref_date, None, &pricing).unwrap();

        // Current window = today: 2 sessions, total = 1.5M input + 0.75M output = 2.25M tokens.
        let cmp_sessions = m.comparison.get("sessions").unwrap();
        assert_eq!(cmp_sessions.current, 2.0);
        assert!(((m.token_dimension.scale.total_tokens as f64) - 2_250_000.0).abs() < 0.5);

        // 环比 vs yesterday (1 session): +100%.
        let mom = cmp_sessions.mom.as_ref().unwrap();
        assert!(mom.pct.unwrap() > 0.0);

        // Cost: deepseek-chat is (2.0 in, 0.5 cache, 8.0 out). fresh=1.5M, out=0.75M.
        // cost = (1.5M*2 + 0.75M*8)/1e6 = 3 + 6 = 9.0 CNY.
        assert!((m.token_dimension.cost.est_cost_cny - 9.0).abs() < 0.01);

        // Leverage: variable (all pay_as_you_go) = 9.0. output_proxy = tool_calls(4) + prompts(0) = 4.
        // leverage = 4 / 9 ≈ 0.44.
        assert!((m.leverage.variable_cost_cny - 9.0).abs() < 0.01);
        assert!((m.leverage.leverage_per_cny - (4.0 / 9.0)).abs() < 0.01);

        // Cache is memoized: a second call returns the same payload instantly.
        let m2 = window_metrics(&db, "day", ref_date, None, &pricing).unwrap();
        assert_eq!(m2.comparison.get("sessions").unwrap().current, 2.0);
    }

    #[test]
    fn window_metrics_source_filter_isolates() {
        let db = fresh_db();
        let pricing = crate::pricing::Pricing::from_db(vec![]);
        let ref_date = NaiveDate::from_ymd_opt(2026, 6, 29).unwrap();
        let today_secs = day_start_secs(ref_date) + 3600;
        seed_session(&db, "test-device", "claude-code", "a1", today_secs, "deepseek-chat", 100, 100, 0);
        seed_session(&db, "test-device", "zcode", "a2", today_secs, "GLM-5.2", 200, 200, 0);

        // No filter: 2 sessions.
        let m_all = window_metrics(&db, "day", ref_date, None, &pricing).unwrap();
        assert_eq!(m_all.comparison.get("sessions").unwrap().current, 2.0);

        // Filter to zcode: 1 session.
        let m_z = window_metrics(&db, "day", ref_date, Some("zcode"), &pricing).unwrap();
        assert_eq!(m_z.comparison.get("sessions").unwrap().current, 1.0);
    }
}
