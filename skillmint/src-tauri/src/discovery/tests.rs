//! SPEC-I2: discovery pipeline unit tests.

use chrono::Datelike;

use crate::db::{new_id, Db, now_secs};
use crate::discovery::{
    capability_gap::CapabilityGapDetector, high_value_prompt::HighValuePromptDetector,
    repeat_pattern::RepeatPatternDetector, skill_feedback::SkillFeedbackDetector, DetectContext,
    Detector, DiscoveryKind, DiscoveryStatus,
};
use crate::models::{Skill, SkillStatus};
use crate::settings::AiConfig;

const DEVICE: &str = "discovery-test-device";

fn test_db() -> (tempfile::TempDir, Db) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let mut db = Db::new(&db_path).unwrap();
    db.init(DEVICE).unwrap();
    (tmp, db)
}

fn insert_prompt(db: &Db, id: &str, session_id: &str, text: &str, started_at: i64) {
    db.conn_ref()
        .execute(
            "INSERT INTO collected_prompts
             (id, device_id, session_id, source, prompt_text, started_at)
             VALUES (?1, ?2, ?3, 'claude-code', ?4, ?5)",
            rusqlite::params![id, DEVICE, session_id, text, started_at],
        )
        .unwrap();
}

fn insert_session(db: &Db, id: &str, message_count: i64, start_time: i64) {
    db.conn_ref()
        .execute(
            "INSERT INTO collected_sessions
             (id, device_id, source, message_count, start_time)
             VALUES (?1, ?2, 'claude-code', ?3, ?4)",
            rusqlite::params![id, DEVICE, message_count, start_time],
        )
        .unwrap();
}

fn insert_token_usage(db: &Db, session_id: &str, input_tokens: i64, output_tokens: i64) {
    db.conn_ref()
        .execute(
            "INSERT INTO collected_token_usage
             (id, device_id, session_id, source, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, 'claude-code', ?4, ?5)",
            rusqlite::params![new_id(), DEVICE, session_id, input_tokens, output_tokens],
        )
        .unwrap();
}

fn insert_skill_attribution(
    db: &Db,
    session_id: &str,
    skill_id: &str,
    skill_name: &str,
    attributed_at: i64,
) {
    db.conn_ref()
        .execute(
            "INSERT INTO skill_usage_attributions
             (id, device_id, skill_id, skill_name, source, session_id, attribution_type, attributed_at)
             VALUES (?1, ?2, ?3, ?4, 'claude-code', ?5, 'explicit', ?6)",
            rusqlite::params![new_id(), DEVICE, skill_id, skill_name, session_id, attributed_at],
        )
        .unwrap();
}

fn insert_skill(db: &Db, name: &str, repo_root: &std::path::Path, content: &str) -> Skill {
    let skill_dir = repo_root.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();

    let now = now_secs();
    let skill = Skill {
        id: new_id(),
        name: name.to_string(),
        repo_path: skill_dir,
        created_at: now,
        updated_at: now,
        status: SkillStatus::Draft,
    };
    db.insert_skill(&skill).unwrap();
    skill
}

fn ctx(db: &Db, start: i64, end: i64) -> DetectContext<'_> {
    DetectContext {
        db,
        device_id: DEVICE,
        window_start: start,
        window_end: end,
    }
}

#[test]
fn repeat_pattern_detects_cluster_of_three() {
    let (_tmp, db) = test_db();
    let now = now_secs() as i64;
    let base = now - 86400;
    let text = "How do I refactor this auth module cleanly?";
    for i in 0..3 {
        insert_prompt(
            &db,
            &format!("rp-prompt-{i}"),
            &format!("rp-session-{i}"),
            text,
            base + i,
        );
    }

    let detector = RepeatPatternDetector;
    let candidates = detector.detect(&ctx(&db, now - 7 * 86400, now)).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, DiscoveryKind::RepeatPattern);
    assert_eq!(candidates[0].payload["count"].as_u64(), Some(3));
    assert!((candidates[0].confidence - 0.6).abs() < f64::EPSILON);
}

