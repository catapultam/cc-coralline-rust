//! Auto-mode server-review state from a CLIProxyAPI gateway.
//!
//! Claude Code's `/status` shows "Auto mode server: Enabled" while a session's
//! auto-mode request is still reviewed by Anthropic's server-side classifier.
//! If a session falls back to its own billed classifier for the rest of the
//! session (fix: restart the session), that's worth a visible warning. This
//! module reads a cache of the gateway's `GET /v1/auto-mode?session=<id>`
//! reply and, when stale, spawns a DETACHED `coralline --auto-mode-refresh
//! <session_id>` child, mirroring proxy_usage.rs so a render never blocks on
//! the network.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::json;
use crate::proxy_usage::{gateway, Gateway};
use crate::Payload;

const CACHE_TTL_SECS: u64 = 10;
const LOCK_STALE_SECS: u64 = 60;
const NET_TIMEOUT: Duration = Duration::from_secs(5);

fn cache_path(coralline_dir: &str, session: &str) -> PathBuf {
    let dir = format!("{coralline_dir}/.cache/auto-mode-native");
    let _ = std::fs::create_dir_all(&dir);
    let safe: String = session
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    PathBuf::from(format!("{dir}/{safe}.json"))
}

fn age_secs(p: &Path) -> Option<u64> {
    let modified = std::fs::metadata(p).ok()?.modified().ok()?;
    SystemTime::now().duration_since(modified).ok().map(|d| d.as_secs())
}

/// Extract the `state` field from a `/v1/auto-mode` JSON reply.
fn parse_state(body: &str) -> Option<String> {
    let j = json::parse(body)?;
    j.path(&["state"]).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// "server" -> the ok segment, "local" -> the warning segment; "off",
/// "unknown", missing, or any other value -> hidden.
fn decide(state: &str) -> Option<&'static str> {
    match state {
        "server" => Some("server"),
        "local" => Some("local"),
        _ => None,
    }
}

/// Fill `p.automode` from the gateway cache for this session. No-op (segment
/// stays hidden) without a gateway, a session id, or a parseable cache.
pub fn enrich(p: &mut Payload, session: &str, coralline_dir: &str) {
    if session.is_empty() || gateway().is_none() {
        return;
    }
    let cache = cache_path(coralline_dir, session);
    if age_secs(&cache).map_or(true, |a| a >= CACHE_TTL_SECS) {
        spawn_refresh(session);
    }
    let Some(body) = std::fs::read_to_string(&cache).ok() else { return };
    let Some(state) = parse_state(&body) else { return };
    p.automode = decide(&state).map(|s| s.to_string());
}

fn spawn_refresh(session: &str) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--auto-mode-refresh").arg(session);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    let _ = cmd.spawn();
}

/// `coralline --auto-mode-refresh <session_id>`: fetch /v1/auto-mode and
/// atomically rewrite the cache. A lock file keeps concurrent renders from
/// stacking requests.
pub fn refresh(session: &str, coralline_dir: &str) {
    let Some(gw) = gateway() else { return };
    let cache = cache_path(coralline_dir, session);
    let lock = cache.with_extension("lock");
    if age_secs(&lock).is_some_and(|a| a < LOCK_STALE_SECS) {
        return;
    }
    if std::fs::write(&lock, b"").is_err() {
        return;
    }
    if let Some(body) = fetch(&gw, session) {
        if json::parse(&body).is_some() {
            let tmp = cache.with_extension("tmp");
            if std::fs::write(&tmp, &body).is_ok() {
                let _ = std::fs::rename(&tmp, &cache);
            }
        }
    }
    let _ = std::fs::remove_file(&lock);
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn fetch(gw: &Gateway, session: &str) -> Option<String> {
    let addr = (gw.host.as_str(), gw.port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, NET_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(NET_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(NET_TIMEOUT)).ok()?;
    // HTTP/1.0 keeps the reply unchunked and closes the connection after it.
    let request = format!(
        "GET {}/v1/auto-mode?session={} HTTP/1.0\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        gw.path_prefix,
        url_encode(session),
        gw.host,
        gw.port,
        gw.token
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.take(1 << 20).read_to_end(&mut raw).ok()?;
    let text = String::from_utf8(raw).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.lines().next()?.split_whitespace().nth(1)?;
    if status != "200" {
        return None;
    }
    Some(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_reads_each_known_value() {
        for s in ["server", "local", "off", "unknown"] {
            let body = format!(
                r#"{{"session":"abc","state":"{s}","since":"2026-10-06T00:00:00Z","updated":""}}"#
            );
            assert_eq!(parse_state(&body).as_deref(), Some(s));
        }
    }

    #[test]
    fn parse_state_handles_missing_or_malformed_input() {
        assert_eq!(parse_state(""), None);
        assert_eq!(parse_state("not json"), None);
        assert_eq!(parse_state(r#"{"session":"abc"}"#), None); // no "state" field
        assert_eq!(parse_state(r#"{"state":123}"#), None); // wrong type
    }

    #[test]
    fn decide_shows_server_and_local_only() {
        assert_eq!(decide("server"), Some("server"));
        assert_eq!(decide("local"), Some("local"));
        assert_eq!(decide("off"), None);
        assert_eq!(decide("unknown"), None);
        assert_eq!(decide("anything-else"), None);
        assert_eq!(decide(""), None);
    }

    #[test]
    fn cache_path_sanitizes_the_session_id() {
        let p = cache_path("C:/home/.claude/coralline", "sess/weird id:1");
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.ends_with("/auto-mode-native/sess_weird_id_1.json"));
    }
}
