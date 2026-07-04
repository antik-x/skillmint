//! ACP (Agent Communication Protocol) client.
//!
//! SkillMint acts as an ACP client that invokes locally-installed terminal agents
//! such as Claude Code CLI or Kimi Code CLI over JSON-RPC 2.0 on stdio (or SSE).
//! This allows prompt classification, daily-summary generation, and future LLM
//! workflows to run against a local agent when no cloud API key is configured.
//!
//! The protocol envelope is intentionally minimal:
//!
//! Request:
//! ```json
//! {"jsonrpc":"2.0","id":1,"method":"agent.chat","params":{"system":"...","messages":[{"role":"user","content":"..."}]}}
//! ```
//!
//! Response:
//! ```json
//! {"jsonrpc":"2.0","id":1,"result":{"content":"..."}}
//! ```

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

pub use crate::settings::AcpConnectionConfig;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpChatParams {
    system: String,
    messages: Vec<AcpMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpRequest {
    jsonrpc: String,
    id: i64,
    method: String,
    params: AcpChatParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpResponse {
    jsonrpc: String,
    id: Option<i64>,
    #[serde(flatten)]
    body: AcpResponseBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AcpResponseBody {
    Result { result: AcpChatResult },
    Error { error: AcpRpcError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpChatResult {
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpRpcError {
    code: i64,
    message: String,
}

/// A lightweight ACP client. Each client is cheap to create; stdio transports
/// spawn a fresh subprocess per `chat` call so the agent lifecycle is simple.
pub struct AcpClient {
    transport: AcpTransport,
    next_id: AtomicI64,
}

impl AcpClient {
    pub fn new(transport: AcpTransport) -> Self {
        Self {
            transport,
            next_id: AtomicI64::new(1),
        }
    }

    /// Send a system+user prompt and return the agent's text reply.
    pub fn chat(&self, system: &str, user: &str) -> Result<Option<String>> {
        match &self.transport {
            AcpTransport::Stdio {
                command,
                args,
                env,
            } => self.chat_stdio(command, args, env, system, user),
            AcpTransport::Sse { url, headers } => self.chat_sse(url, headers, system, user),
        }
    }

    /// Test whether the configured transport can be reached.
    pub fn test(&self) -> Result<String> {
        match &self.transport {
            AcpTransport::Stdio {
                command,
                args,
                env,
            } => self.test_stdio(command, args, env),
            AcpTransport::Sse { url, .. } => Ok(format!("SSE transport configured: {url}")),
        }
    }

    fn chat_stdio(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        system: &str,
        user: &str,
    ) -> Result<Option<String>> {
        if command.trim().is_empty() {
            return Ok(None);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = AcpRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: "agent.chat".to_string(),
            params: AcpChatParams {
                system: system.to_string(),
                messages: vec![AcpMessage {
                    role: "user".to_string(),
                    content: user.to_string(),
                }],
            },
        };
        let request_json = serde_json::to_string(&request)?;

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn ACP agent {command}: {e}"))?;

        // Write the request line and a newline so line-oriented agents can read it.
        if let Some(stdin) = child.stdin.take() {
            let mut stdin = std::io::BufWriter::new(stdin);
            stdin.write_all(request_json.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
        }

        // Read the first JSON-RPC line from stdout.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("agent stdout not available"))?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read_result = reader.read_line(&mut line);

        // Best-effort cleanup; ignore errors.
        let _ = child.kill();
        let _ = child.wait();

        read_result.map_err(|e| anyhow!("failed to read agent stdout: {e}"))?;
        if line.trim().is_empty() {
            return Ok(None);
        }

        let resp: AcpResponse = serde_json::from_str(&line)
            .map_err(|e| anyhow!("invalid ACP response ({e}): {line}"))?;
        match resp.body {
            AcpResponseBody::Result { result } => Ok(Some(result.content)),
            AcpResponseBody::Error { error } => Err(anyhow!("ACP error {}: {}", error.code, error.message)),
        }
    }

    fn test_stdio(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<String> {
        if command.trim().is_empty() {
            return Err(anyhow!("command is empty"));
        }
        let mut cmd = Command::new(command);
        cmd.args(args).envs(env).stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = cmd
            .output()
            .map_err(|e| anyhow!("failed to run {command}: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("{command} exited with {:?}: {stderr}", output.status.code()));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("{command} is available\n{stdout}").trim().to_string())
    }

    fn chat_sse(
        &self,
        _url: &str,
        _headers: &HashMap<String, String>,
        _system: &str,
        _user: &str,
    ) -> Result<Option<String>> {
        // SSE transport is planned but not implemented in this iteration.
        // Return None so callers fall back to cloud LLM.
        Ok(None)
    }
}

/// Try each enabled ACP connection in order and return the first successful reply.
pub fn first_available_chat(
    connections: &[AcpConnectionConfig],
    system: &str,
    user: &str,
) -> Option<String> {
    for conn in connections.iter().filter(|c| c.enabled) {
        let client = AcpClient::new(conn.transport.clone());
        match client.chat(system, user) {
            Ok(Some(reply)) => return Some(reply),
            Ok(None) => continue,
            Err(e) => {
                eprintln!("[acp] connection {} failed: {e}", conn.id);
                continue;
            }
        }
    }
    None
}

/// Test an ACP connection by id. Returns a human-readable status string.
pub fn test_transport(connections: &[AcpConnectionConfig], id: &str) -> Result<String> {
    let conn = connections
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!("ACP connection {} not found", id))?;
    let client = AcpClient::new(conn.transport.clone());
    client.test()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_acp_chat_response() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"content":"hello"}}"#;
        let resp: AcpResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(1));
        match resp.body {
            AcpResponseBody::Result { result } => assert_eq!(result.content, "hello"),
            AcpResponseBody::Error { .. } => panic!("expected result"),
        }
    }

    #[test]
    fn parse_acp_error_response() {
        let json = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"bad request"}}"#;
        let resp: AcpResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(2));
        match resp.body {
            AcpResponseBody::Result { .. } => panic!("expected error"),
            AcpResponseBody::Error { error } => {
                assert_eq!(error.code, -32600);
                assert_eq!(error.message, "bad request");
            }
        }
    }

    #[test]
    fn disabled_connection_returns_none() {
        let connections = vec![AcpConnectionConfig {
            id: "test".to_string(),
            name: "test".to_string(),
            enabled: false,
            transport: AcpTransport::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        }];
        assert_eq!(first_available_chat(&connections, "system", "user"), None);
    }
}

/// P1-1: one candidate agent detected on the local machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedAgent {
    pub command: String,
    pub display_name: String,
    pub args: Vec<String>,
}

/// P1-1: scan PATH for common local Agent CLIs.
pub fn detect_available_agents() -> Vec<DetectedAgent> {
    let candidates: Vec<(&str, &str, &[&str])> = vec![
        ("claude", "Claude Code", &["--mcp"]),
        ("claude-cli", "Claude Code CLI", &["--mcp"]),
        ("kimi", "Kimi Code", &[]),
        ("kimi-code", "Kimi Code", &[]),
        ("codex", "OpenAI Codex CLI", &[]),
        ("zcode", "ZCode", &[]),
    ];

    let mut found = Vec::new();
    for (cmd, name, args) in candidates {
        if which::which(cmd).is_ok() {
            found.push(DetectedAgent {
                command: cmd.to_string(),
                display_name: name.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            });
        }
    }
    found
}