#[test]
fn high_value_prompt_detects_one_shot_success() {
    let (_tmp, db) = test_db();
    let now = now_secs() as i64;
    let session_id = "hvp-session";
    insert_session(&db, session_id, 5, now - 3600);
    insert_prompt(
        &db,
        "hvp-prompt",
        session_id,
        "Generate a unit test for the payment validator",
        now - 3600,
    );
    insert_token_usage(&db, session_id, 100, 50);

    let detector = HighValuePromptDetector;
    let candidates = detector.detect(&ctx(&db, now - 7 * 86400, now)).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, DiscoveryKind::HighValuePrompt);
}

#[test]
fn skill_feedback_detects_positive_effect() {
    let (_tmp, db) = test_db();
    let repo_root = _tmp.path().join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();
    let skill = insert_skill(&db, "auth-skill", &repo_root, "# Auth Skill\n");

    let now = now_secs() as i64;
    // 5 sessions with the skill (low message count).
    for i in 0..5 {
        let sid = format!("with-{i}");
        insert_session(&db, &sid, 2, now - 86400 - i);
        insert_skill_attribution(&db, &sid, &skill.id, &skill.name, now - 86400 - i);
    }
    // 5 sessions without the skill (high message count).
    for i in 0..5 {
        let sid = format!("without-{i}");
        insert_session(&db, &sid, 20, now - 86400 - i);
    }

    let detector = SkillFeedbackDetector;
    let candidates = detector.detect(&ctx(&db, now - 7 * 86400, now)).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, DiscoveryKind::SkillFeedback);
    assert!(candidates[0].title.contains("auth-skill"));
}

#[test]
fn capability_gap_detects_missing_category() {
    let (_tmp, db) = test_db();
    let repo_root = _tmp.path().join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();
    // No skill covers debugging.
    let now = now_secs() as i64;
    for i in 0..5 {
        insert_prompt(
            &db,
            &format!("cg-prompt-{i}"),
            &format!("cg-session-{i}"),
            "debug this crash and fix the exception",
            now - 86400 - i,
        );
    }

    let detector = CapabilityGapDetector;
    let candidates = detector.detect(&ctx(&db, now - 7 * 86400, now)).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].payload["category"].as_str(), Some("debug"));
}

#[test]
fn capability_gap_skips_covered_category() {
    let (_tmp, db) = test_db();
    let repo_root = _tmp.path().join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();
    // A skill whose name/description covers debugging.
    insert_skill(&db, "debugger", &repo_root, "# Debugger\nHelps with debug and bug fixing.");

    let now = now_secs() as i64;
    for i in 0..5 {
        insert_prompt(
            &db,
            &format!("cg-prompt-{i}"),
            &format!("cg-session-{i}"),
            "debug this crash and fix the exception",
            now - 86400 - i,
        );
    }

    let detector = CapabilityGapDetector;
    let candidates = detector.detect(&ctx(&db, now - 7 * 86400, now)).unwrap();
    assert!(candidates.is_empty());
}

#[test]
fn run_pipeline_limits_to_five() {
    let (_tmp, mut db) = test_db();
    let now = now_secs() as i64;
    // Produce 6 distinct repeat-pattern clusters (3 prompts each) so the
    // pipeline caps at 5 on the first run.
    let clusters = [
        "alpha cluster one one one",
        "beta cluster two two two",
        "gamma cluster three three three",
        "delta cluster four four four",
        "epsilon cluster five five five",
        "zeta cluster six six six",
    ];
    for (idx, text) in clusters.iter().enumerate() {
        for i in 0..3 {
            insert_prompt(
                &db,
                &format!("rp-limit-{idx}-{i}"),
                &format!("rp-limit-s-{idx}-{i}"),
                text,
                now - 86400 - i as i64,
            );
        }
    }

    let cfg = AiConfig::default();
    let result = crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert_eq!(result.inserted, 5);
    let pending = db.list_discoveries(Some("pending")).unwrap();
    assert_eq!(pending.len(), 5);
}

