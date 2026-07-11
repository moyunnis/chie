use std::path::{Path, PathBuf};

use crate::theme::Theme;

// Runtime behaviour, separate from the theme. Defaults match "just works": four
// spaces, numbers and highlighting on, mouse on, no backups.
#[derive(Clone)]
pub struct Config {
    pub tab_size: usize,
    pub use_spaces: bool,
    pub show_numbers: bool,
    pub syntax: bool,
    pub autoindent: bool,
    pub mouse: bool,
    pub backup: bool,
    pub scrolloff: usize,
    pub cursorline: bool,
    pub trim_trailing: bool,
    pub final_newline: bool,
    pub ignorecase: bool,
    pub smartcase: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tab_size: 4,
            use_spaces: true,
            show_numbers: true,
            syntax: true,
            autoindent: true,
            mouse: true,
            backup: false,
            scrolloff: 0,
            cursorline: false,
            trim_trailing: false,
            final_newline: false,
            ignorecase: true,
            smartcase: true,
        }
    }
}

pub struct Loaded {
    #[allow(dead_code)]
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

// Where the config lives. CHIE_CONFIG wins for scripts and tests; otherwise the
// usual per-platform spot. Returns None only when we can't find a home at all.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CHIE_CONFIG") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Some(Path::new(&x).join("chie").join("config"));
        }
    }
    #[cfg(windows)]
    if let Ok(x) = std::env::var("APPDATA") {
        if !x.is_empty() {
            return Some(Path::new(&x).join("chie").join("config"));
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        return Some(Path::new(&h).join(".config").join("chie").join("config"));
    }
    #[cfg(windows)]
    if let Ok(h) = std::env::var("USERPROFILE") {
        return Some(Path::new(&h).join(".config").join("chie").join("config"));
    }
    None
}

// Read the config (if any) and fold it into `cfg` and `theme`. A missing file is
// not an error — it just means defaults. Malformed lines are collected as
// warnings so the editor can mention them instead of dying.
pub fn load(cfg: &mut Config, theme: &mut Theme) -> Option<Loaded> {
    let path = config_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut warnings = Vec::new();

    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        // Whole-line comments only — an inline '#' is a hex color, not a comment.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => {
                warnings.push(format!("line {}: expected 'key = value'", i + 1));
                continue;
            }
        };
        if !apply(cfg, theme, key, val) {
            warnings.push(format!("line {}: bad setting '{}'", i + 1, key));
        }
    }
    Some(Loaded { path, warnings })
}

fn apply(cfg: &mut Config, theme: &mut Theme, key: &str, val: &str) -> bool {
    if let Some(name) = key.strip_prefix("color.") {
        return theme.set(name, val);
    }
    match key {
        "tabsize" => match val.parse::<usize>() {
            Ok(n) => {
                cfg.tab_size = n.clamp(1, 16);
                true
            }
            Err(_) => false,
        },
        "tabs" => set_bool(val, |b| cfg.use_spaces = !b),
        "numbers" => set_bool(val, |b| cfg.show_numbers = b),
        "syntax" => set_bool(val, |b| cfg.syntax = b),
        "autoindent" => set_bool(val, |b| cfg.autoindent = b),
        "mouse" => set_bool(val, |b| cfg.mouse = b),
        "backup" => set_bool(val, |b| cfg.backup = b),
        "scrolloff" => match val.parse::<usize>() {
            Ok(n) => {
                cfg.scrolloff = n.min(99);
                true
            }
            Err(_) => false,
        },
        "cursorline" => set_bool(val, |b| cfg.cursorline = b),
        "trim_trailing" => set_bool(val, |b| cfg.trim_trailing = b),
        "final_newline" => set_bool(val, |b| cfg.final_newline = b),
        "ignorecase" => set_bool(val, |b| cfg.ignorecase = b),
        "smartcase" => set_bool(val, |b| cfg.smartcase = b),
        _ => false,
    }
}

fn set_bool(val: &str, mut f: impl FnMut(bool)) -> bool {
    match parse_bool(val) {
        Some(b) => {
            f(b);
            true
        }
        None => false,
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}
