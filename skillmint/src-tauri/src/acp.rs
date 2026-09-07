//! 标准 ACP（Agent Client Protocol, agentclientprotocol.com）客户端。
//!
//! SkillMint 作为 ACP client，通过 stdio 上的换行分隔 JSON-RPC 2.0 驱动本机
//! Agent CLI：Kimi Code 原生支持（`kimi acp`）；Claude Code / Codex 经 Zed
//! 官方适配器（`npx -y @zed-industries/claude-code-acp` / `codex-acp`）。
//!
//! 每次调用走完整握手：`initialize` → `session/new`（cwd 固定为哨兵目录，
//! 见 origin.rs）→ `session/prompt`，期间消费 `session/update` 通知累积
//! agent 文本，并对反向请求 `session/request_permission` 自动选择拒绝项
//! （分析任务是纯文本进出，不声明 fs/terminal 能力，也不该碰工具）。
//!
//! 失败分类与冷却（Q7 决议）：认证/额度类失败冷却 30 分钟，启动失败 10 分钟，
//! 超时/协议 2 分钟——避免每次调用都先撞已知的墙；连接列表顺序即优先级，
//! 首个成功者胜出。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use crate::settings::AcpConnectionConfig;
use crate::origin;

/// initialize 阶段超时。适配器经 npx 冷启动（首跑含下载）可能较慢。
const INIT_TIMEOUT: Duration = Duration::from_secs(60);
/// session/prompt 整体超时（分类/摘要单批足够；超时后发 session/cancel 并杀进程）。
const PROMPT_TIMEOUT: Duration = Duration::from_secs(240);
/// 协议版本（ACP v1，实测 claude-code-acp 返回 1）。
const PROTOCOL_VERSION: i64 = 1;

/// 支持 stdio 的连接数上限内单连接失败分类。
#[derive(Debug, Clone)]
pub enum AcpError {
    /// Agent 未登录（如 `kimi acp --login`）。冷却 30 分钟。
    AuthRequired(String),
    /// 额度耗尽 / 限流。冷却 30 分钟。
    Quota(String),
    /// 启动失败（命令不存在、PATH 缺失等）。冷却 10 分钟。
    Spawn(String),
    /// 响应超时。冷却 2 分钟。
    Timeout(String),
    /// 协议不符 / 意外响应。冷却 2 分钟。
    Protocol(String),
}

impl AcpError {
    fn from_rpc(code: i64, message: &str) -> Self {
        let m = message.to_string();
        let lower = m.to_lowercase();
        // ACP 约定 auth_required = -32000；消息兜底匹配登录类文案。
        if code == -32000
            || lower.contains("auth")
            || m.contains("登录")
            || lower.contains("login")
        {
            return AcpError::AuthRequired(m);
        }
        if lower.contains("quota")
            || lower.contains("credit")
            || lower.contains("usage limit")
            || lower.contains("rate limit")
            || lower.contains("429")
            || m.contains("额度")
            || m.contains("余额")
        {
            return AcpError::Quota(m);
        }
        AcpError::Protocol(format!("RPC error {code}: {m}"))
    }

