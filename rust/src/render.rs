//! Segment building + layout, ported faithfully from upstream statusline.sh so
//! output is byte-identical to the bash renderer.
use crate::config::Config;
use crate::git::GitInfo;
use crate::Payload;

const R: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const NORM: &str = "\x1b[22m";

pub(crate) struct Seg {
    pub(crate) bg: String,
    pub(crate) txt: String,
    pub(crate) len: usize,
}

fn color(spec: &str, fgbg: u8) -> String {
    if spec.is_empty() {
        return String::new();
    }
    if spec.contains(',') {
        let mut it = spec.split(',');
        let r = it.next().unwrap_or("0");
        let g = it.next().unwrap_or("0");
        let b = it.next().unwrap_or("0");
        format!("\x1b[{};2;{};{};{}m", fgbg, r, g, b)
    } else {
        format!("\x1b[{};5;{}m", fgbg, spec)
    }
}
pub(crate) fn fg(spec: &str) -> String {
    color(spec, 38)
}
fn bg(spec: &str) -> String {
    color(spec, 48)
}

/// Terminal display columns of a code point, matching upstream statusline.sh's
/// seg_len(): wide CJK / kana / Hangul / fullwidth / emoji count as 2, combining
/// and zero-width marks as 0, everything else as 1.
fn char_width(cp: u32) -> usize {
    if cp < 768 {
        return 1; // ASCII + Latin fast path
    }
    let in_range = |lo: u32, hi: u32| cp >= lo && cp <= hi;
    // combining / ZWSP / variation selector → 0 columns
    if in_range(768, 879) || in_range(8203, 8207) || in_range(65024, 65039) {
        0
    // East-Asian wide / fullwidth / emoji → 2 columns
    } else if in_range(4352, 4447)
        || in_range(11904, 42191)
        || in_range(44032, 55203)
        || in_range(63744, 64255)
        || in_range(65040, 65049)
        || in_range(65072, 65103)
        || in_range(65280, 65376)
        || in_range(65504, 65510)
        || in_range(127744, 129791)
        || in_range(131072, 262143)
    {
        2
    } else {
        1
    }
}

/// Visible display width (terminal columns) with ANSI escape sequences (ESC…m)
/// stripped. Counts columns, not code points, so wide CJK/emoji and zero-width
/// marks measure correctly — drives auto-layout wrapping in parity with bash.
fn seg_len(s: &str) -> usize {
    let mut n = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            n += char_width(c as u32);
        }
    }
    n
}

/// Normalize a path prefix for matching: backslashes → '/', and an MSYS-style
/// leading "/c/Users" drive → "c:/Users".
fn normalize_prefix(dir: &str) -> String {
    let mut h = dir.replace('\\', "/");
    let hb = h.as_bytes();
    if hb.len() >= 3 && hb[0] == b'/' && hb[1].is_ascii_alphabetic() && hb[2] == b'/' {
        h = format!("{}:{}", &h[1..2], &h[2..]);
    }
    h
}

/// If `short` is under one of the configured project roots, rewrite it relative
/// to that root in place and return true. Matching is case-insensitive (Windows
/// paths) on '/'-normalized forms. Returns false (leaving `short` untouched) when
/// no root is configured or matches. Native-only extension over upstream.
fn strip_project_root(short: &mut String, roots: &[String]) -> bool {
    if roots.is_empty() {
        return false;
    }
    // Normalize the cwd side too: real Windows payloads use backslashes, so match
    // (and emit the stripped remainder) on a '/'-normalized form.
    let norm = normalize_prefix(short);
    let low = norm.to_lowercase();
    for root in roots {
        if root.is_empty() {
            continue;
        }
        let h = normalize_prefix(root);
        let hlow = h.to_lowercase();
        if low == hlow {
            // Sitting in the root itself: keep just its basename.
            if let Some(base) = norm.rsplit('/').find(|p| !p.is_empty()) {
                *short = base.to_string();
            }
            return true;
        } else if low.starts_with(&format!("{hlow}/")) {
            let cut = (h.len() + 1).min(norm.len());
            *short = norm[cut..].to_string();
            return true;
        }
    }
    false
}

