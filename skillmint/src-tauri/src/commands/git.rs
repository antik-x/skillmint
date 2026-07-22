use super::*;

fn git_available() -> std::result::Result<(), anyhow::Error> {
    let out = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map_err(|e| anyhow::anyhow!("无法调用 git：{e}。请确认系统已安装 Git。"))?;
    if !out.status.success() {
        return Err(anyhow::anyhow!("git --version 返回错误"));
    }
    Ok(())
}

fn run_git(repo: &std::path::Path, args: &[&str]) -> std::result::Result<String, anyhow::Error> {
    git_available()?;
    let out = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("执行 git {:?} 失败：{e}", args))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow::anyhow!("git {:?} 失败：{}", args, stderr));
    }
    Ok(stdout)
}

/// PRD-09 §4: initialize a Git repository in the Center Repo.
#[tauri::command]
pub fn git_init_repo(state: State<'_, AppState>) -> Result<(), String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    git_init_repo_at(&center_repo)
}

pub(crate) fn git_init_repo_at(center_repo: &std::path::Path) -> Result<(), String> {
    if !center_repo.is_dir() {
        return Err("中心仓库不存在".to_string());
    }
    run_git(center_repo, &["init"]).map_err(|e| e.to_string())?;
    // P2-1: default ignores — app-private metadata, OS files, and the
    // dependency/cache dirs that must never be tracked.
    let gitignore = center_repo.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(
            &gitignore,
            ".DS_Store\n*.DS_Store\n.skillmint/\nnode_modules/\n__pycache__/\n",
        );
    }
    Ok(())
}

/// P2-1: top-level directories in `repo` that contain an embedded `.git`
/// (e.g. a vendored plugin marketplace mirror). Staging such a dir with
/// `git add` would create a broken gitlink; these dirs must be gitignored
/// instead (the user can still `git submodule add` deliberately).
fn find_embedded_git_dirs(repo: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(repo) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || path.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            if path.join(".git").exists() {
                found.push(name);
            }
        }
    }
    found.sort();
    found
}

/// P2-1: append `/<name>/` ignore rules for embedded-git dirs (idempotent).
/// Returns the names that are now covered.
fn gitignore_embedded_dirs(repo: &std::path::Path, names: &[String]) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    let gitignore = repo.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let mut additions = Vec::new();
    for name in names {
        let rule = format!("/{name}/");
        let covered = existing
            .lines()
            .any(|l| l.trim() == rule || l.trim() == format!("{name}/"));
        if !covered {
            additions.push(rule);
        }
    }
    if !additions.is_empty() {
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for rule in &additions {
            content.push_str(rule);
            content.push('\n');
        }
        let _ = std::fs::write(&gitignore, content);
    }
    names.to_vec()
}

/// P2-1: shared staging core. Embedded-git top-level dirs are gitignored and
/// excluded from staging (never turned into gitlinks); everything else is
/// staged with `git add -A` and committed. Returns the ignored dir names.
pub(crate) fn git_stage_and_commit(
    repo: &std::path::Path,
    message: &str,
) -> Result<Vec<String>, String> {
    let embedded = find_embedded_git_dirs(repo);
    let ignored = gitignore_embedded_dirs(repo, &embedded);
    if !ignored.is_empty() {
        eprintln!(
            "[git] skipped embedded git repos (added to .gitignore): {}",
            ignored.join(", ")
        );
    }
    run_git(repo, &["add", "-A"]).map_err(|e| e.to_string())?;
    // Allow empty commits so users can mark a version even with no changes.
    run_git(repo, &["commit", "-m", message, "--allow-empty"]).map_err(|e| e.to_string())?;
    Ok(ignored)
}

/// PRD-09 §4: stage all changes and commit with the given message.
/// Returns the embedded-git dirs that were ignored instead of staged (P2-1).
#[tauri::command]
pub fn git_commit(message: String, state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    if !center_repo.join(".git").is_dir() {
        return Err("中心仓库尚未初始化 Git，请先点击「初始化 Git」".to_string());
    }
    git_stage_and_commit(&center_repo, &message)
}

/// P2-1: after a successful import, auto-commit when the user enabled it.
/// Never initializes a repo on its own; failures are logged, not raised —
/// an import must not fail because of git.
pub(crate) fn maybe_auto_commit_after_import(settings: &Settings, skill_names: &[String]) {
    if !settings.auto_commit_after_import || skill_names.is_empty() {
        return;
    }
    let repo = &settings.center_repo;
    if !repo.join(".git").is_dir() {
        return;
    }
    let message = if skill_names.len() == 1 {
        format!("Import skill {}", skill_names[0])
    } else {
        format!("Import {} skills: {}", skill_names.len(), skill_names.join(", "))
    };
    match git_stage_and_commit(repo, &message) {
        Ok(ignored) => {
            if !ignored.is_empty() {
                eprintln!(
                    "[git] auto-commit ignored embedded repos: {}",
                    ignored.join(", ")
                );
            }
        }
        Err(e) => eprintln!("[git] auto-commit after import failed: {e}"),
    }
}

/// PRD-09 §4: push the current branch to the configured origin.
#[tauri::command]
pub fn git_push(
    remote: Option<String>,
    branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    let remote = remote.unwrap_or_else(|| "origin".to_string());
    let branch = branch.unwrap_or_else(|| "HEAD".to_string());
    run_git(&center_repo, &["push", &remote, &branch]).map_err(|e| e.to_string())?;
    Ok(())
}

/// PRD-09 §4: pull from the configured origin and merge.
#[tauri::command]
pub fn git_pull(
    remote: Option<String>,
    branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    git_pull_inner(&center_repo, remote, branch)
}

/// Internal version of `git_pull` used by the scheduled-task registry.
pub(crate) fn git_pull_inner(
    center_repo: &std::path::Path,
    remote: Option<String>,
    branch: Option<String>,
) -> Result<(), String> {
    let remote = remote.unwrap_or_else(|| "origin".to_string());
    let branch = branch.unwrap_or_else(|| "HEAD".to_string());
    run_git(center_repo, &["pull", &remote, &branch]).map_err(|e| e.to_string())?;
    Ok(())
}

/// PRD-09 §4: quick check whether the Center Repo already has a `.git` dir.
#[tauri::command]
pub fn git_status_inited(state: State<'_, AppState>) -> Result<bool, String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    Ok(center_repo.join(".git").is_dir())
}

/// PRD-09 §4: list the recent Git commits in the Center Repo.
#[tauri::command]
pub fn git_versions(state: State<'_, AppState>) -> Result<Vec<GitCommit>, String> {
    let center_repo = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.center_repo.clone()
    };
    if !center_repo.join(".git").is_dir() {
        return Ok(Vec::new());
    }
    let out = run_git(
        &center_repo,
        &[
            "log",
            "--pretty=format:%H|%s|%an|%at",
            "-n",
            "50",
        ],
    )
    .map_err(|e| e.to_string())?;

    let mut commits = Vec::new();
    for line in out.lines() {
        let parts = line.splitn::<char>(4, '|').collect::<Vec<&str>>();
        if parts.len() != 4 {
            continue;
        }
        let timestamp = parts[3].parse::<u64>().unwrap_or(0);
        commits.push(GitCommit {
            sha: parts[0].to_string(),
            message: parts[1].to_string(),
            author: parts[2].to_string(),
            timestamp,
        });
    }
    Ok(commits)
}

