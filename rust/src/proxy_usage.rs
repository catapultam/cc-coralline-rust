//! Plan usage from a CLIProxyAPI gateway.
//!
//! Claude Code leaves `rate_limits` out of the status-line payload when it
//! authenticates to a gateway (`ANTHROPIC_BASE_URL` plus a token), so the 5h
//! and 7d segments would stay empty. When that happens and the base URL is a
//! plain-http CLIProxyAPI, the render reads a cache of its
//! `GET /v1/usage?model=<id>` reply (pooled usage across the accounts serving
//! the model). A stale cache spawns a DETACHED `coralline --usage-refresh
//! <model>` child that fetches it, mirroring the git refresh, so the render
//! never blocks on the network.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::json::{self, Json};
use crate::{fmt_num, Payload};

const CACHE_TTL_SECS: u64 = 30;
const LOCK_STALE_SECS: u64 = 60;
const NET_TIMEOUT: Duration = Duration::from_secs(5);

struct Gateway {
    host: String,
    port: u16,
    path_prefix: String,
    token: String,
}

fn gateway() -> Option<Gateway> {
    let base = std::env::var("ANTHROPIC_BASE_URL").ok()?;
    let rest = base.trim().strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (authority.to_string(), 80),
    };
    let token = std::env::var("ANTHROPIC_AUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .ok()
        .filter(|t| !t.trim().is_empty())?;
    if host.is_empty() {
        return None;
    }
    Some(Gateway {
        host,
        port,
        path_prefix: path.to_string(),
        token: token.trim().to_string(),
    })
}

/// "claude-opus-5-5[1m]" -> "claude-opus-5-5"
fn base_model(model_id: &str) -> String {
    model_id.split('[').next().unwrap_or("").trim().to_string()
}

fn cache_path(coralline_dir: &str, model: &str) -> PathBuf {
    let dir = format!("{coralline_dir}/.cache/usage-native");
    let _ = std::fs::create_dir_all(&dir);
    let safe: String = model
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    PathBuf::from(format!("{dir}/{safe}"))
}

fn age_secs(p: &Path) -> Option<u64> {
    let modified = std::fs::metadata(p).ok()?.modified().ok()?;
    SystemTime::now().duration_since(modified).ok().map(|d| d.as_secs())
}

/// Fill the 5h/7d fields from the gateway cache when Claude Code sent none.
pub fn enrich(p: &mut Payload, j: &Json, coralline_dir: &str) {
    if p.fh_pct.is_some() || p.wd_pct.is_some() {
        return;
    }
    if gateway().is_none() {
        return;
    }
    let model = base_model(j.path(&["model", "id"]).and_then(|v| v.as_str()).unwrap_or(""));
    if model.is_empty() {
        return;
    }
    let cache = cache_path(coralline_dir, &model);
    if age_secs(&cache).map_or(true, |a| a >= CACHE_TTL_SECS) {
        spawn_refresh(&model);
    }
    let Some(usage) = std::fs::read_to_string(&cache).ok().and_then(|s| json::parse(&s)) else {
        return;
    };
    let window = |w: &Json| -> (Option<f64>, String) {
        let pct = w.path(&["used_percentage"]).and_then(|v| v.as_f64());
        let rst = w.path(&["resets_at"]).and_then(|v| v.as_f64()).map(fmt_num).unwrap_or_default();
        (pct, rst)
    };
    // 5h is left empty on purpose: the gateway moves a session to another
    // account when its 5h window fills, so a pooled 5h figure says little.
    // The current model's weekly pool feeds burn and limit sampling.
    if let Some(w) = usage.path(&["seven_day"]) {
        let (pct, rst) = window(w);
        p.wd_pct = pct;
        p.wd_pct_raw = pct.map(fmt_num).unwrap_or_default();
        p.wd_rst = rst;
    }
    if let Some(Json::Arr(list)) = usage.path(&["seven_day_by_provider"]) {
        for entry in list {
            let name = entry.path(&["provider"]).and_then(|v| v.as_str()).unwrap_or("");
            if let (false, Some(w)) = (name.is_empty(), entry.path(&["seven_day"])) {
                if let (Some(pct), rst) = window(w) {
                    p.gw_weekly.push((name.to_string(), pct, rst));
                }
            }
        }
    }
}

fn spawn_refresh(model: &str) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--usage-refresh").arg(model);
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

/// `coralline --usage-refresh <model>`: fetch /v1/usage and atomically rewrite
/// the cache. A lock file keeps concurrent renders from stacking requests.
pub fn refresh(model: &str, coralline_dir: &str) {
    let Some(gw) = gateway() else { return };
    let cache = cache_path(coralline_dir, model);
    let lock = cache.with_extension("lock");
    if age_secs(&lock).is_some_and(|a| a < LOCK_STALE_SECS) {
        return;
    }
    if std::fs::write(&lock, b"").is_err() {
        return;
    }
    if let Some(body) = fetch(&gw, model) {
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

fn fetch(gw: &Gateway, model: &str) -> Option<String> {
    let addr = (gw.host.as_str(), gw.port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, NET_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(NET_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(NET_TIMEOUT)).ok()?;
    // HTTP/1.0 keeps the reply unchunked and closes the connection after it.
    let request = format!(
        "GET {}/v1/usage?model={} HTTP/1.0\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        gw.path_prefix,
        url_encode(model),
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