/// First line of a pin file (.nvmrc / .python-version), whitespace-trimmed.
/// `[ -f ]` semantics: a directory of the same name never matches.
fn pin_file(dir: &str, name: &str) -> Option<String> {
    let path = crate::config::msys_to_win(&format!("{dir}/{name}"));
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(p).ok()?;
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

/// Walk `dir` and its ancestors exactly like upstream's runtime_* loop: step up
/// by stripping the last '/'-component, stop when no '/' is left or the dir is
/// "/" or empty. (A backslash-only Windows path therefore only checks the leaf
/// directory — same as bash.)
fn walk_up(dir: &str, mut probe: impl FnMut(&str) -> Option<String>) -> Option<String> {
    let mut d = dir.to_string();
    while !d.is_empty() && d != "/" {
        if let Some(v) = probe(&d) {
            return Some(v);
        }
        match d.rfind('/') {
            Some(i) => d.truncate(i),
            None => break,
        }
    }
    None
}

/// Active Node version label for `dir` (pin files first, PATH probe opt-in).
fn runtime_node(dir: &str, probe: bool) -> Option<String> {
    let hit = walk_up(dir, |d| {
        for f in [".nvmrc", ".node-version"] {
            if let Some(v) = pin_file(d, f) {
                return Some(v.strip_prefix('v').unwrap_or(&v).to_string());
            }
        }
        None
    });
    if hit.is_some() {
        return hit;
    }
    if probe {
        let out = std::process::Command::new("node").arg("--version").output().ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.is_empty() {
            return Some(v.strip_prefix('v').unwrap_or(&v).to_string());
        }
    }
    None
}

/// Active Python env/version label for `dir` (venv, conda, pin file, probe).
fn runtime_python(dir: &str, probe: bool) -> Option<String> {
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        if !venv.is_empty() {
            return Some(venv.rsplit('/').next().unwrap_or(&venv).to_string());
        }
    }
    // conda auto-activates `base` for most users, so it is not a meaningful env.
    if let Ok(conda) = std::env::var("CONDA_DEFAULT_ENV") {
        if !conda.is_empty() && conda != "base" {
            return Some(conda);
        }
    }
    let hit = walk_up(dir, |d| pin_file(d, ".python-version"));
    if hit.is_some() {
        return hit;
    }
    if probe {
        // some builds print the version to stderr
        let out = std::process::Command::new("python3").arg("--version").output().ok()?;
        let mut v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if v.is_empty() {
            v = String::from_utf8_lossy(&out.stderr).trim().to_string();
        }
        let v = v.strip_prefix("Python ").unwrap_or(&v).trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

/// Plain text of `s` with ANSI escape sequences (ESC…m) removed. Mirrors
/// upstream statusline.sh's strip_ansi, used to build the float readout.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn make_bar(pct: i64, width: i64, fill: &str, empty: &str) -> String {
    let mut filled = (pct * width + 50) / 100;
    if filled > width {
        filled = width;
    }
    if filled < 0 {
        filled = 0;
    }
    let mut s = String::new();
    for _ in 0..filled {
        s.push_str(fill);
    }
    for _ in filled..width {
        s.push_str(empty);
    }
    s
}

/// 1234 → 1.2k · 1234567 → 1.2M (integer math only), matching upstream fmt_tok.
pub(crate) fn fmt_tok(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{}.{}M", n / 1_000_000, (n % 1_000_000) / 100_000)
    } else if n >= 1000 {
        format!("{}.{}k", n / 1000, (n % 1000) / 100)
    } else {
        format!("{}", n)
    }
}

fn fmt_duration(ms: i64) -> String {
    let s = ms / 1000;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if h > 0 {
        format!("{}h{:02}m", h, m)
    } else if m > 0 {
        format!("{}m", m)
    } else {
        format!("{}s", s)
    }
}