#[test]
fn run_pipeline_is_idempotent() {
    let (_tmp, mut db) = test_db();
    let now = now_secs() as i64;
    // Three distinct repeat-pattern clusters, all fit within the daily limit.
    let clusters = [
        "alpha cluster one one one",
        "beta cluster two two two",
        "gamma cluster three three three",
    ];
    for (idx, text) in clusters.iter().enumerate() {
        for i in 0..3 {
            insert_prompt(
                &db,
                &format!("rp-idem-{idx}-{i}"),
                &format!("rp-idem-s-{idx}-{i}"),
                text,
                now - 86400 - i as i64,
            );
        }
    }

    let cfg = AiConfig::default();
    let result1 = crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert_eq!(result1.inserted, 3);

    let result2 = crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert_eq!(result2.inserted, 0);
    let pending = db.list_discoveries(Some("pending")).unwrap();
    assert_eq!(pending.len(), 3);
}

#[test]
fn dismissed_dedup_is_cooled() {
    let (_tmp, mut db) = test_db();
    let now = now_secs() as i64;
    for i in 0..5 {
        insert_prompt(
            &db,
            &format!("cool-p-{i}"),
            &format!("cool-s-{i}"),
            "debug this crash",
            now - 86400 - i,
        );
    }

    let cfg = AiConfig::default();
    crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    let discovery = db.list_discoveries(Some("pending")).unwrap().pop().unwrap();

    crate::discovery::decide_discovery(&db, DEVICE, _tmp.path(), &discovery.id, "dismiss", None).unwrap();

    // Re-running with the same data must not recreate the dismissed discovery.
    let result = crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert_eq!(result.inserted, 0);
}

#[test]
fn accept_failure_rolls_back_to_pending() {
    let (_tmp, mut db) = test_db();
    let repo_root = _tmp.path().join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();

    // Seed a discovery with a draft_skill payload that would collide with an existing dir.
    let discovery = crate::models::Discovery {
        id: new_id(),
        kind: DiscoveryKind::CapabilityGap,
        title: "能力缺口：调试 5 次".to_string(),
        payload: serde_json::json!({
            "draft_skill": { "name": "existing-skill", "body": "# Draft" }
        }),
        confidence: 0.5,
        dedup_key: "cg:test".to_string(),
        status: DiscoveryStatus::Pending,
        created_at: now_secs(),
        decided_at: None,
        resulting_skill_id: None,
    };
    db.insert_discoveries(std::slice::from_ref(&discovery)).unwrap();

    // Create a file at the skill path so create_dir_all fails.
    let blocking_path = repo_root.join("existing-skill");
    std::fs::write(&blocking_path, "block").unwrap();

    let result = crate::discovery::decide_discovery(&db, DEVICE, &repo_root, &discovery.id, "accept", None);
    assert!(result.is_err());

    let after = db.get_discovery_by_id(&discovery.id).unwrap().unwrap();
    assert_eq!(after.status, DiscoveryStatus::Pending);
}

#[test]
fn pipeline_runs_without_llm_config() {
    let (_tmp, mut db) = test_db();
    let now = now_secs() as i64;
    for i in 0..5 {
        insert_prompt(
            &db,
            &format!("fallback-p-{i}"),
            &format!("fallback-s-{i}"),
            "debug this crash",
            now - 86400 - i,
        );
    }

    let cfg = AiConfig::default();
    let result = crate::discovery::run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert!(result.inserted > 0);
}

#[test]
fn this_monday_utc_is_monday() {
    let monday = crate::discovery::this_monday_utc();
    let parsed = chrono::NaiveDate::parse_from_str(&monday, "%Y-%m-%d").unwrap();
    assert_eq!(parsed.weekday(), chrono::Weekday::Mon);
}

// =============================================================================
// SPEC-C1: three-way decision contract + tiered cooling + gate ledger
// =============================================================================

use crate::discovery::{decide_discovery, run_pipeline};
use crate::models::Discovery;

fn insert_pending_discovery(db: &Db, id: &str, dedup_key: &str, draft_name: Option<&str>) -> Discovery {
    let now = now_secs() as i64;
    let payload = match draft_name {
        Some(name) => serde_json::json!({
            "draft_skill": { "name": name, "body": format!("# {}\n\nDraft body.", name) }
        }),
        None => serde_json::json!({}),
    };
    db.conn_ref()
        .execute(
            "INSERT INTO discoveries (id, kind, title, payload, confidence, dedup_key, status, created_at)
             VALUES (?1, 'high_value_prompt', ?2, ?3, 0.8, ?4, 'pending', ?5)",
            rusqlite::params![id, format!("Discovery {}", id), payload.to_string(), dedup_key, now],
        )
        .unwrap();
    db.get_discovery_by_id(id).unwrap().unwrap()
}

