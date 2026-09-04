//! P3-1: runtime bridge to the `skills` CLI (npm package `skills`,
//! github.com/vercel-labs/skills), invoked as `npx -y <spec> <args...>`.
//!
//! SkillMint is the GUI layer over the CLI (Mole model): every mutation goes
//! through the real binary so the on-disk state (skills-lock.json,
//! ~/.agents/.skill-lock.json, agent dirs) stays the single source of truth.
//! This module owns the node/npx discovery chain, argument building (always
//! non-interactive: explicit `-s`/`-a` + `-y`), China-acceleration env
//! injection, streaming execution, and the machine-readable `ls --json`
//! parser.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Result;

/// Minimum node version required by `skills` v1.5.x (engines field).
pub const MIN_NODE_VERSION: (u64, u64, u64) = (22, 20, 0);

/// Settings snapshot consumed by this module (decoupled from `Settings` so
/// tests can build it without the full settings file).
#[derive(Debug, Clone)]
pub struct NpxConfig {
    /// npm package spec to invoke, e.g. `skills@latest` or a pinned `skills@1.5.23`.
    pub package_spec: String,
    /// Override for the skills.sh search API base URL (China acceleration).
    pub skills_api_url: String,
    /// Proxy URL injected as HTTPS_PROXY/HTTP_PROXY/ALL_PROXY into the child
    /// process. Note: only the git-clone steps of the CLI honor it, but that
    /// covers the actual download path; api.github.com failures fall back to
    /// git clone inside the CLI.
    pub proxy: String,
    /// When true (default), inject DISABLE_TELEMETRY=1 + DO_NOT_TRACK=1.
    pub disable_telemetry: bool,
    /// Optional explicit node bin dir (or path to the node binary). GUI apps
    /// inherit a minimal PATH, so the discovery chain below usually matters.
    pub node_path_override: String,
}

impl Default for NpxConfig {
    fn default() -> Self {
        Self {
            package_spec: "skills@latest".to_string(),
            skills_api_url: String::new(),
            proxy: String::new(),
            disable_telemetry: true,
            node_path_override: String::new(),
        }
    }
}

/// A resolved node installation: the bin dir that holds `node` + `npx`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeEnv {
    pub bin_dir: PathBuf,
    pub node_version: String,
    /// Human-readable hint when detection failed or the version is too old.
    pub problem: Option<String>,
}

impl NodeEnv {
    pub fn ok(&self) -> bool {
        self.problem.is_none()
    }
}

fn parse_node_version(out: &str) -> Option<(u64, u64, u64)> {
    let s = out.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn probe_node(bin_dir: &Path) -> Option<(String, (u64, u64, u64))> {
    let node = bin_dir.join(if cfg!(windows) { "node.exe" } else { "node" });
    if !node.exists() {
        return None;
    }
    let out = std::process::Command::new(&node)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).to_string();
    let parsed = parse_node_version(&v)?;
    Some((format!("v{}", parsed.0), parsed))
}

fn version_tuple_to_string(t: (u64, u64, u64)) -> String {
    format!("v{}.{}.{}", t.0, t.1, t.2)
}