/// Middle-truncate to `max` visible chars with … ; max<=0 → unchanged.
pub(crate) fn trunc(s: &str, max: i64) -> String {
    if max <= 0 {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    if len <= max {
        return s.to_string();
    }
    if max < 3 {
        return chars[..max as usize].iter().collect();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let start = len - tail;
    let h: String = chars[..head as usize].iter().collect();
    let t: String = chars[start as usize..].iter().collect();
    format!("{h}\u{2026}{t}")
}

pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Canonical UTC ISO timestamp → epoch, strict (upstream iso_epoch): exact
/// "####-##-##T##:##:##" shape after dropping a trailing Z / fraction, no
/// timezone offset, calendar-validated. Returns None for everything else.
pub(crate) fn iso_epoch_strict(t: &str) -> Option<i64> {
    let tm = t.split_once('T')?.1;
    if tm.contains('+') || tm.contains('-') {
        return None;
    }
    let s = t.strip_suffix('Z').unwrap_or(t);
    let s = s.split('.').next().unwrap_or(s);
    let b = s.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    for (i, &c) in b.iter().enumerate() {
        if ![4, 7, 10, 13, 16].contains(&i) && !c.is_ascii_digit() {
            return None;
        }
    }
    let g = |a: usize, z: usize| s[a..z].parse::<i64>().ok();
    let (y, mo, d) = (g(0, 4)?, g(5, 7)?, g(8, 10)?);
    let (h, mi, sec) = (g(11, 13)?, g(14, 16)?, g(17, 19)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dim = match mo {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 31,
    };
    if !(1..=12).contains(&mo) || !(1..=dim).contains(&d) || h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec)
}

pub(crate) fn to_epoch(t: &str) -> Option<i64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    if t.contains('T') {
        let b = t.as_bytes();
        if b.len() < 19 {
            return None;
        }
        let g = |a: usize, z: usize| std::str::from_utf8(&b[a..z]).ok()?.parse::<i64>().ok();
        let y = g(0, 4)?;
        let mo = g(5, 7)?;
        let d = g(8, 10)?;
        let h = g(11, 13)?;
        let mi = g(14, 16)?;
        let s = g(17, 19)?;
        Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
    } else {
        t.split('.').next().unwrap_or(t).parse::<i64>().ok()
    }
}

fn fmt_countdown(reset: &str, now: i64) -> String {
    let rst = match to_epoch(reset) {
        Some(v) => v,
        None => return String::new(),
    };
    let diff = rst - now;
    if diff <= 0 {
        return "now".into();
    }
    let d = diff / 86400;
    let h = (diff % 86400) / 3600;
    let m = (diff % 3600) / 60;
    if d > 0 {
        format!("{}d{:02}h", d, h)
    } else if h > 0 {
        format!("{}h{:02}m", h, m)
    } else {
        format!("{}m", m)
    }
}

fn round_pct(v: f64) -> i64 {
    v.round() as i64
}

/// d/h/m rendering of a burn ETA in seconds (mirrors fmt_countdown's shapes).
fn fmt_eta(s: i64) -> String {
    let d = s / 86400;
    let h = (s % 86400) / 3600;
    let m = (s % 3600) / 60;
    if d > 0 {
        format!("{}d{:02}h", d, h)
    } else if h > 0 {
        format!("{}h{:02}m", h, m)
    } else {
        format!("{}m", m)
    }
}

struct Ctx<'a> {
    cfg: &'a Config,
    p: &'a Payload,
    git: &'a GitInfo,
    home: &'a str,
    hour: u32,
    min: u32,
    sec: u32,
    now_epoch: i64,
    burn: Option<&'a crate::burn::Burn>,
    fg_text: String,
    fg_dim: String,
    fg_ok: String,
    fg_warn: String,
    fg_hot: String,
}