#[test]
fn c1_accept_records_adopted_as_is_and_syncs() {
    let (tmp, db) = test_db();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // No enabled agents => sync_summary is 0/0 (no failure, no success).
    let d = insert_pending_discovery(&db, "c1-accept", "dk:accept", Some("AcceptedSkill"));

    let result = decide_discovery(&db, DEVICE, &repo, &d.id, "accept", None).unwrap();
    assert_eq!(result.discovery.status, DiscoveryStatus::Accepted);
    assert!(result.created_skill_id.is_some());
    let summary = result.sync_summary.expect("accept must return a sync_summary");
    assert_eq!(summary.success, 0);
    assert_eq!(summary.failed, 0);

    // adoption_events ledger row.
    let (decision, reason) = db.latest_adoption_event(&d.id).unwrap().unwrap();
    assert_eq!(decision, "adopted_as_is");
    assert!(reason.is_none());
}

#[test]
fn c1_accept_edited_records_adopted_edited_no_sync() {
    let (tmp, db) = test_db();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let d = insert_pending_discovery(&db, "c1-edit", "dk:edit", Some("EditedSkill"));

    let result = decide_discovery(&db, DEVICE, &repo, &d.id, "accept_edited", None).unwrap();
    assert!(result.created_skill_id.is_some());
    assert!(result.sync_summary.is_none(), "accept_edited must not sync");

    let (decision, _) = db.latest_adoption_event(&d.id).unwrap().unwrap();
    assert_eq!(decision, "adopted_edited");
}

#[test]
fn c1_dismiss_with_reason_records_rejected_and_tier() {
    let (_tmp, db) = test_db();
    let repo = std::path::Path::new("/nonexistent-c1");
    for (reason, expected_days) in [("duplicate", 90u32), ("trivial", 30), ("wrong", 14)] {
        let id = format!("c1-{}", reason);
        let d = insert_pending_discovery(&db, &id, &format!("dk:{}", reason), None);
        decide_discovery(&db, DEVICE, repo, &d.id, "dismiss", Some(reason)).unwrap();

        let (decision, stored_reason) = db.latest_adoption_event(&d.id).unwrap().unwrap();
        assert_eq!(decision, "rejected");
        assert_eq!(stored_reason.as_deref(), Some(reason));

        // The cooling window for this reason should match the tier.
        assert_eq!(crate::discovery::config::cooling_days_for_reason(Some(reason)), expected_days);
    }
}

#[test]
fn c1_dismiss_invalid_reason_errors_without_mutation() {
    let (_tmp, db) = test_db();
    let repo = std::path::Path::new("/nonexistent-c1");
    let d = insert_pending_discovery(&db, "c1-bad", "dk:bad", None);
    let err = decide_discovery(&db, DEVICE, repo, &d.id, "dismiss", Some("bogus")).unwrap_err();
    assert!(err.to_string().contains("invalid dismiss reason"));
    // Discovery stays pending.
    let after = db.get_discovery_by_id(&d.id).unwrap().unwrap();
    assert_eq!(after.status, DiscoveryStatus::Pending);
}

#[test]
fn c1_unknown_action_errors() {
    let (_tmp, db) = test_db();
    let repo = std::path::Path::new("/nonexistent-c1");
    let d = insert_pending_discovery(&db, "c1-unknown", "dk:unknown", None);
    let err = decide_discovery(&db, DEVICE, repo, &d.id, "bogus", None).unwrap_err();
    assert!(err.to_string().contains("unknown action"));
}

#[test]
fn c1_dismiss_default_reason_is_trivial_for_backcompat() {
    let (_tmp, db) = test_db();
    let repo = std::path::Path::new("/nonexistent-c1");
    let d = insert_pending_discovery(&db, "c1-noreason", "dk:noreason", None);
    decide_discovery(&db, DEVICE, repo, &d.id, "dismiss", None).unwrap();
    let (_, reason) = db.latest_adoption_event(&d.id).unwrap().unwrap();
    assert_eq!(reason.as_deref(), Some("trivial"));
}