/// Collect candidate node bin dirs in priority order:
/// 1. user override (dir, or a binary path whose parent is used)
/// 2. current PATH (via `which node`)
/// 3. nvm installs (highest version wins)
/// 4. Homebrew / system prefixes
fn candidate_bin_dirs(node_path_override: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.as_os_str().is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };

    let ov = node_path_override.trim();
    if !ov.is_empty() {
        let p = PathBuf::from(ov);
        if p.is_file() {
            if let Some(parent) = p.parent() {
                push(parent.to_path_buf());
            }
        } else if p.is_dir() {
            push(p);
        }
    }
    if let Ok(node) = which::which("node") {
        if let Some(parent) = node.parent() {
            push(parent.to_path_buf());
        }
    }
    if let Some(home) = dirs::home_dir() {
        // nvm: pick the highest installed version.
        let nvm_dir = std::env::var("NVM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".nvm"));
        let mut versions: Vec<PathBuf> = std::fs::read_dir(nvm_dir.join("versions/node"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for v in versions {
            push(v.join("bin"));
        }
    }
    for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
        push(PathBuf::from(prefix));
    }
    out
}

/// Resolve the node runtime, probing each candidate until one satisfies the
/// minimum version. An explicit `node_path_override` is authoritative: when
/// set, only that location is probed (no silent fallback). Returns a `NodeEnv`
/// with `problem` set when nothing usable was found (the frontend shows an
/// install guide instead of an npm error).
pub fn detect_node_env(config: &NpxConfig) -> NodeEnv {
    let override_set = !config.node_path_override.trim().is_empty();
    let mut best: Option<NodeEnv> = None;
    for dir in candidate_bin_dirs(&config.node_path_override) {
        if let Some((vstr, parsed)) = probe_node(&dir) {
            if parsed >= MIN_NODE_VERSION {
                return NodeEnv {
                    bin_dir: dir.clone(),
                    node_version: vstr,
                    problem: None,
                };
            }
            let problem = format!(
                "node {} 过旧，skills CLI 需要 ≥ {}",
                version_tuple_to_string(parsed),
                version_tuple_to_string(MIN_NODE_VERSION)
            );
            // Keep the first (highest-priority) too-old candidate as reported env.
            if best.is_none() {
                best = Some(NodeEnv {
                    bin_dir: dir,
                    node_version: version_tuple_to_string(parsed),
                    problem: Some(problem),
                });
            }
        } else if override_set {
            return NodeEnv {
                bin_dir: dir.clone(),
                node_version: String::new(),
                problem: Some(format!(
                    "设置的 node 路径 {} 下没有可用的 node 可执行文件",
                    dir.display()
                )),
            };
        }
    }
    best.unwrap_or(NodeEnv {
        bin_dir: PathBuf::new(),
        node_version: String::new(),
        problem: Some(
            "未找到 Node.js。请安装 Node ≥ 22.20（推荐 nvm：`nvm install 22 && nvm use 22`），\
             或在设置中手动指定 node 路径。"
                .to_string(),
        ),
    })
}

/// Everything needed to invoke the CLI once.
#[derive(Debug, Clone)]
pub struct SkillsInvocation {
    /// Subcommand + args, e.g. `["add", "vercel-labs/skills", "-s", "pdf", ...]`.
    pub args: Vec<String>,
    /// Project scope runs with cwd = project root (so skills-lock.json lands
    /// there); global scope runs with cwd = $HOME.
    pub project_root: Option<PathBuf>,
    /// Wall-clock guard so a hung install cannot freeze the UI forever.
    pub timeout_secs: u64,
}

/// Build the argv for `skills add`.
pub fn build_add_args(
    source: &str,
    skills: &[String],
    agents: &[String],
    global: bool,
    copy: bool,
) -> Vec<String> {
    let mut args = vec!["add".to_string(), source.to_string()];
    for s in skills {
        args.push("-s".into());
        args.push(s.clone());
    }
    for a in agents {
        args.push("-a".into());
        args.push(a.clone());
    }
    if global {
        args.push("-g".into());
    }
    if copy {
        args.push("--copy".into());
    }
    args.push("-y".into());
    args
}

/// Build the argv for `skills remove`.
pub fn build_remove_args(skill: &str, agents: &[String], global: bool) -> Vec<String> {
    let mut args = vec!["remove".to_string(), skill.to_string()];
    for a in agents {
        args.push("-a".into());
        args.push(a.clone());
    }
    if global {
        args.push("-g".into());
    }
    args.push("-y".into());
    args
}

/// Build the argv for `skills update` (whole scope; per-skill via `skill`).
pub fn build_update_args(skill: Option<&str>, global: bool) -> Vec<String> {
    let mut args = vec!["update".to_string()];
    if let Some(s) = skill {
        args.push(s.to_string());
    }
    if global {
        args.push("-g".into());
    }
    args.push("-y".into());
    args
}

/// Build the argv for `skills ls --json`.
pub fn build_list_args(global: bool) -> Vec<String> {
    let mut args = vec!["ls".to_string()];
    if global {
        args.push("-g".into());
    }
    args.push("--json".into());
    args
}

/// The exact command a user could paste into a terminal — Mole-style
/// transparency shown in the install panel.
pub fn equivalent_command(config: &NpxConfig, invocation: &SkillsInvocation) -> String {
    let mut cmd = format!("npx -y {}", config.package_spec);
    for a in &invocation.args {
        if a == "-y" {
            continue; // npx already carries -y; the CLI's own -y is implied for humans
        }
        if a.contains(' ') {
            cmd.push_str(&format!(" '{}'", a));
            continue;
        }
        cmd.push(' ');
        cmd.push_str(a);
    }
    cmd
}

fn child_env(config: &NpxConfig, bin_dir: &Path) -> std::collections::HashMap<String, String> {
    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    // The npx shebang is `#!/usr/bin/env node`; the child needs bin_dir on PATH.
    let path_key = if cfg!(windows) { "Path" } else { "PATH" };
    let existing = env.get(path_key).cloned().unwrap_or_default();
    env.insert(
        path_key.to_string(),
        format!(
            "{}:{}",
            bin_dir.display(),
            if existing.is_empty() { "/usr/bin:/bin" } else { &existing }
        ),
    );
    if config.disable_telemetry {
        env.insert("DISABLE_TELEMETRY".into(), "1".into());
        env.insert("DO_NOT_TRACK".into(), "1".into());
    }
    if !config.skills_api_url.trim().is_empty() {
        env.insert("SKILLS_API_URL".into(), config.skills_api_url.trim().to_string());
    }
    let proxy = config.proxy.trim();
    if !proxy.is_empty() {
        for k in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY"] {
            env.insert(k.into(), proxy.to_string());
        }
    }
    env
}

