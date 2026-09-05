//! OpenViking context-database integration — P0 read-only probe.
//!
//! See `docs/openviking-integration.md`. OpenViking is an *optional
//! enhancement* (the "外置上下文引擎"): every call is gated behind both
//! `remote_enabled` and `openviking.enabled`, P0 only ever issues read-only
//! GET requests, and any failure degrades to a status label without affecting
//! existing features.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::settings::OpenVikingConfig;

/// How long to wait for each probe request. Kept short so the probe can run
/// on the UI thread path without feeling stuck.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Probe outcome, surfaced verbatim in the settings UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ProbeState {
    /// Gates off (`remote_enabled` or `openviking.enabled` false). Zero requests.
    Disabled,
    /// Server reachable, authenticated, all P0 endpoints present.
    Available,
    /// Server reachable but rejected the Bearer key (401/403).
    AuthFailed,
    /// Server reachable but a P0 endpoint is missing (404) — version drift.
    Incompatible,
    /// Connection refused, DNS failure or timeout.
    Unreachable,
}

/// Everything the settings UI needs to render the integration status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub state: ProbeState,
    /// Human-readable detail (Chinese) for the status line.
    pub detail: String,
    /// Per-endpoint capability check results (only when we got that far).
    #[serde(default)]
    pub capabilities: Vec<EndpointCheck>,
}

/// One checked endpoint: path (+query) and whether it answered 2xx.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointCheck {
    pub path: String,
    pub ok: bool,
}

/// Gate check + probe. Returns `Disabled` without touching the network when
/// either switch is off — the local-first guarantee.
pub fn probe_gated(remote_enabled: bool, cfg: &OpenVikingConfig) -> ProbeResult {
    if !remote_enabled || !cfg.enabled {
        return ProbeResult {
            state: ProbeState::Disabled,
            detail: "未启用：需要在「远程功能」与「OpenViking」两处开关同时打开".to_string(),
            capabilities: Vec::new(),
        };
    }
    probe(&cfg.base_url, &cfg.api_key)
}

/// First request doubles as reachability + authentication; the rest check the
/// P0 read-only endpoints. `/relations` requires a `uri` query param — the
/// `viking://` root is always valid (verified against v0.4.10).
pub fn probe(base_url: &str, api_key: &str) -> ProbeResult {
    let base = base_url.trim_end_matches('/');
    let client = match reqwest::blocking::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeResult {
                state: ProbeState::Unreachable,
                detail: format!("HTTP 客户端构建失败：{e}"),
                capabilities: Vec::new(),
            }
        }
    };

    // First request doubles as reachability + authentication check.
    let auth_url = format!("{}/api/v1/console/dashboard/summary", base);
    let resp = client
        .get(&auth_url)
        .bearer_auth(api_key)
        .send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return ProbeResult {
                state: ProbeState::Unreachable,
                detail: format!("无法连接 {base}（服务未启动或超时）：{e}"),
                capabilities: Vec::new(),
            }
        }
    };
    match resp.status() {
        s if s == reqwest::StatusCode::UNAUTHORIZED || s == reqwest::StatusCode::FORBIDDEN => {
            return ProbeResult {
                state: ProbeState::AuthFailed,
                detail: "认证失败：API key 无效或缺失".to_string(),
                capabilities: Vec::new(),
            }
        }
        s if !s.is_success() => {
            return ProbeResult {
                state: ProbeState::Incompatible,
                detail: format!("服务返回异常状态 {}", s.as_u16()),
                capabilities: Vec::new(),
            }
        }
        _ => {}
    }

    // Auth OK: check the remaining P0 read-only endpoints.
    let mut capabilities = vec![EndpointCheck {
        path: "/api/v1/console/dashboard/summary".to_string(),
        ok: true,
    }];
    let mut missing = 0;
    for path in ["/api/v1/skills", "/api/v1/relations?uri=viking://"] {
        let ok = match client
            .get(format!("{base}{path}"))
            .bearer_auth(api_key)
            .send()
        {
            Ok(r) => {
                let s = r.status();
                // 401/403 on a capability endpoint means the key passed the
                // auth probe but lacks scope for it (e.g. a root/admin key —
                // data endpoints need a user-level key).
                if s == reqwest::StatusCode::UNAUTHORIZED || s == reqwest::StatusCode::FORBIDDEN {
                    return ProbeResult {
                        state: ProbeState::AuthFailed,
                        detail: format!(
                            "key 通过了鉴权探针但无权访问 {path}（{}）：数据接口需要 user 级 API key，而非 root/admin key",
                            s.as_u16()
                        ),
                        capabilities,
                    };
                }
                s.is_success()
            }
            Err(_) => false,
        };
        if !ok {
            missing += 1;
        }
        capabilities.push(EndpointCheck {
            path: path.to_string(),
            ok,
        });
    }

    if missing > 0 {
        return ProbeResult {
            state: ProbeState::Incompatible,
            detail: format!("服务版本可能不兼容：{missing} 个 P0 只读接口不可用"),
            capabilities,
        };
    }
    ProbeResult {
        state: ProbeState::Available,
        detail: "可用：连接、鉴权与 P0 只读接口全部正常".to_string(),
        capabilities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_off_sends_nothing_and_reports_disabled() {
        let cfg = OpenVikingConfig::default();
        let r = probe_gated(false, &cfg);
        assert_eq!(r.state, ProbeState::Disabled);
        assert!(r.capabilities.is_empty());

        let cfg_on = OpenVikingConfig {
            enabled: true,
            ..Default::default()
        };
        let r = probe_gated(false, &cfg_on);
        assert_eq!(r.state, ProbeState::Disabled);

        // remote_enabled=true alone is still not enough.
        let r = probe_gated(true, &cfg);
        assert_eq!(r.state, ProbeState::Disabled);
    }

    #[test]
    fn probe_unreachable_when_nothing_listens() {
        // Port 1 on localhost is (practically) never listening; keeps the test
        // hermetic without a mock server.
        let r = probe("http://127.0.0.1:1", "k");
        assert_eq!(r.state, ProbeState::Unreachable);
    }

    #[test]
    fn probe_auth_failure_against_local_reject() {
        // A tiny TCP listener that always answers 401 keeps this hermetic.
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let r = probe(&format!("http://{addr}"), "bad-key");
        assert_eq!(r.state, ProbeState::AuthFailed);
        handle.join().expect("join");
    }
}