#[test]
fn c1_gate_rejections_records_cooling_path() {
    // The most reliable end-to-end gate path is "cooling": dismiss a discovery,
    // then re-run the pipeline — the candidate must be recorded as cooling.
    let (_tmp, mut db) = test_db();
    let now = now_secs() as i64;
    // Seed a cluster of 3 identical prompts so the repeat-pattern detector
    // produces exactly one candidate.
    for i in 0..3 {
        insert_prompt(&db, &format!("cool-p-{i}"), &format!("cool-s-{i}"),
            "please refactor this module to use async", now - 86400);
    }
    let cfg = AiConfig::default();
    let r1 = run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert!(r1.inserted >= 1, "first run should insert the candidate");

    // Dismiss it so it enters the cooling window.
    let d = db.list_discoveries(Some("pending")).unwrap().pop().unwrap();
    decide_discovery(&db, DEVICE, _tmp.path(), &d.id, "dismiss", Some("trivial")).unwrap();

    // Re-run: the same dedup_key is now cooling, so it should be recorded as a
    // gate rejection with reason='cooling'.
    let _ = run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    let all = db.list_gate_rejections(None, 1000).unwrap();
    assert!(
        all.iter().any(|r| r.reason == "cooling"),
        "expected a cooling gate rejection, got {:?}",
        all.iter().map(|r| &r.reason).collect::<Vec<_>>()
    );
}

#[test]
fn c1_gate_rejections_records_below_threshold_directly() {
    // The below_threshold and daily_limit paths are exercised by the pipeline's
    // sort/truncate steps; those depend on detector signal which is fragile in
    // unit tests. Here we verify the DB recording primitive directly so the
    // contract is covered regardless of detector output.
    let (_tmp, db) = test_db();
    db.insert_gate_rejection(
        "below_threshold",
        "high_value_prompt",
        0.2,
        &serde_json::json!({"title": "low signal", "dedup_key": "dk:low"}),
    )
    .unwrap();
    let bt = db.list_gate_rejections(Some("below_threshold"), 10).unwrap();
    assert_eq!(bt.len(), 1);
    assert_eq!(bt[0].kind, "high_value_prompt");
    assert!((bt[0].confidence - 0.2).abs() < 1e-9);
}

#[test]
fn c1_list_gate_rejections_filters_by_reason() {
    let (_tmp, db) = test_db();
    db.insert_gate_rejection("below_threshold", "high_value_prompt", 0.2, &serde_json::json!({"k": "a"})).unwrap();
    db.insert_gate_rejection("daily_limit", "repeat_pattern", 0.9, &serde_json::json!({"k": "b"})).unwrap();
    let bt = db.list_gate_rejections(Some("below_threshold"), 10).unwrap();
    assert_eq!(bt.len(), 1);
    assert_eq!(bt[0].reason, "below_threshold");
    let dl = db.list_gate_rejections(Some("daily_limit"), 10).unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0].reason, "daily_limit");
}

#[test]
fn c1_prune_gate_rejections_removes_old_rows() {
    let (_tmp, db) = test_db();
    db.insert_gate_rejection("cooling", "repeat_pattern", 0.5, &serde_json::json!({})).unwrap();
    // Manually backdate one row to 40 days ago.
    let old_ts = (now_secs() as i64) - 40 * 86400;
    db.conn_ref()
        .execute(
            "INSERT INTO gate_rejections (reason, kind, confidence, payload, created_at) VALUES ('cooling', 'x', 0.5, '{}', ?1)",
            rusqlite::params![old_ts],
        )
        .unwrap();
    let pruned = db.prune_gate_rejections(30).unwrap();
    assert_eq!(pruned, 1);
    let remaining = db.list_gate_rejections(None, 100).unwrap();
    assert_eq!(remaining.len(), 1);
}

// =============================================================================
// SPEC-C1 T2: adopt-and-sync direct path — all-success / partial-failure
// =============================================================================

use crate::models::{Agent, SyncMode, SyncTarget};
use std::path::PathBuf;