    /// 冷却时长（秒）。冷却期内 route_chat 跳过该连接。
    pub fn cooldown_secs(&self) -> u64 {
        match self {
            AcpError::AuthRequired(_) | AcpError::Quota(_) => 30 * 60,
            AcpError::Spawn(_) => 10 * 60,
            AcpError::Timeout(_) | AcpError::Protocol(_) => 2 * 60,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            AcpError::AuthRequired(_) => "需要登录",
            AcpError::Quota(_) => "额度/限流",
            AcpError::Spawn(_) => "启动失败",
            AcpError::Timeout(_) => "响应超时",
            AcpError::Protocol(_) => "协议错误",
        }
    }
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcpError::AuthRequired(m) => write!(f, "Agent 未登录：{m}"),
            AcpError::Quota(m) => write!(f, "额度/限流：{m}"),
            AcpError::Spawn(m) => write!(f, "启动失败：{m}"),
            AcpError::Timeout(m) => write!(f, "响应超时：{m}"),
            AcpError::Protocol(m) => write!(f, "协议错误：{m}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 连接健康状态（Q7：排序 + 失败降级 + 冷却 + UI 状态灯）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct ConnHealth {
    /// "ok" | "cooldown" | "error"
    state: String,
    detail: String,
    /// 冷却截止（epoch 秒）；空 = 不在冷却。
    cooldown_until: Option<u64>,
}

fn health_map() -> &'static Mutex<HashMap<String, ConnHealth>> {
    static MAP: OnceLock<Mutex<HashMap<String, ConnHealth>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn record_health(id: &str, result: Result<(), &AcpError>) {
    let mut map = match health_map().lock() {
        Ok(m) => m,
        Err(_) => return,
    };
    match result {
        Ok(()) => {
            map.insert(id.to_string(), ConnHealth { state: "ok".into(), detail: String::new(), cooldown_until: None });
        }
        Err(e) => {
            map.insert(
                id.to_string(),
                ConnHealth {
                    state: "cooldown".into(),
                    detail: format!("{}：{}", e.kind_label(), e),
                    cooldown_until: Some(now_secs() + e.cooldown_secs()),
                },
            );
        }
    }
}

/// UI 状态灯数据（get_acp_health 命令返回）。
#[derive(Debug, Clone, Serialize)]
pub struct AcpHealthEntry {
    pub id: String,
    /// "ok" | "cooldown" | "idle"（从未调用）
    pub state: String,
    pub detail: String,
    /// 冷却剩余秒数（0 = 不在冷却）。
    pub cooldown_remaining: u64,
}

pub fn health_snapshot(connections: &[AcpConnectionConfig]) -> Vec<AcpHealthEntry> {
    let map = health_map().lock().ok();
    let now = now_secs();
    connections
        .iter()
        .map(|c| {
            let entry = map.as_ref().and_then(|m| m.get(&c.id)).cloned();
            match entry {
                Some(h) => {
                    let remaining = h.cooldown_until.map(|u| u.saturating_sub(now)).unwrap_or(0);
                    let state = if remaining == 0 && h.state == "cooldown" { "error".to_string() } else { h.state };
                    AcpHealthEntry { id: c.id.clone(), state, detail: h.detail, cooldown_remaining: remaining }
                }
                None => AcpHealthEntry { id: c.id.clone(), state: "idle".into(), detail: String::new(), cooldown_remaining: 0 },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// stdio 连接：spawn + 读线程 + 带超时的请求/通知分发
// ---------------------------------------------------------------------------

struct StdioConn {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    /// 累积的 agent 文本（session/update agent_message_chunk）。
    agent_text: String,
    stderr_tail: Arc<Mutex<String>>,
}

impl StdioConn {
    fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self, AcpError> {
        let resolved = resolve_command(command)
            .ok_or_else(|| AcpError::Spawn(format!("命令 {command} 不存在（PATH 中未找到）")))?;
        let mut cmd = Command::new(resolved);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 分析会话一律落在哨兵工作目录（origin.rs 防线 1）。
        let workspace = origin::acp_workspace_dir();
        let _ = std::fs::create_dir_all(&workspace);
        cmd.current_dir(&workspace);
        // GUI 启动时 PATH 受限：注入增强 PATH，保证适配器能找到 node/claude。
        if let Some(p) = augmented_path() {
            cmd.env("PATH", p);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| AcpError::Spawn(format!("{command}: {e}")))?;
        let stdin = child.stdin.take().ok_or_else(|| AcpError::Spawn("stdin 不可用".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| AcpError::Spawn("stdout 不可用".into()))?;
        let stderr_tail: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        // 读线程：逐行转发给主循环；stdout 只准写 ACP 消息，但容错跳过非 JSON 行。
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // stderr 收尾线程：截尾部 2KB，用于错误报告。
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                    if t.len() > 2048 {
                        t.clear();
                    }
                    t.push_str(&line);
                    t.push('\n');
                }
            });
        }

        Ok(Self { child, stdin, rx, next_id: 1, agent_text: String::new(), stderr_tail })
    }

    fn send(&mut self, value: &Value) -> Result<(), AcpError> {
        let mut line = serde_json::to_string(value)
            .map_err(|e| AcpError::Protocol(format!("序列化失败: {e}")))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| AcpError::Protocol(format!("写入 agent stdin 失败（进程可能已退出）: {e}")))
    }

    fn next_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// 发送请求并等待其响应；期间处理通知与反向请求。deadline 到点发
    /// session/cancel 再宽限 5 秒，仍无响应则杀进程报超时。
    fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, AcpError> {
        let id = self.next_id();
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let mut cancelled = false;
        loop {
            let now = Instant::now();
            if now >= deadline && !cancelled {
                // 通知 + 宽限，给 agent 体面收尾的机会。
                let _ = self.send(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{}}));
                cancelled = true;
            }
            let wait = if cancelled {
                Duration::from_secs(5)
            } else {
                deadline.saturating_duration_since(now)
            };
            let line = match self.rx.recv_timeout(wait) {
                Ok(l) => l,
                Err(_) if cancelled => {
                    let stderr = self.stderr_tail.lock().map(|t| t.clone()).unwrap_or_default();
                    return Err(AcpError::Timeout(format!("{method} 超时且 agent 无响应。stderr: {}", truncate_tail(&stderr))));
                }
                Err(_) => continue,
            };
            let msg: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue, // 非 JSON 行（日志串扰）忽略
            };

