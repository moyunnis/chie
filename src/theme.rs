use std::collections::HashMap;

use crossterm::style::Color;

use crate::highlight::Style;

// A theme is just a lookup from semantic style to (color, bold), plus the chrome
// colors for the bars, gutter and highlights. Everything here can be overridden
// from the config file, so a fork or a user never edits Rust to reskin chie.
#[derive(Clone)]
pub struct Theme {
    styles: HashMap<Style, (Color, bool)>,
    pub bar_fg: Color,
    pub bar_bg: Color,
    pub key_fg: Color,
    pub key_bg: Color,
    pub gutter: Color,
    pub gutter_current: Color,
    pub selection_bg: Color,
    pub match_fg: Color,
    pub match_bg: Color,
    pub cursorline_bg: Color,
}

impl Theme {
    pub fn paint(&self, s: Style) -> (Color, bool) {
        self.styles
            .get(&s)
            .copied()
            .unwrap_or((Color::Reset, false))
    }

    // Apply `color.<name> = <value>` from the config. Returns false for an
    // unknown name so the loader can warn instead of silently ignoring a typo.
    pub fn set(&mut self, name: &str, value: &str) -> bool {
        use Style::*;
        if let Some(style) = match name {
            "text" => Some(Text),
            "keyword" => Some(Keyword),
            "builtin" => Some(Builtin),
            "type" => Some(Type),
            "function" => Some(Function),
            "string" => Some(String),
            "template" => Some(Template),
            "interp" => Some(Interp),
            "number" => Some(Number),
            "constant" => Some(Constant),
            "comment" => Some(Comment),
            "operator" => Some(Operator),
            "punct" => Some(Punct),
            "escape" => Some(Escape),
            "preproc" => Some(Preproc),
            _ => None,
        } {
            match parse_value(value) {
                Some(v) => {
                    self.styles.insert(style, v);
                    true
                }
                None => false,
            }
        } else {
            let color = match parse_color(value) {
                Some(c) => c,
                None => return false,
            };
            let slot = match name {
                "bar_fg" => &mut self.bar_fg,
                "bar_bg" => &mut self.bar_bg,
                "key_fg" => &mut self.key_fg,
                "key_bg" => &mut self.key_bg,
                "gutter" => &mut self.gutter,
                "gutter_current" => &mut self.gutter_current,
                "selection" => &mut self.selection_bg,
                "match_fg" => &mut self.match_fg,
                "match_bg" => &mut self.match_bg,
                "cursorline" => &mut self.cursorline_bg,
                _ => return false,
            };
            *slot = color;
            true
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        use Style::*;
        let mut styles = HashMap::new();
        styles.insert(Text, (Color::Reset, false));
        styles.insert(Keyword, (Color::Blue, true));
        styles.insert(Builtin, (Color::Cyan, false));
        styles.insert(Type, (Color::DarkCyan, true));
        styles.insert(Function, (Color::Yellow, false));
        styles.insert(String, (Color::Green, false));
        styles.insert(Template, (Color::Green, false));
        styles.insert(Interp, (Color::Yellow, true));
        styles.insert(Number, (Color::Magenta, false));
        styles.insert(Constant, (Color::Magenta, true));
        styles.insert(Comment, (Color::DarkGrey, false));
        styles.insert(Operator, (Color::Red, false));
        styles.insert(Punct, (Color::Reset, false));
        styles.insert(Escape, (Color::Yellow, true));
        styles.insert(Preproc, (Color::Magenta, false));
        styles.insert(Error, (Color::Red, true));
        Theme {
            styles,
            bar_fg: Color::Black,
            bar_bg: Color::Grey,
            key_fg: Color::White,
            key_bg: Color::DarkBlue,
            gutter: Color::DarkGrey,
            gutter_current: Color::Yellow,
            selection_bg: Color::DarkBlue,
            match_fg: Color::Black,
            match_bg: Color::DarkYellow,
            cursorline_bg: Color::AnsiValue(236),
        }
    }
}

// "<color> [bold]" — e.g. "brightblue bold", "#ff8800", "212", "green".
pub fn parse_value(s: &str) -> Option<(Color, bool)> {
    let mut bold = false;
    let mut color = None;
    for tok in s.split_whitespace() {
        if tok.eq_ignore_ascii_case("bold") {
            bold = true;
        } else {
            color = Some(parse_color(tok)?);
        }
    }
    Some((color?, bold))
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb { r, g, b });
        }
        return None;
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Color::AnsiValue(n));
    }
    // Names map the way people expect: "red" is red, "brightred" is the bold
    // variant. crossterm's own naming (Dark* = normal) is the opposite, hence
    // the manual table.
    Some(match s.to_ascii_lowercase().as_str() {
        "default" | "none" | "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::DarkRed,
        "green" => Color::DarkGreen,
        "yellow" => Color::DarkYellow,
        "blue" => Color::DarkBlue,
        "magenta" | "purple" => Color::DarkMagenta,
        "cyan" => Color::DarkCyan,
        "white" => Color::Grey,
        "brightblack" | "gray" | "grey" => Color::DarkGrey,
        "brightred" => Color::Red,
        "brightgreen" => Color::Green,
        "brightyellow" => Color::Yellow,
        "brightblue" => Color::Blue,
        "brightmagenta" => Color::Magenta,
        "brightcyan" => Color::Cyan,
        "brightwhite" => Color::White,
        _ => return None,
    })
}