fn insert_agent_at(db: &Db, name: &str, skill_dir: PathBuf) -> Agent {
    std::fs::create_dir_all(&skill_dir).unwrap();
    let agent = Agent {
        id: new_id(),
        name: name.to_string(),
        skill_directory: skill_dir,
        is_enabled: true,
        discovery_rule: None,
        description: None,
        source: None,
    };
    db.insert_agent(&agent).unwrap();
    agent
}

#[test]
fn c2_accept_syncs_to_all_enabled_agents_success() {
    // Two enabled agents with writable directories: accept should sync to both.
    let (tmp, db) = test_db();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let _a1 = insert_agent_at(&db, "agent-ok-1", tmp.path().join("a1").join("skills"));
    let _a2 = insert_agent_at(&db, "agent-ok-2", tmp.path().join("a2").join("skills"));

    let d = insert_pending_discovery(&db, "c2-ok", "dk:ok", Some("SyncableSkill"));
    let result = decide_discovery(&db, DEVICE, &repo, &d.id, "accept", None).unwrap();
    let summary = result.sync_summary.expect("accept returns sync_summary");
    assert_eq!(summary.success, 2, "both agents should succeed");
    assert_eq!(summary.failed, 0);
    // Files actually landed.
    assert!(tmp.path().join("a1").join("skills").join("SyncableSkill").join("SKILL.md").exists());
    assert!(tmp.path().join("a2").join("skills").join("SyncableSkill").join("SKILL.md").exists());
}

#[test]
fn c2_accept_partial_failure_keeps_skill_and_reports_failed() {
    // One writable agent + one whose directory is read-only (chmod 555 on the
    // parent so the symlink/copy cannot land). The skill must still be created
    // and the discovery accepted; sync_summary.failed must be >= 1.
    let (tmp, db) = test_db();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let _a1 = insert_agent_at(&db, "agent-ok", tmp.path().join("ok").join("skills"));

    // Create a read-only agent directory: the skill subdir can't be written.
    let ro_root = tmp.path().join("ro");
    let ro_skills = ro_root.join("skills");
    std::fs::create_dir_all(&ro_skills).unwrap();
    let _a2 = insert_agent_at(&db, "agent-ro", ro_skills.clone());
    // Lock the skills dir to read+execute only (no write). On macOS this blocks
    // both symlink and copy creation inside it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro_skills, std::fs::Permissions::from_mode(0o555)).unwrap();
    }

    let d = insert_pending_discovery(&db, "c2-pf", "dk:pf", Some("PartialFailSkill"));
    let result = decide_discovery(&db, DEVICE, &repo, &d.id, "accept", None).unwrap();

    // PRD-12 A-R1: skill was created regardless of sync outcome.
    assert!(result.created_skill_id.is_some());
    assert_eq!(result.discovery.status, DiscoveryStatus::Accepted);

    let summary = result.sync_summary.expect("accept returns sync_summary");
    assert!(summary.failed >= 1, "expected at least one sync failure, got {summary:?}");
    // At least one agent succeeded.
    assert!(summary.success >= 1, "expected the writable agent to succeed");
    // Failure carries a recovery hint.
    assert!(summary.failures.iter().any(|f| f.agent_id == _a2.id), "failure must name the RO agent");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Restore permissions so the temp dir cleanup works.
        let _ = std::fs::set_permissions(&ro_skills, std::fs::Permissions::from_mode(0o755));
    }
}

// =============================================================================
// SPEC-C1 T3: tiered cooling — boundary behavior (suppressed vs regenerated)
// =============================================================================