            if msg.get("method").is_some() {
                let resp_id = msg.get("id").cloned();
                let m = msg["method"].as_str().unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                match resp_id {
                    Some(rid) => self.handle_reverse_request(&m, params, rid)?,
                    None => self.handle_notification(&m, params),
                }
                continue;
            }

            // 响应：按 id 匹配。
            if msg.get("id").and_then(|v| v.as_i64()) == Some(id) {
                if let Some(err) = msg.get("error") {
                    let code = err["code"].as_i64().unwrap_or(0);
                    let message = err["message"].as_str().unwrap_or("unknown error");
                    return Err(AcpError::from_rpc(code, message));
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            // 其它 id 的响应（不该出现）忽略。
        }
    }

    /// 反向请求：权限请求自动选拒绝项；其余能力请求一律 method-not-found
    ///（initialize 未声明 fs/terminal，正常不应出现）。
    fn handle_reverse_request(
        &mut self,
        method: &str,
        params: Value,
        rid: Value,
    ) -> Result<(), AcpError> {
        let result = if method == "session/request_permission" {
            permission_reject_outcome(&params)
        } else {
            return self.send(&json!({
                "jsonrpc":"2.0","id":rid,
                "error":{"code":-32601,"message":format!("SkillMint 不支持 {method}（分析任务无需该能力）")}
            }));
        };
        self.send(&json!({"jsonrpc":"2.0","id":rid,"result":result}))
    }

