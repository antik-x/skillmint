//! SPEC-I2: tunable thresholds and constants for the discovery pipeline.
//!
//! All values are named constants so product can tune signal/noise without
//! touching detector logic. Numbers mirror the design doc; raise to reduce
//! false positives, lower to increase recall.

/// How many days back the detectors scan.
pub const DETECTION_WINDOW_DAYS: i64 = 7;

/// Maximum fresh discoveries written per pipeline run. Keeps the inbox small
/// and actionable.
pub const DAILY_DISCOVERY_LIMIT: usize = 5;

/// Pending discoveries older than this are auto-expired.
pub const STALE_DISCOVERY_DAYS: u32 = 7;

/// SPEC-C1 T4: gate-rejection ledger rows are pruned after this many days.
pub const GATE_REJECTION_RETENTION_DAYS: u32 = 30;

/// SPEC-C1 T4: confidence band for the "low_confidence" silent inbox (PRD-12
/// §3.1). Candidates in [LOW_CONFIDENCE_MIN, LOW_CONFIDENCE_MAX) that were
/// rejected at the threshold gate remain queryable via list_gate_rejections.
pub const LOW_CONFIDENCE_MIN: f64 = 0.4;
pub const LOW_CONFIDENCE_MAX: f64 = 0.6;

/// A dismissed dedup_key is suppressed for this many days.
///
/// This is the legacy uniform window; SPEC-C1 T3 introduces tiered cooling
/// driven by `reject_reason`. Use [`cooling_days_for_reason`] to pick the
/// right window per candidate.
pub const DISMISS_COOLING_DAYS: u32 = 30;

/// SPEC-C1 T3: cooling windows keyed by reject reason.
//@{
pub const COOLING_DAYS_DUPLICATE: u32 = 90;
pub const COOLING_DAYS_TRIVIAL: u32 = 30; // == DISMISS_COOLING_DAYS
pub const COOLING_DAYS_WRONG: u32 = 14;
//@}

/// Pick the cooling window for a reject reason. `None` falls back to the
/// legacy uniform window (historical dismissed items without an
/// adoption_events row, or unknown reasons).
pub fn cooling_days_for_reason(reason: Option<&str>) -> u32 {
    match reason {
        Some("duplicate") => COOLING_DAYS_DUPLICATE,
        Some("trivial") => COOLING_DAYS_TRIVIAL,
        Some("wrong") => COOLING_DAYS_WRONG,
        _ => DISMISS_COOLING_DAYS,
    }
}

// Repeat-pattern detector -----------------------------------------------------

/// Minimum 3-gram Jaccard similarity to consider two prompts part of the same
/// cluster.
pub const REPEAT_JACCARD_THRESHOLD: f64 = 0.6;

/// Minimum prompts in a cluster before it becomes a discovery.
pub const REPEAT_MIN_PROMPTS: usize = 3;

/// Confidence is `count / REPEAT_CONFIDENCE_DENOMINATOR` capped at 1.0.
pub const REPEAT_CONFIDENCE_DENOMINATOR: f64 = 5.0;

// High-value prompt detector --------------------------------------------------

/// Minimum message count to consider a session "short".
pub const HIGH_VALUE_MAX_MESSAGES: i64 = 8;

/// Minimum token efficiency (output/input) for a high-value signal.
pub const HIGH_VALUE_MIN_TOKEN_EFFICIENCY: f64 = 0.15;

/// Minimum prompt length in characters.
pub const HIGH_VALUE_MIN_PROMPT_LENGTH: usize = 10;

/// Confidence boost when the prompt looks like a one-shot success.
pub const HIGH_VALUE_ONE_SHOT_CONFIDENCE: f64 = 0.75;

// Skill-feedback detector -----------------------------------------------------

/// Minimum sample size on each side of the comparison.
pub const SKILL_FEEDBACK_MIN_SAMPLES: usize = 5;

/// Relative difference threshold to report a skill as effective or needing work.
pub const SKILL_FEEDBACK_DIFF_THRESHOLD: f64 = 0.30;

// Capability-gap detector -----------------------------------------------------

/// Minimum prompts in a category within the window.
pub const CAPABILITY_GAP_MIN_PROMPTS: usize = 5;

/// Weekly report ---------------------------------------------------------------
/// How many days back the weekly report looks.
pub const WEEKLY_REPORT_WINDOW_DAYS: i64 = 7;