/// Outcome of one CLI run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NpxRunResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub output: String,
    pub command: String,
    pub timed_out: bool,
}

/// Run the skills CLI via npx, streaming merged stdout/stderr lines to
/// `on_line` and returning the full transcript.
pub fn run_skills(
    config: &NpxConfig,
    invocation: &SkillsInvocation,
    mut on_line: impl FnMut(&str),
) -> Result<NpxRunResult> {
    let env = detect_node_env(config);
    if let Some(p) = &env.problem {
        anyhow::bail!("{}", p);
    }
    let npx = env.bin_dir.join(if cfg!(windows) { "npx.cmd" } else { "npx" });

    let mut cmd = std::process::Command::new(&npx);
    cmd.arg("-y").arg(&config.package_spec);
    for a in &invocation.args {
        cmd.arg(a);
    }
    let cwd = invocation
        .project_root
        .clone()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
    std::fs::create_dir_all(&cwd)?;
    cmd.current_dir(&cwd);
    cmd.envs(child_env(config, &env.bin_dir));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let display = equivalent_command(config, invocation);
    on_line(&format!("$ {}", display));

    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("无法启动 npx（{}）：{e}", npx.display()))?;

    // Merge stdout+stderr into one channel so the UI sees a single timeline.
    // Reader threads push lines; the caller's `on_line` callback stays on THIS
    // thread (it may emit Tauri events, which are not Send-friendly).
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut handles = Vec::new();
    let streams: Vec<Box<dyn std::io::Read + Send>> = vec![
        Box::new(child.stdout.take().ok_or_else(|| anyhow::anyhow!("npx stdout 不可读"))?),
        Box::new(child.stderr.take().ok_or_else(|| anyhow::anyhow!("npx stderr 不可读"))?),
    ];
    for stream in streams {
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    let mut transcript: Vec<String> = Vec::new();
    let mut drain = |rx: &std::sync::mpsc::Receiver<String>, on_line: &mut dyn FnMut(&str)| {
        while let Ok(line) = rx.try_recv() {
            on_line(&line);
            transcript.push(line);
        }
    };

    // Poll for exit with a deadline, draining output as it arrives; kill on timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(invocation.timeout_secs);
    let mut timed_out = false;
    let status: std::io::Result<std::process::ExitStatus> = loop {
        drain(&rx, &mut on_line);
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    timed_out = true;
                    let _ = child.kill();
                    // Wait so the zombie is reaped before readers are joined.
                    break child.wait();
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => break Err(e),
        }
    };

    for h in handles {
        let _ = h.join();
    }
    drain(&rx, &mut on_line);
    let output = transcript.join("\n");

    match status {
        Err(e) => Err(anyhow::anyhow!("等待 npx 退出失败：{e}")),
        Ok(st) => Ok(NpxRunResult {
            success: st.success() && !timed_out,
            exit_code: st.code(),
            output,
            command: display,
            timed_out,
        }),
    }
}