    fn handle_notification(&mut self, method: &str, params: Value) {
        if method != "session/update" {
            return;
        }
        let update = &params["update"];
        if update["sessionUpdate"].as_str() == Some("agent_message_chunk") {
            if let Some(text) = content_block_text(&update["content"]) {
                self.agent_text.push_str(&text);
            }
        }
        // agent_thought_chunk / tool_call / plan 等对分析无用，忽略。
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn truncate_tail(s: &str) -> String {
    let t = s.trim();
    if t.len() <= 400 {
        t.to_string()
    } else {
        t[t.len() - 400..].to_string()
    }
}

/// 从 params.options 里挑拒绝项；没有就 cancelled。
fn permission_reject_outcome(params: &Value) -> Value {
    let options = params["options"].as_array();
    if let Some(opts) = options {
        for want in ["reject_once", "reject_always", "reject"] {
            if let Some(opt) = opts.iter().find(|o| o["kind"].as_str() == Some(want)) {
                if let Some(option_id) = opt["optionId"].as_str() {
                    return json!({"outcome": {"outcome": "selected", "optionId": option_id}});
                }
            }
        }
    }
    json!({"outcome": {"outcome": "cancelled"}})
}

/// 提取 ContentBlock（{type:"text",text:...}）中的文本；非文本块返回 None。
fn content_block_text(block: &Value) -> Option<String> {
    if block["type"].as_str() == Some("text") {
        return block["text"].as_str().map(String::from);
    }
    None
}

// ---------------------------------------------------------------------------
// PATH 增强（GUI 启动时进程 PATH 受限；复用 npx.rs 的探测思路）
// ---------------------------------------------------------------------------

/// 候选 bin 目录：用户常见安装位 + nvm + Homebrew + 当前 PATH。
fn candidate_dirs() -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let mut push = |p: std::path::PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(p) = std::env::var("PATH") {
        for d in std::env::split_paths(&p) {
            push(d);
        }
    }
    if let Some(home) = dirs::home_dir() {
        push(home.join(".local/bin"));
        push(home.join("Library/pnpm"));
        push(home.join(".kimi-code/bin"));
        let nvm_dir = std::env::var("NVM_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| home.join(".nvm"));
        let mut versions: Vec<std::path::PathBuf> = std::fs::read_dir(nvm_dir.join("versions/node"))
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for v in versions {
            push(v.join("bin"));
        }
    }
    for prefix in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        push(std::path::PathBuf::from(prefix));
    }
    out
}

fn augmented_path() -> Option<String> {
    let dirs = candidate_dirs();
    std::env::join_paths(dirs.iter().filter(|d| d.is_dir())).ok().map(|c| c.to_string_lossy().to_string())
}

fn resolve_command(command: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(command);
    if p.components().count() > 1 {
        // 含路径分隔符：直接用。
        return if p.is_file() { Some(p.to_path_buf()) } else { None };
    }
    candidate_dirs()
        .into_iter()
        .map(|d| d.join(command))
        .find(|c| c.is_file())
}

// ---------------------------------------------------------------------------
// ACP 会话流程
// ---------------------------------------------------------------------------

/// 一次 chat 的结果：连接名 + agent 全文回复。
pub struct AcpReply {
    pub connection: String,
    pub text: String,
}

/// 多连接路由的结果（llm.rs 消费；attempts 进审计）。
pub enum AcpRouteResult {
    Succeeded { connection: String, reply: String, attempts: Vec<String> },
    Failed { attempts: Vec<String> },
}

/// Supported transport for talking to a local agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// 已弃用（P5）：从未实现，保留类型仅为旧 settings.json 反序列化兼容。
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

impl Default for AcpTransport {
    fn default() -> Self {
        AcpTransport::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }
}

fn spawn_failed_hint(command: &str) -> String {
    format!("{command}（请确认 CLI 已安装并在 PATH 中；适配器连接需要 Node ≥ 22）")
}

/// Agent 的登录提示（Q7：UI 与错误信息展示）。
pub fn login_hint(command: &str) -> &'static str {
    if command.contains("kimi") {
        "请在终端运行 `kimi acp --login` 完成 Kimi 登录"
    } else if command.contains("claude") {
        "请在终端运行 `claude` 并完成登录（适配器复用 Claude Code 的登录态）"
    } else if command.contains("codex") {
        "请先完成 Codex CLI 登录（`codex login`）"
    } else {
        "请先在该 CLI 的终端里完成登录"
    }
}

fn chat_stdio(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    system: &str,
    user: &str,
) -> Result<String, AcpError> {
    if command.trim().is_empty() {
        return Err(AcpError::Spawn("命令为空".into()));
    }
    let mut conn = StdioConn::spawn(command, args, env)?;

    let result = (|| -> Result<String, AcpError> {
        // 1. initialize（不声明 fs/terminal 能力——分析任务纯文本进出）。
        let init = conn.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {},
                "clientInfo": {"name": "skillmint", "version": env!("CARGO_PKG_VERSION")},
            }),
            Instant::now() + INIT_TIMEOUT,
        )?;
        let pv = init["protocolVersion"].as_i64().unwrap_or(PROTOCOL_VERSION);
        if pv > PROTOCOL_VERSION {
            return Err(AcpError::Protocol(format!("agent 要求 ACP v{pv}，本客户端支持 v{PROTOCOL_VERSION}")));
        }

        // 2. session/new（cwd = 哨兵目录）。
        let workspace = origin::acp_workspace_dir();
        let new_session = conn.request(
            "session/new",
            json!({"cwd": workspace.to_string_lossy(), "mcpServers": []}),
            Instant::now() + INIT_TIMEOUT,
        )?;
        let session_id = new_session["sessionId"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("session/new 未返回 sessionId".into()))?
            .to_string();

        // 3. session/prompt（system 与 user 合并为单条文本——ACP 无独立 system 字段；
        //    系统提示首行带内容哨兵，采集端据此兜底打标）。
        let prompt_text = format!("{}\n\n{}", system, user);
        let stop = conn.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": prompt_text}],
            }),
            Instant::now() + PROMPT_TIMEOUT,
        )?;
        let stop_reason = stop["stopReason"].as_str().unwrap_or("end_turn");
        let text = conn.agent_text.trim().to_string();
        match stop_reason {
            "end_turn" | "max_tokens" | "max_turn_requests" => {
                if text.is_empty() {
                    Err(AcpError::Protocol("agent 结束但未返回任何文本".into()))
                } else {
                    Ok(text)
                }
            }
            "refusal" => Err(AcpError::Protocol("agent 拒绝执行该任务".into())),
            other => Err(AcpError::Protocol(format!("异常结束: {other}"))),
        }
    })();

    match result {
        Ok(text) => {
            conn.kill();
            Ok(text)
        }
        Err(e) => {
            conn.kill();
            Err(e)
        }
    }
}

