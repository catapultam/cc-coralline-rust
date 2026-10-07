//! Configuration: built-in defaults (mirroring upstream statusline.sh + the
//! claude-coral theme) overridden by the user's bash config file. We don't run
//! bash — we read the conf as simple `KEY=VALUE` assignments and follow one
//! level of `. include` / `source include` (used to pull in the theme file),
//! which is all coralline's conf format actually uses.
use std::path::{Path, PathBuf};

pub struct Config {
    pub style: String,
    pub lean_sep: String,
    pub lean_fg: String,
    pub lean_bg: String,
    pub lean_cap_l: String,
    pub lean_cap_r: String,
    pub bg_bar: String,
    pub layout: String,
    pub max_lines: i64,
    pub wrap_margin: i64,
    pub segments: String,
    pub segments2: String,
    pub segments3: String,
    pub float: bool,
    pub float_segments: String,
    pub float_sep: String,
    pub float_file: String,
    pub bar_width: i64,
    pub bar_fill: String,
    pub bar_empty: String,
    pub clock: String,
    pub clock_seconds: bool,
    pub path_depth: i64,
    pub project_roots: Vec<String>,
    pub name_max: i64,
    pub cost_decimals: usize,
    pub warn_pct: i64,
    pub hot_pct: i64,
    pub ascii: bool,
    pub cap_l: String,
    pub cap_r: String,
    pub sep: String,
    pub git_ttl: i64,
    pub bg_dir: String,
    pub bg_project: String,
    pub bg_wt: String,
    pub bg_git_ok: String,
    pub bg_git_dirty: String,
    pub bg_stash: String,
    pub bg_model: String,
    pub bg_ctx: String,
    pub bg_5h: String,
    pub bg_7d: String,
    pub bg_cost: String,
    pub bg_clock: String,
    pub bg_lines: String,
    pub bg_style: String,
    pub bg_duration: String,
    pub bg_effort: String,
    pub bg_node: String,
    pub bg_python: String,
    pub node_glyph: String,
    pub py_glyph: String,
    pub runtime_probe: bool,
    // auto-mode server-review segment (native-only; gateway-fed)
    pub automode: bool,
    pub bg_automode: String,
    pub automode_glyph: String,
    // burn segment
    pub burn_window: i64,
    pub burn_glyph: String,
    pub bg_burn: String,
    pub burn_file: String,
    pub burn_trim: i64,
    // cross-session limit sync
    pub limit_sync: bool,
    pub rl5h_file: String,
    pub rl7d_file: String,
    // subagent panel rows
    pub sub_segments: String,
    pub bg_sub_name: String,
    pub bg_sub_model: String,
    pub bg_sub_ctx: String,
    pub bg_sub_elapsed: String,
    pub fg_text: String,
    pub fg_dim: String,
    pub fg_ok: String,
    pub fg_warn: String,
    pub fg_hot: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            style: "pill".into(),
            lean_sep: "".into(),
            lean_fg: "".into(),
            lean_bg: "".into(),
            lean_cap_l: "".into(),
            lean_cap_r: "".into(),
            bg_bar: "".into(),
            layout: "fixed".into(),
            max_lines: 3,
            wrap_margin: 4,
            segments: "dir git model ctx limit5h limit7d cost clock".into(),
            segments2: "".into(),
            segments3: "".into(),
            float: false,
            float_segments: "model ctx cost".into(),
            float_sep: "  \u{b7}  ".into(),
            float_file: "".into(), // resolved to $HOME/.claude/coralline/float.txt in load()
            bar_width: 5,
            bar_fill: "▰".into(),
            bar_empty: "▱".into(),
            clock: "12h".into(),
            clock_seconds: true,
            path_depth: 4,
            project_roots: Vec::new(),
            name_max: 0,
            cost_decimals: 2,
            warn_pct: 50,
            hot_pct: 75,
            ascii: false,
            cap_l: "\u{E0B6}".into(),
            cap_r: "\u{E0B4}".into(),
            sep: "\u{E0B0}".into(),
            git_ttl: 300,
            bg_dir: "81,166,199".into(),
            bg_project: "".into(), // optional; falls back to bg_dir when empty
            bg_wt: "152,130,190".into(),
            bg_git_ok: "65".into(),
            bg_git_dirty: "130".into(),
            bg_stash: "".into(), // optional; falls back to bg_git_ok when empty
            bg_model: "173".into(),
            bg_ctx: "238".into(),
            bg_5h: "237".into(),
            bg_7d: "236".into(),
            bg_cost: "212,125,145".into(),
            bg_clock: "70,80,110".into(),
            bg_lines: "240".into(),
            bg_style: "96".into(),
            bg_duration: "60".into(),
            bg_effort: "141".into(),
            bg_node: "".into(),   // optional; falls back to bg_model when empty
            bg_python: "".into(), // optional; falls back to bg_model when empty
            node_glyph: "\u{E718}".into(),
            py_glyph: "\u{E73C}".into(),
            runtime_probe: false,
            automode: true,
            bg_automode: "24".into(),
            automode_glyph: "\u{25C9}".into(),
            burn_window: 600,
            burn_glyph: "\u{2197}".into(),
            bg_burn: "".into(), // empty → inherits bg_5h at the use site
            burn_file: "".into(), // resolved in load(): $CORALLINE_BURN_FILE or default
            burn_trim: 1500,
            limit_sync: false,
            rl5h_file: "".into(), // resolved in load(): $CORALLINE_RL5H_FILE or default
            rl7d_file: "".into(),
            sub_segments: "name model ctx elapsed".into(),
            bg_sub_name: "".into(), // panel-row colors; empty → main-bar counterparts
            bg_sub_model: "".into(),
            bg_sub_ctx: "".into(),
            bg_sub_elapsed: "".into(),
            fg_text: "231".into(),
            fg_dim: "245".into(),
            fg_ok: "114".into(),
            fg_warn: "179".into(),
            fg_hot: "167".into(),
        }
    }
}

