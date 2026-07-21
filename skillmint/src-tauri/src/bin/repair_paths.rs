//! SPEC-F5 T5 real-machine verification harness, extended for P1-5.
//!
//! Runs the SAME logic as the `repair_skill_paths` Tauri command against the
//! live DB + `~/.skillmint/` filesystem:
//! - legacy flat-layout migration (`migrate_skills_to_repo`)
//! - drift detection (`scan_repair_drift`): rename pairing, orphan discovery,
//!   legacy leftover links
//!
//! Default is DRY-RUN: prints the report and changes nothing.
//! Pass `--apply` to execute the migration + drift repair.
//!
//! Usage: `cargo run --bin repair_paths [-- --apply]`

fn main() {
    let apply = std::env::args().any(|a| a == "--apply");

    let app_dir = dirs::home_dir()
        .expect("no home dir")
        .join("Library/Application Support/com.skillmint");
    let settings = skillmint_lib::settings::Settings::load_or_default(&app_dir)
        .expect("failed to load settings");
    let db_path = app_dir.join("skillmint.db");
    let mut db = skillmint_lib::db::Db::new(&db_path).expect("failed to open db");
    db.init(&settings.device_id).expect("db init failed");

    println!("center_repo: {:?}", settings.center_repo);
    println!("db: {}", db_path.display());
    println!("mode: {}", if apply { "APPLY" } else { "DRY-RUN (pass --apply to execute)" });

    // --- P1-5 drift scan (always read-only) ---
    let drift = skillmint_lib::commands::scan_repair_drift(&db, &settings)
        .unwrap_or_else(|e| {
            eprintln!("FATAL (drift scan): {e}");
            std::process::exit(1);
        });

    println!("--- drift report ---");
    println!("rename pairs ({}):", drift.rename_pairs.len());
    for p in &drift.rename_pairs {
        println!("  - {} -> {} [{}]", p.old_name, p.new_name, p.evidence);
    }
    println!("rename unresolved ({}):", drift.rename_unresolved.len());
    for f in &drift.rename_unresolved {
        println!("  - {}: {}", f.skill, f.reason);
    }
    println!("orphans ({}):", drift.orphans.len());
    for o in &drift.orphans {
        println!("  - {} ({})", o.name, o.path);
    }
    println!("legacy dangling links ({}):", drift.legacy_links.len());
    for l in &drift.legacy_links {
        println!("  - {} -> {}", l.path, l.target);
    }

    if !apply {
        println!("--- dry-run: no changes made ---");
        return;
    }

    // --- apply: legacy migration + drift repair ---
    let report =
        skillmint_lib::commands::migrate_skills_to_repo(&db, &settings.center_repo).unwrap_or_else(|e| {
            eprintln!("FATAL (migration): {e}");
            std::process::exit(1);
        });

    println!("--- repair_skill_paths report ---");
    println!("migrated ({}): {:?}", report.migrated.len(), report.migrated);
    println!("failed   ({}):", report.failed.len());
    for f in &report.failed {
        println!("  - {}: {}", f.skill, f.reason);
    }

    let summary = skillmint_lib::commands::apply_repair_drift(&db, &settings, &drift)
        .unwrap_or_else(|e| {
            eprintln!("FATAL (drift apply): {e}");
            std::process::exit(1);
        });

    println!("--- drift apply summary ---");
    println!("renamed ({}): {:?}", summary.renamed.len(), summary.renamed);
    println!(
        "orphans registered ({}): {:?}",
        summary.orphans_registered.len(),
        summary.orphans_registered
    );
    println!(
        "legacy links removed ({}): {:?}",
        summary.legacy_links_removed.len(),
        summary.legacy_links_removed
    );
    println!("failed ({}):", summary.failed.len());
    for f in &summary.failed {
        println!("  - {}: {}", f.skill, f.reason);
    }
}