impl<'a> Ctx<'a> {
    fn pct_fg(&self, p: i64) -> String {
        if p >= self.cfg.hot_pct {
            self.fg_hot.clone()
        } else if p >= self.cfg.warn_pct {
            self.fg_warn.clone()
        } else {
            self.fg_ok.clone()
        }
    }

    fn build(&self, list: &str) -> Vec<Seg> {
        let mut segs = Vec::new();
        for name in list.split_whitespace() {
            self.seg(name, &mut segs);
        }
        segs
    }

    fn push(&self, segs: &mut Vec<Seg>, bgc: &str, txt: String) {
        let len = seg_len(&txt);
        segs.push(Seg {
            bg: bgc.to_string(),
            txt,
            len,
        });
    }

    fn seg_dir(&self, segs: &mut Vec<Seg>) {
        let cfg = self.cfg;
        let p = self.p;
        if p.cwd.is_empty() {
            return;
        }
        let mut short = if !self.home.is_empty() && p.cwd.starts_with(self.home) {
            format!("~{}", &p.cwd[self.home.len()..])
        } else {
            p.cwd.clone()
        };
        // VL_PROJECT_ROOTS (native-only extension): when home didn't already win,
        // strip a configured project-root prefix (e.g. D:/GitHub) so deep repo
        // paths render relative to it, marked with a house glyph. With no roots
        // configured this is inert and output stays byte-identical to bash.
        let in_root = !short.starts_with('~') && strip_project_root(&mut short, &cfg.project_roots);
        if in_root {
            // Under a project root, show just the repo name (the first path
            // component); deeper location lives in the worktree/git segments.
            if let Some(repo) = short.split('/').find(|s| !s.is_empty()) {
                short = repo.to_string();
            }
        }
        // Split like bash `set -- $short` with IFS=/: a leading '/' yields
        // a leading empty field, so "/a/b/c/d" counts as 5 fields (and the
        // rebuilt "$1/$2/…/$last" keeps the leading slash). Don't drop empties.
        let parts: Vec<&str> = short.split('/').collect();
        let mut disp = if parts.len() as i64 > cfg.path_depth && parts.len() >= 2 {
            format!("{}/{}/\u{2026}/{}", parts[0], parts[1], parts[parts.len() - 1])
        } else {
            short
        };
        if in_root && !cfg.ascii {
            disp = format!("\u{2302} {disp}"); // ⌂ marks a stripped project root
        }
        self.push(
            segs,
            &cfg.bg_dir,
            format!("{BOLD}{} {} {NORM}", self.fg_text, disp),
        );
    }