/// 连接探测（测试连接按钮）：initialize + session/new，不发 prompt、零 token 消耗。
fn probe_stdio(command: &str, args: &[String], env: &HashMap<String, String>) -> Result<String, AcpError> {
    if command.trim().is_empty() {
        return Err(AcpError::Spawn("命令为空".into()));
    }
    let mut conn = StdioConn::spawn(command, args, env)?;
    let result = (|| -> Result<String, AcpError> {
        let init = conn.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {},
                "clientInfo": {"name": "skillmint", "version": env!("CARGO_PKG_VERSION")},
            }),
            Instant::now() + INIT_TIMEOUT,
        )?;
        let agent = &init["agentInfo"];
        let agent_label = format!(
            "{} {}",
            agent["title"].as_str().or_else(|| agent["name"].as_str()).unwrap_or("agent"),
            agent["version"].as_str().unwrap_or("")
        );
        let workspace = origin::acp_workspace_dir();
        conn.request(
            "session/new",
            json!({"cwd": workspace.to_string_lossy(), "mcpServers": []}),
            Instant::now() + INIT_TIMEOUT,
        )?;
        Ok(format!("握手成功：{agent_label}（initialize + session/new 通过，未消耗额度）"))
    })();
    match result {
        Ok(msg) => {
            conn.kill();
            Ok(msg)
        }
        Err(e) => {
            conn.kill();
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// 公共 API（llm.rs / commands 消费）
// ---------------------------------------------------------------------------

/// 按连接顺序尝试启用的 ACP 连接（冷却中的跳过），首个成功者胜出。
/// attempts 摘要（含失败原因）供审计日志与 strict 模式报错使用。
pub fn route_chat(
    connections: &[AcpConnectionConfig],
    system: &str,
    user: &str,
) -> AcpRouteResult {
    let mut attempts: Vec<String> = Vec::new();
    for conn in connections.iter().filter(|c| c.enabled) {
        // 冷却检查：认证/额度类失败后的冷静期内不再撞墙。
        if let Ok(map) = health_map().lock() {
            if let Some(h) = map.get(&conn.id) {
                if let Some(until) = h.cooldown_until {
                    if now_secs() < until {
                        attempts.push(format!("{}：冷却中（{}）", conn.name, h.detail));
                        continue;
                    }
                }
            }
        }

        let (command, args, env) = match &conn.transport {
            AcpTransport::Stdio { command, args, env } => (command.clone(), args.clone(), env.clone()),
            AcpTransport::Sse { .. } => {
                attempts.push(format!("{}：SSE 传输已弃用，请改用 stdio", conn.name));
                continue;
            }
        };
        match chat_stdio(&command, &args, &env, system, user) {
            Ok(text) => {
                record_health(&conn.id, Ok(()));
                return AcpRouteResult::Succeeded { connection: conn.name.clone(), reply: text, attempts };
            }
            Err(e) => {
                record_health(&conn.id, Err(&e));
                let hint = if matches!(e, AcpError::AuthRequired(_)) {
                    format!("（{}）", login_hint(&command))
                } else if matches!(e, AcpError::Spawn(_)) {
                    format!("（{}）", spawn_failed_hint(&command))
                } else {
                    String::new()
                };
                attempts.push(format!("{}：{}{}", conn.name, e, hint));
            }
        }
    }
    AcpRouteResult::Failed { attempts }
}

/// 测试某条 ACP 连接（真实 initialize + session/new 握手，零额度消耗）。
/// 结果同样记入健康状态（UI 状态灯即时反映）。
pub fn test_transport(connections: &[AcpConnectionConfig], id: &str) -> Result<String> {
    let conn = connections
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!("ACP 连接 {id} 不存在"))?;
    let (command, args, env) = match &conn.transport {
        AcpTransport::Stdio { command, args, env } => (command.clone(), args.clone(), env.clone()),
        AcpTransport::Sse { url, .. } => {
            return Err(anyhow!("SSE 传输已弃用（{url}），请改用 stdio 连接"));
        }
    };
    match probe_stdio(&command, &args, &env) {
        Ok(msg) => {
            record_health(&conn.id, Ok(()));
            Ok(msg)
        }
        Err(e) => {
            record_health(&conn.id, Err(&e));
            Err(match e {
                AcpError::AuthRequired(_) => anyhow!("{}。{}", e, login_hint(&command)),
                AcpError::Spawn(_) => anyhow!("{}。{}", e, spawn_failed_hint(&command)),
                other => anyhow!("{other}"),
            })
        }
    }
}

/// P1-1: one candidate agent detected on the local machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedAgent {
    pub command: String,
    pub display_name: String,
    pub args: Vec<String>,
    /// 登录/前置条件提示（Q7）。
    #[serde(default)]
    pub login_hint: String,
}