impl Config {
    pub fn load(home: &str) -> Config {
        let mut c = Config::default();
        // Env-derived defaults land before the config file is sourced (upstream
        // sets BURN_FILE / RL*_FILE from env in its defaults block), so a config
        // assignment can still override them.
        let envdef = |var: &str, def: String| match std::env::var(var) {
            Ok(v) if !v.is_empty() => v,
            _ => def,
        };
        c.burn_file = envdef(
            "CORALLINE_BURN_FILE",
            format!("{home}/.claude/coralline/burn-5h.tsv"),
        );
        c.rl5h_file = envdef(
            "CORALLINE_RL5H_FILE",
            format!("{home}/.claude/coralline/limit-5h.tsv"),
        );
        c.rl7d_file = envdef(
            "CORALLINE_RL7D_FILE",
            format!("{home}/.claude/coralline/limit-7d.tsv"),
        );
        let conf = std::env::var("CORALLINE_CONFIG")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("{home}/.claude/coralline.conf")));
        c.apply_file(&conf, home, 0);
        c.burn_file = expand(&c.burn_file, home);
        c.rl5h_file = expand(&c.rl5h_file, home);
        c.rl7d_file = expand(&c.rl7d_file, home);
        if c.float_file.is_empty() {
            c.float_file = format!("{home}/.claude/coralline/float.txt");
        } else {
            c.float_file = expand(&c.float_file, home);
        }
        c.post();
        c
    }

    fn apply_file(&mut self, path: &Path, home: &str, depth: u8) {
        if depth > 4 {
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return,
        };
        for raw in text.lines() {
            let line = match raw.find('#') {
                Some(i) => &raw[..i],
                None => raw,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let inc = line
                .strip_prefix(". ")
                .or_else(|| line.strip_prefix("source "))
                .map(str::trim);
            if let Some(incpath) = inc {
                let expanded = expand(incpath, home);
                self.apply_file(Path::new(&expanded), home, depth + 1);
                continue;
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim();
                let val = unquote(line[eq + 1..].trim());
                self.set(key, &val);
            }
        }
    }

    fn set(&mut self, key: &str, val: &str) {
        let v = val.to_string();
        match key {
            "VL_STYLE" => self.style = v,
            "VL_LEAN_SEP" => self.lean_sep = v,
            "VL_LEAN_FG" => self.lean_fg = v,
            "VL_LEAN_BG" => self.lean_bg = v,
            "VL_LEAN_CAP_L" => self.lean_cap_l = v,
            "VL_LEAN_CAP_R" => self.lean_cap_r = v,
            "VL_BG_BAR" => self.bg_bar = v,
            "VL_LAYOUT" => self.layout = v,
            "VL_MAX_LINES" => self.max_lines = v.parse().unwrap_or(self.max_lines),
            "VL_WRAP_MARGIN" => self.wrap_margin = v.parse().unwrap_or(self.wrap_margin),
            "VL_SEGMENTS" => self.segments = v,
            "VL_SEGMENTS2" => self.segments2 = v,
            "VL_SEGMENTS3" => self.segments3 = v,
            "VL_FLOAT" => self.float = v == "1",
            "VL_FLOAT_SEGMENTS" => self.float_segments = v,
            "VL_FLOAT_SEP" => self.float_sep = v,
            "VL_FLOAT_FILE" => self.float_file = v,
            "VL_BAR_WIDTH" => self.bar_width = v.parse().unwrap_or(self.bar_width),
            "VL_BAR_FILL" => self.bar_fill = v,
            "VL_BAR_EMPTY" => self.bar_empty = v,
            "VL_CLOCK" => self.clock = v,
            "VL_CLOCK_SECONDS" => self.clock_seconds = v == "1",
            "VL_PATH_DEPTH" => self.path_depth = v.parse().unwrap_or(self.path_depth),
            "VL_PROJECT_ROOTS" => {
                self.project_roots = v
                    .split([',', ';'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            "VL_NAME_MAX" => self.name_max = v.parse().unwrap_or(self.name_max),
            "VL_COST_DECIMALS" => self.cost_decimals = v.parse().unwrap_or(self.cost_decimals),
            "VL_WARN_PCT" => self.warn_pct = v.parse().unwrap_or(self.warn_pct),
            "VL_HOT_PCT" => self.hot_pct = v.parse().unwrap_or(self.hot_pct),
            "VL_ASCII" => self.ascii = v == "1",
            "VL_GIT_TTL" => self.git_ttl = v.parse().unwrap_or(self.git_ttl),
            "VL_CAP_L" => self.cap_l = v,
            "VL_CAP_R" => self.cap_r = v,
            "VL_SEP" => self.sep = v,
            "VL_BG_DIR" => self.bg_dir = v,
            "VL_BG_PROJECT" => self.bg_project = v,
            "VL_BG_WT" => self.bg_wt = v,
            "VL_BG_GIT_OK" => self.bg_git_ok = v,
            "VL_BG_GIT_DIRTY" => self.bg_git_dirty = v,
            "VL_BG_STASH" => self.bg_stash = v,
            "VL_BG_MODEL" => self.bg_model = v,
            "VL_BG_CTX" => self.bg_ctx = v,
            "VL_BG_5H" => self.bg_5h = v,
            "VL_BG_7D" => self.bg_7d = v,
            "VL_BG_COST" => self.bg_cost = v,
            "VL_BG_CLOCK" => self.bg_clock = v,
            "VL_BG_LINES" => self.bg_lines = v,
            "VL_BG_STYLE" => self.bg_style = v,
            "VL_BG_DURATION" => self.bg_duration = v,
            "VL_BG_EFFORT" => self.bg_effort = v,
            "VL_BG_NODE" => self.bg_node = v,
            "VL_BG_PYTHON" => self.bg_python = v,
            "VL_NODE_GLYPH" => self.node_glyph = v,
            "VL_PY_GLYPH" => self.py_glyph = v,
            "VL_RUNTIME_PROBE" => self.runtime_probe = v == "1",
            // Opt-out knob (default enabled): "0" disables, anything else
            // (including unset) leaves the segment active.
            "VL_AUTOMODE" => self.automode = v != "0",
            "VL_BG_AUTOMODE" => self.bg_automode = v,
            "VL_AUTOMODE_GLYPH" => self.automode_glyph = v,
            "CORALLINE_BURN_WINDOW" => self.burn_window = v.parse().unwrap_or(self.burn_window),
            "VL_BURN_GLYPH" => self.burn_glyph = v,
            "VL_BG_BURN" => self.bg_burn = v,
            "BURN_FILE" => self.burn_file = v,
            "BURN_TRIM" => self.burn_trim = v.parse().unwrap_or(self.burn_trim),
            "VL_LIMIT_SYNC" => self.limit_sync = v == "1",
            "RL5H_FILE" => self.rl5h_file = v,
            "RL7D_FILE" => self.rl7d_file = v,
            "VL_SUB_SEGMENTS" => self.sub_segments = v,
            "VL_BG_SUB_NAME" => self.bg_sub_name = v,
            "VL_BG_SUB_MODEL" => self.bg_sub_model = v,
            "VL_BG_SUB_CTX" => self.bg_sub_ctx = v,
            "VL_BG_SUB_ELAPSED" => self.bg_sub_elapsed = v,
            "VL_FG_TEXT" => self.fg_text = v,
            "VL_FG_DIM" => self.fg_dim = v,
            "VL_FG_OK" => self.fg_ok = v,
            "VL_FG_WARN" => self.fg_warn = v,
            "VL_FG_HOT" => self.fg_hot = v,
            _ => {}
        }
    }

    fn post(&mut self) {
        if self.ascii {
            self.cap_l.clear();
            self.cap_r.clear();
            self.sep.clear();
            self.bar_fill = "#".into();
            self.bar_empty = "-".into();
            self.node_glyph = "node".into();
            self.py_glyph = "py".into();
        }
        // Classic = lean on one uniform dark bar with a trailing cap. Resolved
        // after the ASCII block (so an ASCII render's cleared sep leaves the cap
        // empty but the bar still paints) and before the lean block, exactly like
        // upstream. Explicit VL_LEAN_BG / VL_LEAN_CAP_R win over the preset.
        if self.style == "classic" {
            self.style = "lean".into();
            if self.lean_bg.is_empty() {
                self.lean_bg = if self.bg_bar.is_empty() {
                    "238".into()
                } else {
                    self.bg_bar.clone()
                };
            }
            if self.lean_cap_r.is_empty() {
                self.lean_cap_r = self.sep.clone();
            }
        }
        if self.style == "lean" {
            self.cap_l.clear();
            self.cap_r.clear();
            self.fg_text = self.lean_fg.clone();
        }
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn expand(s: &str, home: &str) -> String {
    let s = s.trim();
    let p = if let Some(rest) = s.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if s == "~" {
        home.to_string()
    } else if let Some(rest) = s.strip_prefix("$HOME/") {
        format!("{home}/{rest}")
    } else {
        s.to_string()
    };
    msys_to_win(&p)
}

/// Convert an MSYS absolute path (/c/Users/…) to a Windows one (c:/Users/…) so
/// std::fs can read it — a config might `. /c/...` instead of using ~. Windows
/// only: on Linux/macOS `/x/...` is a legitimate native path, so leave it alone.
#[cfg(windows)]
pub(crate) fn msys_to_win(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
        format!("{}:{}", &p[1..2], &p[2..])
    } else {
        p.to_string()
    }
}

#[cfg(not(windows))]
pub(crate) fn msys_to_win(p: &str) -> String {
    p.to_string()
}