/// One entry of `skills ls --json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListedSkill {
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(rename = "sourceType", alias = "source_type", default)]
    pub source_type: Option<String>,
}

/// `skills ls --json` for a scope. Pure parse helper `parse_ls_json` is unit
/// tested against real CLI output.
pub fn list_skills(config: &NpxConfig, global: bool, project_root: Option<PathBuf>) -> Result<Vec<ListedSkill>> {
    let invocation = SkillsInvocation {
        args: build_list_args(global),
        project_root,
        timeout_secs: 120,
    };
    let mut captured: Vec<String> = Vec::new();
    let result = run_skills(config, &invocation, |line| captured.push(line.to_string()))?;
    if !result.success {
        anyhow::bail!("skills ls 失败：{}", captured.join("\n"));
    }
    // The JSON array is the whole stdout; skip the `$ command` echo line.
    let stdout: String = captured
        .into_iter()
        .filter(|l| !l.starts_with("$ "))
        .collect::<Vec<_>>()
        .join("\n");
    parse_ls_json(&stdout)
}

pub fn parse_ls_json(stdout: &str) -> Result<Vec<ListedSkill>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let start = trimmed.find('[').ok_or_else(|| anyhow::anyhow!("skills ls 输出中没有 JSON 数组"))?;
    let parsed: Vec<ListedSkill> = serde_json::from_str(&trimmed[start..])?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NpxConfig {
        NpxConfig {
            skills_api_url: "https://mirror.example.com".into(),
            proxy: "http://127.0.0.1:7890".into(),
            ..Default::default()
        }
    }

    #[test]
    fn add_args_are_explicit_and_non_interactive() {
        let args = build_add_args(
            "owner/repo",
            &["pdf".into(), "docx".into()],
            &["claude-code".into(), "zcode".into()],
            true,
            false,
        );
        assert_eq!(
            args,
            vec!["add", "owner/repo", "-s", "pdf", "-s", "docx", "-a", "claude-code", "-a", "zcode", "-g", "-y"]
        );
    }

    #[test]
    fn copy_flag_and_project_scope_default() {
        let args = build_add_args("./hub", &["a".into()], &["zcode".into()], false, true);
        assert!(args.contains(&"--copy".to_string()));
        assert!(!args.contains(&"-g".to_string()));
        assert_eq!(build_remove_args("x", &["cursor".into()], false).last().unwrap(), "-y");
    }

    #[test]
    fn equivalent_command_is_pasteable() {
        let invocation = SkillsInvocation {
            args: build_add_args("owner/repo", &["pdf".into()], &["claude-code".into()], true, false),
            project_root: None,
            timeout_secs: 60,
        };
        let cmd = equivalent_command(&cfg(), &invocation);
        assert!(cmd.starts_with("npx -y skills@latest add owner/repo -s pdf -a claude-code -g"), "{cmd}");
    }

    #[test]
    fn env_injection_covers_mirror_proxy_and_telemetry() {
        let env = child_env(&cfg(), Path::new("/fake/bin"));
        assert_eq!(env.get("SKILLS_API_URL").unwrap(), "https://mirror.example.com");
        assert_eq!(env.get("HTTPS_PROXY").unwrap(), "http://127.0.0.1:7890");
        assert_eq!(env.get("DISABLE_TELEMETRY").unwrap(), "1");
        assert!(env.get("PATH").unwrap().starts_with("/fake/bin:"));
    }

    #[test]
    fn telemetry_on_by_default_setting_respected() {
        let mut c = cfg();
        c.disable_telemetry = false;
        let env = child_env(&c, Path::new("/fake/bin"));
        assert!(!env.contains_key("DISABLE_TELEMETRY"));
    }

    #[test]
    fn parse_ls_json_handles_real_shape() {
        let raw = r#"[
          {"name":"pdf","path":"/Users/x/.agents/skills/pdf","scope":"global",
           "agents":["Claude Code","ZCode"],"source":"vercel-labs/agent-skills",
           "sourceUrl":"https://github.com/vercel-labs/agent-skills","sourceType":"github"},
          {"name":"local-one","path":"/p/.agents/skills/local-one","scope":"project",
           "agents":["Cursor"],"source":null,"sourceUrl":null,"sourceType":null}
        ]"#;
        let parsed = parse_ls_json(raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].agents.len(), 2);
        assert_eq!(parsed[1].source, None);
        assert_eq!(parsed[0].source_type.as_deref(), Some("github"));
    }

    #[test]
    fn parse_ls_json_empty_and_prefixed_output() {
        assert!(parse_ls_json("").unwrap().is_empty());
        let parsed = parse_ls_json("$ npx skills ls --json\n[]").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn version_parse_and_compare() {
        assert_eq!(parse_node_version("v22.20.1"), Some((22, 20, 1)));
        assert_eq!(parse_node_version("v18.0.0"), Some((18, 0, 0)));
        assert!(parse_node_version("v22.20.1").unwrap() >= MIN_NODE_VERSION);
        assert!(parse_node_version("v22.19.0").unwrap() < MIN_NODE_VERSION);
        assert_eq!(parse_node_version("garbage"), None);
    }

    #[test]
    fn run_skills_reports_missing_node_cleanly() {
        // Override points at an empty dir with no node binary: the runtime must
        // surface the friendly problem, not an npm stack trace.
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.node_path_override = tmp.path().to_string_lossy().to_string();
        let invocation = SkillsInvocation {
            args: build_list_args(false),
            project_root: None,
            timeout_secs: 5,
        };
        let err = run_skills(&c, &invocation, |_| {}).unwrap_err();
        assert!(err.to_string().contains("Node") || err.to_string().contains("node"), "{err}");
    }

    #[test]
    fn run_skills_streams_fake_cli_output() {
        // A fake npx that echoes a line and exits 0 proves the spawn/stream path.
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let node_shim = bin.join("node");
        std::fs::write(&node_shim, "#!/bin/sh\necho v22.20.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&node_shim, std::fs::Permissions::from_mode(0o755)).unwrap();
            let npx_shim = bin.join("npx");
            std::fs::write(&npx_shim, "#!/bin/sh\nshift 2\necho \"ran: $@\"\n").unwrap();
            std::fs::set_permissions(&npx_shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut c = cfg();
        c.node_path_override = bin.to_string_lossy().to_string();
        let inv = SkillsInvocation {
            args: vec!["ls".into()],
            project_root: Some(tmp.path().to_path_buf()),
            timeout_secs: 30,
        };
        let mut lines = Vec::new();
        let res = run_skills(&c, &inv, |l| lines.push(l.to_string())).unwrap();
        assert!(res.success, "{:?}", lines);
        assert!(lines.iter().any(|l| l.contains("ran: ls")), "{lines:?}");
    }
}