    fn seg(&self, name: &str, segs: &mut Vec<Seg>) {
        let cfg = self.cfg;
        let p = self.p;
        match name {
            "project" => {
                // Repo-root name in a repo; outside one fall back to `dir` so a
                // `project`-in-place-of-`dir` layout still shows a path — unless
                // `dir` is already in the active layout (avoids rendering twice).
                if self.git.root.is_empty() {
                    let active =
                        format!(" {} {} {} ", cfg.segments, cfg.segments2, cfg.segments3);
                    if active.contains(" dir ") {
                        return;
                    }
                    self.seg_dir(segs);
                    return;
                }
                let bg = if cfg.bg_project.is_empty() {
                    &cfg.bg_dir
                } else {
                    &cfg.bg_project
                };
                self.push(
                    segs,
                    bg,
                    format!(
                        "{BOLD}{} \u{2B22} {} {NORM}",
                        self.fg_text,
                        trunc(&self.git.root, cfg.name_max)
                    ),
                );
            }
            // Extension (not in upstream): the linked-worktree name as its own pill.
            "worktree" => {
                if self.git.wt_name.is_empty() {
                    return;
                }
                self.push(
                    segs,
                    &cfg.bg_wt,
                    format!("{BOLD}{} \u{2442} {} {NORM}", self.fg_text, self.git.wt_name),
                );
            }
            "dir" => self.seg_dir(segs),
            "git" => {
                if self.git.branch.is_empty() {
                    return;
                }
                let bgc = if self.git.dirty {
                    &cfg.bg_git_dirty
                } else {
                    &cfg.bg_git_ok
                };
                self.push(
                    segs,
                    bgc,
                    format!(
                        "{BOLD}{} \u{2387} {}{}{} {NORM}",
                        self.fg_text,
                        trunc(&self.git.branch, cfg.name_max),
                        self.git.marks,
                        self.git.ab
                    ),
                );
            }
            "node" => {
                if p.cwd.is_empty() {
                    return;
                }
                let Some(v) = runtime_node(&p.cwd, cfg.runtime_probe) else {
                    return;
                };
                let bgc = if cfg.bg_node.is_empty() {
                    &cfg.bg_model
                } else {
                    &cfg.bg_node
                };
                self.push(
                    segs,
                    bgc,
                    format!("{} {} {} ", self.fg_text, cfg.node_glyph, v),
                );
            }
            "python" => {
                if p.cwd.is_empty() {
                    return;
                }
                let Some(v) = runtime_python(&p.cwd, cfg.runtime_probe) else {
                    return;
                };
                let bgc = if cfg.bg_python.is_empty() {
                    &cfg.bg_model
                } else {
                    &cfg.bg_python
                };
                self.push(
                    segs,
                    bgc,
                    format!("{} {} {} ", self.fg_text, cfg.py_glyph, v),
                );
            }
            "model" => {
                if p.model.is_empty() {
                    return;
                }
                let m = p.model.strip_prefix("Claude ").unwrap_or(&p.model);
                self.push(
                    segs,
                    &cfg.bg_model,
                    format!("{BOLD}{} \u{25C6} {} {NORM}", self.fg_text, m),
                );
            }
            "ctx" => {
                let cp = match p.ctx_pct {
                    Some(v) => v,
                    None => return,
                };
                let ci = round_pct(cp);
                let bar = make_bar(ci, cfg.bar_width, &cfg.bar_fill, &cfg.bar_empty);
                let cn = self.pct_fg(ci);
                self.push(
                    segs,
                    &cfg.bg_ctx,
                    format!(
                        "{} \u{2B21} {} {}% {}\u{2191}{} \u{2193}{} cr:{} cw:{} ",
                        cn,
                        bar,
                        ci,
                        self.fg_dim,
                        fmt_tok(p.tok_in),
                        fmt_tok(p.tok_out),
                        fmt_tok(p.tok_cr),
                        fmt_tok(p.tok_cw)
                    ),
                );
            }
            "limit5h" => {
                // With VL_LIMIT_SYNC, show the freshest cross-session value for
                // the current window (falling back to this session's snapshot).
                let (mut pv, mut rs) = (p.fh_pct, p.fh_rst.clone());
                if cfg.limit_sync {
                    if let Some((pct, rst)) =
                        crate::burn::rl_latest(&cfg.rl5h_file, crate::burn::RL_MAX_5H, self.now_epoch)
                    {
                        pv = pct.parse().ok();
                        rs = rst.to_string();
                    }
                }
                self.seg_limit(segs, "5h", pv, &rs, &cfg.bg_5h)
            }
            "limit7d" => {
                if !p.gw_weekly.is_empty() {
                    for (name, pct, rst) in &p.gw_weekly {
                        self.seg_limit(segs, &format!("7d ({name})"), Some(*pct), rst, &cfg.bg_7d);
                    }
                    return;
                }
                let (mut pv, mut rs) = (p.wd_pct, p.wd_rst.clone());
                if cfg.limit_sync {
                    if let Some((pct, rst)) =
                        crate::burn::rl_latest(&cfg.rl7d_file, crate::burn::RL_MAX_7D, self.now_epoch)
                    {
                        pv = pct.parse().ok();
                        rs = rst.to_string();
                    }
                }
                self.seg_limit(segs, "7d", pv, &rs, &cfg.bg_7d)
            }
            "burn" => {
                // range-to-empty ETA until the binding 5h/7d limit hits 100%
                if p.fh_pct_raw.is_empty() && p.wd_pct_raw.is_empty() {
                    return;
                }
                let Some(b) = self.burn else { return };
                let bgc = if cfg.bg_burn.is_empty() {
                    &cfg.bg_5h
                } else {
                    &cfg.bg_burn
                };
                if b.state != "active" {
                    // Idle (stopped burning) is genuinely all-good → dim ✓;
                    // warming (no samples yet) is "unknown" → a distinct dim ….
                    let glyph = if b.state == "warming" { "\u{2026}" } else { "\u{2713}" };
                    self.push(
                        segs,
                        bgc,
                        format!("{} {} {} ", self.fg_dim, cfg.burn_glyph, glyph),
                    );
                    return;
                }
                // All good: projected empty exceeds the limit's whole window.
                let win = if b.label == "5h" { 18000 } else { 604800 };
                if b.eta > win {
                    self.push(
                        segs,
                        bgc,
                        format!("{} {} \u{2713} ", self.fg_ok, cfg.burn_glyph),
                    );
                    return;
                }
                let col = if b.eta <= b.ttr {
                    &self.fg_hot
                } else if 10 * b.ttr >= 8 * b.eta {
                    &self.fg_warn
                } else {
                    &self.fg_ok
                };
                self.push(
                    segs,
                    bgc,
                    format!(
                        "{} {} {} \u{21E2} {} ",
                        col,
                        cfg.burn_glyph,
                        b.label,
                        fmt_eta(b.eta)
                    ),
                );
            }
            "cost" => {
                let c = match p.cost {
                    Some(v) if v != 0.0 => v,
                    _ => return,
                };
                self.push(
                    segs,
                    &cfg.bg_cost,
                    format!("{} ${:.*} ", self.fg_text, cfg.cost_decimals, c),
                );
            }
            "clock" => self.seg_clock(segs),
            "lines" => {
                if p.lines_add <= 0 && p.lines_del <= 0 {
                    return;
                }
                self.push(
                    segs,
                    &cfg.bg_lines,
                    format!(
                        " {}+{} {}-{} ",
                        self.fg_ok, p.lines_add, self.fg_hot, p.lines_del
                    ),
                );
            }
            "style" => {
                if p.out_style.is_empty() || p.out_style == "default" {
                    return;
                }
                self.push(
                    segs,
                    &cfg.bg_style,
                    format!("{} \u{270E} {} ", self.fg_text, p.out_style),
                );
            }
            "duration" => {
                if p.dur_ms <= 0 {
                    return;
                }
                self.push(
                    segs,
                    &cfg.bg_duration,
                    format!("{} \u{29D6} {} ", self.fg_text, fmt_duration(p.dur_ms)),
                );
            }
            // reasoning effort level (low/medium/high/xhigh/max); glyph ψ is editable
            "effort" => {
                if p.effort.is_empty() {
                    return;
                }
                let label = if p.effort == "medium" { "med" } else { p.effort.as_str() };
                self.push(
                    segs,
                    &cfg.bg_effort,
                    format!("{} \u{3C8} {} ", self.fg_text, label),
                );
            }
            "stash" => {
                if self.git.branch.is_empty() {
                    return;
                }
                let n = self.stash_count();
                if n > 0 {
                    let bgc = if cfg.bg_stash.is_empty() {
                        &cfg.bg_git_ok
                    } else {
                        &cfg.bg_stash
                    };
                    self.push(segs, bgc, format!("{} \u{2691} {} ", self.fg_text, n));
                }
            }
            _ => {}
        }
    }

