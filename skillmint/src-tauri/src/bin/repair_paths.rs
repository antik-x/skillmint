//! SPEC-F5 T5 real-machine verification harness.
//!
//! Runs the SAME `migrate_skills_to_repo` logic that the `repair_skill_paths`
//! Tauri command invokes, against the live DB + `~/.skillmint/` filesystem, and
//! prints a structured report. This produces the "命令输出 + ls 收敛证据" the
//! SPEC acceptance clause requires, without needing to launch the full Tauri
// app and click the button.
//!
//! Usage: `cargo run --bin repair_paths`

fn main() {
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
    println!("--- before ---");
    for s in db.get_skills().unwrap() {
        if s.repo_path.parent() != settings.center_repo.parent() {
            continue;
        }
        println!("  LEGACY  {} -> {}", s.name, s.repo_path.display());
    }

    let report =
        skillmint_lib::commands::migrate_skills_to_repo(&db, &settings.center_repo).unwrap_or_else(|e| {
            eprintln!("FATAL: {e}");
            std::process::exit(1);
        });

    println!("--- repair_skill_paths report ---");
    println!("migrated ({}): {:?}", report.migrated.len(), report.migrated);
    println!("failed   ({}):", report.failed.len());
    for f in &report.failed {
        println!("  - {}: {}", f.skill, f.reason);
    }
}