#[test]
fn c3_cooling_suppresses_before_window_and_regenerates_after() {
    // Dismiss a candidate as 'wrong' (14-day cooling). Within the window the
    // pipeline must record a 'cooling' gate rejection; after we backdate the
    // dismissal past 14 days the candidate must regenerate.
    let (tmp, mut db) = test_db();
    let now = now_secs() as i64;
    // Seed an identical 3-prompt cluster so the repeat detector fires.
    for i in 0..3 {
        insert_prompt(&db, &format!("bound-p-{i}"), &format!("bound-s-{i}"),
            "optimize this query for postgres indexing performance", now - 86400);
    }
    let cfg = AiConfig::default();
    let r1 = run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    assert!(r1.inserted >= 1);

    let d = db.list_discoveries(Some("pending")).unwrap().pop().unwrap();
    decide_discovery(&db, DEVICE, tmp.path(), &d.id, "dismiss", Some("wrong")).unwrap();

    // Within the 14-day window: re-run, candidate should be cooling-suppressed.
    let _ = run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    let cooling_rows = db.list_gate_rejections(Some("cooling"), 100).unwrap();
    assert!(!cooling_rows.is_empty(), "expected a cooling gate rejection within the window");
    // No new pending discovery with the same dedup_key.
    let pending = db.list_discoveries(Some("pending")).unwrap();
    assert!(pending.iter().all(|p| p.dedup_key != d.dedup_key),
        "candidate should be suppressed within cooling window");

    // Backdate the dismissal to 15 days ago (past the 14-day 'wrong' window).
    let backdated = now - 15 * 86400;
    db.conn_ref()
        .execute(
            "UPDATE discoveries SET decided_at = ?1 WHERE id = ?2",
            rusqlite::params![backdated, d.id],
        )
        .unwrap();
    // Also backdate the adoption_events row so latest_reject_reason_for_dedup still sees it,
    // but the is_dedup_cooling check should now pass (decided_at < cutoff).
    db.conn_ref()
        .execute(
            "UPDATE adoption_events SET decided_at = ?1 WHERE discovery_id = ?2",
            rusqlite::params![backdated, d.id],
        )
        .unwrap();

    // After the window: the candidate can regenerate.
    let before_count = db.list_discoveries(Some("pending")).unwrap().len();
    let _ = run_pipeline(&mut db, DEVICE, &cfg).unwrap();
    let after = db.list_discoveries(Some("pending")).unwrap();
    assert!(
        after.iter().any(|p| p.dedup_key == d.dedup_key) || after.len() > before_count,
        "candidate should regenerate after the cooling window expires"
    );
}

#[test]
fn c3_cooling_duplicate_window_longer_than_wrong() {
    // Sanity: a 'duplicate' dismissal (90d) is still cooling at 20 days, while
    // a 'wrong' dismissal (14d) is not.
    let (_tmp, db) = test_db();
    let now = now_secs() as i64;
    let twenty_days_ago = now - 20 * 86400;

    // duplicate dismissal, 20 days ago — still within 90d.
    db.conn_ref()
        .execute(
            "INSERT INTO discoveries (id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at)
             VALUES ('dup-id', 'high_value_prompt', 'dup', '{}', 0.8, 'dk:dup', 'dismissed', ?1, ?1)",
            rusqlite::params![twenty_days_ago],
        )
        .unwrap();
    db.conn_ref()
        .execute(
            "INSERT INTO adoption_events (discovery_id, decision, reject_reason, decided_at)
             VALUES ('dup-id', 'rejected', 'duplicate', ?1)",
            rusqlite::params![twenty_days_ago],
        )
        .unwrap();
    let reason = db.latest_reject_reason_for_dedup("dk:dup").unwrap();
    assert_eq!(reason.as_deref(), Some("duplicate"));
    let days = crate::discovery::config::cooling_days_for_reason(reason.as_deref());
    assert_eq!(days, 90);
    assert!(db.is_dedup_cooling("dk:dup", days).unwrap(),
        "duplicate at 20 days should still be cooling (90d window)");

    // wrong dismissal, 20 days ago — past 14d.
    db.conn_ref()
        .execute(
            "INSERT INTO discoveries (id, kind, title, payload, confidence, dedup_key, status, created_at, decided_at)
             VALUES ('wrong-id', 'high_value_prompt', 'wrong', '{}', 0.8, 'dk:wrong', 'dismissed', ?1, ?1)",
            rusqlite::params![twenty_days_ago],
        )
        .unwrap();
    db.conn_ref()
        .execute(
            "INSERT INTO adoption_events (discovery_id, decision, reject_reason, decided_at)
             VALUES ('wrong-id', 'rejected', 'wrong', ?1)",
            rusqlite::params![twenty_days_ago],
        )
        .unwrap();
    let days_wrong = crate::discovery::config::cooling_days_for_reason(Some("wrong"));
    assert_eq!(days_wrong, 14);
    assert!(!db.is_dedup_cooling("dk:wrong", days_wrong).unwrap(),
        "wrong at 20 days should be past the 14d cooling window");
}