    fn seg_limit(&self, segs: &mut Vec<Seg>, label: &str, pct: Option<f64>, reset: &str, bgc: &str) {
        let pv = match pct {
            Some(v) => v,
            None => return,
        };
        let v = round_pct(pv);
        let bar = make_bar(v, self.cfg.bar_width, &self.cfg.bar_fill, &self.cfg.bar_empty);
        let cn = self.pct_fg(v);
        let cd = fmt_countdown(reset, self.now_epoch);
        let rst = if cd.is_empty() {
            String::new()
        } else {
            format!("{}\u{21BA}{}", self.fg_dim, cd)
        };
        self.push(
            segs,
            bgc,
            format!("{} {} {} {}% {} ", cn, label, bar, v, rst),
        );
    }

    fn seg_clock(&self, segs: &mut Vec<Seg>) {
        let cfg = self.cfg;
        if cfg.clock == "off" {
            return;
        }
        let (t, ap);
        if cfg.clock == "24h" {
            t = if cfg.clock_seconds {
                format!("{:02}:{:02}:{:02}", self.hour, self.min, self.sec)
            } else {
                format!("{:02}:{:02}", self.hour, self.min)
            };
            ap = String::new();
        } else {
            let mut h12 = self.hour % 12;
            if h12 == 0 {
                h12 = 12;
            }
            t = if cfg.clock_seconds {
                format!("{:02}:{:02}:{:02}", h12, self.min, self.sec)
            } else {
                format!("{:02}:{:02}", h12, self.min)
            };
            ap = format!(" {}", if self.hour < 12 { "am" } else { "pm" });
        }
        self.push(
            segs,
            &cfg.bg_clock,
            format!("{} \u{2299} {}{} ", self.fg_text, t, ap),
        );
    }