/// 扫描本机可用的 ACP agent（P5 更新：Kimi 原生 `kimi acp`；Claude Code /
/// Codex 经 Zed 官方适配器；ZCode 无 ACP 已移除）。
pub fn detect_available_agents() -> Vec<DetectedAgent> {
    let mut found = Vec::new();

    if resolve_command("kimi").is_some() {
        found.push(DetectedAgent {
            command: "kimi".into(),
            display_name: "Kimi Code（原生 ACP）".into(),
            args: vec!["acp".into()],
            login_hint: login_hint("kimi").into(),
        });
    }

    let npx_ready = resolve_command("npx").is_some();
    if npx_ready {
        if resolve_command("claude").is_some() {
            found.push(DetectedAgent {
                command: "npx".into(),
                display_name: "Claude Code（ACP 适配器）".into(),
                args: vec!["-y".into(), "@zed-industries/claude-code-acp@0.16".into()],
                login_hint: login_hint("claude").into(),
            });
        }
        if resolve_command("codex").is_some() {
            found.push(DetectedAgent {
                command: "npx".into(),
                display_name: "OpenAI Codex（ACP 适配器）".into(),
                args: vec!["-y".into(), "@zed-industries/codex-acp@0.16".into()],
                login_hint: login_hint("codex").into(),
            });
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_message_chunk_text() {
        let msg: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"你好"}}}}"#,
        )
        .unwrap();
        let update = &msg["params"]["update"];
        assert_eq!(update["sessionUpdate"].as_str(), Some("agent_message_chunk"));
        assert_eq!(content_block_text(&update["content"]).as_deref(), Some("你好"));
    }

    #[test]
    fn permission_reject_prefers_reject_option() {
        let params: Value = serde_json::from_str(
            r#"{"sessionId":"s","options":[{"optionId":"allow","name":"Allow","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}],"toolCall":{}}"#,
        )
        .unwrap();
        let out = permission_reject_outcome(&params);
        assert_eq!(out["outcome"]["outcome"], "selected");
        assert_eq!(out["outcome"]["optionId"], "no");

        // 没有拒绝项 → cancelled。
        let params2: Value = serde_json::from_str(
            r#"{"sessionId":"s","options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}],"toolCall":{}}"#,
        )
        .unwrap();
        let out2 = permission_reject_outcome(&params2);
        assert_eq!(out2["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn rpc_errors_are_classified() {
        assert!(matches!(AcpError::from_rpc(-32000, "Agent not authenticated"), AcpError::AuthRequired(_)));
        assert!(matches!(AcpError::from_rpc(-32603, "usage limit exceeded"), AcpError::Quota(_)));
        assert!(matches!(AcpError::from_rpc(-32603, "quota exceeded for this account"), AcpError::Quota(_)));
        assert!(matches!(AcpError::from_rpc(-32602, "bad params"), AcpError::Protocol(_)));
        assert_eq!(AcpError::AuthRequired("x".into()).cooldown_secs(), 30 * 60);
        assert_eq!(AcpError::Timeout("x".into()).cooldown_secs(), 2 * 60);
    }

    #[test]
    fn route_skips_disabled_and_cold_connections() {
        let conns = vec![
            AcpConnectionConfig {
                id: "cold".into(),
                name: "cold".into(),
                enabled: true,
                transport: AcpTransport::Stdio { command: "definitely-missing-cmd-xyz".into(), args: vec![], env: HashMap::new() },
            },
            AcpConnectionConfig {
                id: "off".into(),
                name: "off".into(),
                enabled: false,
                transport: AcpTransport::Stdio { command: "echo".into(), args: vec![], env: HashMap::new() },
            },
        ];
        // 先制造冷却。
        record_health("cold", Err(&AcpError::Spawn("missing".into())));
        let result = route_chat(&conns, "sys", "user");
        match result {
            AcpRouteResult::Failed { attempts } => {
                assert_eq!(attempts.len(), 1, "冷却中的连接应被跳过：{attempts:?}");
                assert!(attempts[0].contains("冷却中"));
            }
            _ => panic!("expected failure"),
        }
        // 冷却结束后（强制清除）会尝试 spawn 并失败。
        health_map().lock().unwrap().clear();
        let result2 = route_chat(&conns, "sys", "user");
        match result2 {
            AcpRouteResult::Failed { attempts } => {
                assert_eq!(attempts.len(), 1);
                assert!(attempts[0].contains("启动失败"), "got: {attempts:?}");
            }
            _ => panic!("expected failure"),
        }
    }

    #[test]
    fn health_snapshot_reports_states() {
        let conns = vec![AcpConnectionConfig {
            id: "h1".into(),
            name: "h1".into(),
            enabled: true,
            transport: AcpTransport::Stdio { command: "echo".into(), args: vec![], env: HashMap::new() },
        }];
        health_map().lock().unwrap().remove("h1");
        let snap = health_snapshot(&conns);
        assert_eq!(snap[0].state, "idle");

        record_health("h1", Ok(()));
        let snap = health_snapshot(&conns);
        assert_eq!(snap[0].state, "ok");
    }

    #[test]
    fn detect_agents_use_acp_capable_commands() {
        let agents = detect_available_agents();
        for a in &agents {
            assert!(
                a.command == "kimi" || a.command == "npx",
                "只应检出 ACP 兼容命令，got {}",
                a.command
            );
        }
    }

    /// 真机集成测试（默认忽略）：cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_kimi_handshake() {
        let msg = probe_stdio("kimi", &["acp".to_string()], &HashMap::new()).expect("kimi acp 握手失败");
        println!("{msg}");
    }

    #[test]
    #[ignore]
    fn real_claude_adapter_handshake() {
        let msg = probe_stdio(
            "npx",
            &["-y".to_string(), "@zed-industries/claude-code-acp@0.16".to_string()],
            &HashMap::new(),
        )
        .expect("claude-code-acp 握手失败");
        println!("{msg}");
    }

    /// 真机端到端（默认忽略，消耗极少量额度）：完整 session/prompt 链路。
    #[test]
    #[ignore]
    fn real_kimi_chat() {
        let text = chat_stdio(
            "kimi",
            &["acp".to_string()],
            &HashMap::new(),
            &format!("{}\n你是一个连通性测试助手。", crate::origin::ANALYSIS_SENTINEL),
            "请只回复两个字符：OK",
        )
        .expect("kimi acp chat 失败");
        println!("agent 回复: {text}");
        assert!(!text.is_empty());
    }

    /// 真机端到端（默认忽略，消耗极少量额度）：Claude 适配器完整链路。
    #[test]
    #[ignore]
    fn real_claude_adapter_chat() {
        let text = chat_stdio(
            "npx",
            &["-y".to_string(), "@zed-industries/claude-code-acp@0.16".to_string()],
            &HashMap::new(),
            &format!("{}\n你是一个连通性测试助手。", crate::origin::ANALYSIS_SENTINEL),
            "请只回复两个字符：OK",
        )
        .expect("claude 适配器 chat 失败");
        println!("agent 回复: {text}");
        assert!(!text.is_empty());
    }
}