    /// Build VL_FLOAT_SEGMENTS as a single plain-text (ANSI-stripped) line and
    /// write it atomically to VL_FLOAT_FILE. Mirrors upstream emit_float: a
    /// "bring your own carrier" hook (see `coralline --float-carrier`).
    fn emit_float(&self) {
        let cfg = self.cfg;
        let segs = self.build(&cfg.float_segments);
        let mut line = String::new();
        for s in &segs {
            let plain = strip_ansi(&s.txt);
            let t = plain.trim();
            if t.is_empty() {
                continue;
            }
            if !line.is_empty() {
                line.push_str(&cfg.float_sep);
            }
            line.push_str(t);
        }
        let path = std::path::Path::new(&cfg.float_file);
        let Some(dir) = path.parent() else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!(".float.tmp.{}", std::process::id()));
        if std::fs::write(&tmp, format!("{line}\n")).is_ok() {
            if std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn stash_count(&self) -> i64 {
        if let Some(gd) = crate::git::git_dir_of(&self.p.cwd) {
            let logf = gd.join("logs").join("refs").join("stash");
            if let Ok(c) = std::fs::read_to_string(logf) {
                return c.lines().filter(|l| !l.trim().is_empty()).count() as i64;
            }
        }
        0
    }
}

fn print_range_pill(cfg: &Config, segs: &[Seg], start: usize, end: usize) -> String {
    let mut out = String::new();
    out.push_str(R);
    out.push_str(&fg(&segs[start].bg));
    out.push_str(&cfg.cap_l);
    for i in start..=end {
        out.push_str(&bg(&segs[i].bg));
        out.push_str(&segs[i].txt);
        if i < end {
            out.push_str(&bg(&segs[i + 1].bg));
            out.push_str(&fg(&segs[i].bg));
            out.push_str(&cfg.sep);
        }
    }
    out.push_str(R);
    out.push_str(&fg(&segs[end].bg));
    out.push_str(&cfg.cap_r);
    out.push_str(R);
    out
}

fn print_range_lean(cfg: &Config, segs: &[Seg], start: usize, end: usize) -> String {
    // VL_LEAN_BG paints one uniform background behind the row (the p10k
    // "classic" look); re-asserted after every reset so the bar stays
    // continuous across separators. The caps bevel the bar into the terminal:
    // the glyph is drawn in the bar colour on the default background.
    let lbg = if cfg.lean_bg.is_empty() {
        String::new()
    } else {
        bg(&cfg.lean_bg)
    };
    let mut out = String::new();
    if !lbg.is_empty() && !cfg.lean_cap_l.is_empty() {
        out.push_str(R);
        out.push_str(&fg(&cfg.lean_bg));
        out.push_str(&cfg.lean_cap_l);
    }
    for i in start..=end {
        out.push_str(R);
        out.push_str(&lbg);
        out.push_str(&fg(&segs[i].bg));
        out.push_str(&segs[i].txt);
        if i < end {
            out.push_str(R);
            out.push_str(&lbg);
            out.push_str(&cfg.lean_sep);
        }
    }
    if !lbg.is_empty() && !cfg.lean_cap_r.is_empty() {
        out.push_str(R);
        out.push_str(&fg(&cfg.lean_bg));
        out.push_str(&cfg.lean_cap_r);
    }
    out.push_str(R);
    out
}

pub(crate) fn print_range(cfg: &Config, segs: &[Seg], start: usize, end: usize) -> String {
    if cfg.style == "lean" {
        print_range_lean(cfg, segs, start, end)
    } else {
        print_range_pill(cfg, segs, start, end)
    }
}

fn term_cols() -> i64 {
    if let Ok(c) = std::env::var("COLUMNS") {
        if !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()) {
            return c.parse().unwrap_or(0);
        }
    }
    0
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    cfg: &Config,
    p: &Payload,
    git: &GitInfo,
    home: &str,
    hour: u32,
    min: u32,
    sec: u32,
    now_epoch: i64,
    burn: Option<&crate::burn::Burn>,
) -> String {
    let ctx = Ctx {
        cfg,
        p,
        git,
        home,
        hour,
        min,
        sec,
        now_epoch,
        burn,
        fg_text: fg(&cfg.fg_text),
        fg_dim: fg(&cfg.fg_dim),
        fg_ok: fg(&cfg.fg_ok),
        fg_warn: fg(&cfg.fg_warn),
        fg_hot: fg(&cfg.fg_hot),
    };

    // Side effect before the main render: emit the plain-text float readout.
    if cfg.float {
        ctx.emit_float();
    }

    let mut rows: Vec<String> = Vec::new();

    if cfg.layout == "auto" {
        let segs = ctx.build(&cfg.segments);
        let total = segs.len();
        if total == 0 {
            return String::new();
        }
        let w0 = term_cols();
        if w0 <= 0 || cfg.max_lines <= 1 {
            rows.push(print_range(cfg, &segs, 0, total - 1));
        } else {
            let mut w = w0 - cfg.wrap_margin;
            if w < 1 {
                w = 1;
            }
            let (cap_w, sep_w): (i64, i64) = if cfg.style == "lean" {
                (
                    (cfg.lean_cap_l.chars().count() + cfg.lean_cap_r.chars().count()) as i64,
                    cfg.lean_sep.chars().count() as i64,
                )
            } else {
                (2, 1)
            };
            let mut start = 0usize;
            let mut line = 1i64;
            let mut cur = cap_w + segs[0].len as i64;
            let mut i = 1usize;
            while i < total {
                let need = cur + sep_w + segs[i].len as i64;
                if need > w && line < cfg.max_lines {
                    rows.push(print_range(cfg, &segs, start, i - 1));
                    start = i;
                    line += 1;
                    cur = cap_w + segs[i].len as i64;
                } else {
                    cur = need;
                }
                i += 1;
            }
            rows.push(print_range(cfg, &segs, start, total - 1));
        }
    } else {
        for list in [&cfg.segments, &cfg.segments2, &cfg.segments3] {
            if list.is_empty() {
                continue;
            }
            let segs = ctx.build(list);
            if !segs.is_empty() {
                rows.push(print_range(cfg, &segs, 0, segs.len() - 1));
            }
        }
    }

    rows.join("\n")
}
