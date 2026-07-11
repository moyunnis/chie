mod about;
mod buffer;
mod config;
mod editor;
mod highlight;
mod theme;
mod width;

use std::path::PathBuf;
use std::process::exit;

use config::Config;
use editor::Editor;
use theme::Theme;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config::default();
    let mut theme = Theme::default();

    // Precedence: built-in defaults < config file < command-line flags.
    let loaded = config::load(&mut cfg, &mut theme);
    let mut file: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "-v" | "--version" => {
                println!("chie {}", about::VERSION);
                return;
            }
            "--config" => {
                match config::config_path() {
                    Some(p) => {
                        let state = if p.exists() {
                            "exists"
                        } else {
                            "not created yet"
                        };
                        println!("{} ({})", p.display(), state);
                    }
                    None => println!("chie: could not resolve a config location"),
                }
                return;
            }
            "--no-syntax" => cfg.syntax = false,
            "--no-mouse" => cfg.mouse = false,
            "--no-numbers" => cfg.show_numbers = false,
            "--no-autoindent" => cfg.autoindent = false,
            "--tabs" => cfg.use_spaces = false,
            "--tabsize" => {
                if let Some(n) = it.next().and_then(|s| s.parse::<usize>().ok()) {
                    cfg.tab_size = n.clamp(1, 16);
                }
            }
            s if s.starts_with("--tabsize=") => {
                if let Ok(n) = s["--tabsize=".len()..].parse::<usize>() {
                    cfg.tab_size = n.clamp(1, 16);
                }
            }
            s if s.starts_with('-') && s.len() > 1 => {
                eprintln!("chie: unknown option '{}'. Try --help.", s);
                exit(2);
            }
            s => file = Some(PathBuf::from(s)),
        }
    }

    // Even on a panic we want the terminal handed back sane, not stuck in raw
    // mode with the alternate screen up.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(
            out,
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
            crossterm::cursor::Show
        );
        let _ = crossterm::terminal::disable_raw_mode();
        default_hook(info);
    }));

    let mut ed = match Editor::open(file, cfg, theme) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("chie: {}", e);
            exit(1);
        }
    };
    if let Some(l) = loaded {
        if !l.warnings.is_empty() {
            let extra = match l.warnings.len() {
                1 => String::new(),
                n => format!(" (+{} more)", n - 1),
            };
            ed.note(format!("config: {}{}", l.warnings[0], extra));
        }
    }
    if let Err(e) = ed.run() {
        eprintln!("chie: {}", e);
        exit(1);
    }
}

fn print_usage() {
    println!(
        "chie {ver} — a nano killer with first-class UniLand highlighting

USAGE:
    chie [OPTIONS] [FILE]

OPTIONS:
    -h, --help            show this help
    -v, --version         print version
        --config          print the config file path and exit
        --tabsize <N>     spaces per indent step (default 4)
        --tabs            indent with real tabs instead of spaces
        --no-syntax       start with syntax highlighting off
        --no-numbers      start with line numbers off
        --no-autoindent   don't copy indentation onto new lines
        --no-mouse        disable mouse capture

Config lives at ~/.config/chie/config (see --config). It sets behaviour and
your own colors, Ghostty-style: e.g. `color.keyword = brightmagenta bold`.

Open chie, then press Ctrl+G for the keybindings or Alt+A for the about box.",
        ver = about::VERSION
    );
}

#[cfg(test)]
mod tests {
    use crate::buffer::{Buffer, Kind};
    use crate::highlight::{self, Highlighter, Style};
    use std::path::Path;

    fn uni() -> Box<dyn Highlighter> {
        highlight::detect(Some(Path::new("x.uni")), "")
    }

    // Every highlighter must return exactly one style per character, whatever
    // the incoming state — the renderer indexes into it blindly.
    #[test]
    fn styles_cover_every_char() {
        let hl = uni();
        for line in [
            "let x = `hi {name}!`",
            "func add(a, b) { return a + b }",
            "// comment with unicode: привет",
            "/* open block",
            "python {",
            "    x = [1, 2, 3]",
            "}",
            "",
        ] {
            let (styles, _) = hl.line(line, 0);
            assert_eq!(styles.len(), line.chars().count(), "line: {line:?}");
        }
    }

    #[test]
    fn uniland_keywords_and_templates() {
        let hl = uni();
        let (s, _) = hl.line("let x = 1", 0);
        assert_eq!(s[0], Style::Keyword); // let
        assert_eq!(s[8], Style::Number); // 1

        let (s, _) = hl.line("`a {x} b`", 0);
        assert_eq!(s[0], Style::Template);
        assert_eq!(s[3], Style::Interp); // '{'
    }

    #[test]
    fn uniland_block_comment_threads_state() {
        let hl = uni();
        let (s0, st) = hl.line("code /* start", 0);
        assert!(matches!(s0[5], Style::Comment));
        assert_ne!(st, 0); // still inside the block comment
        let (s1, st2) = hl.line("still comment */ let y = 2", st);
        assert!(matches!(s1[0], Style::Comment));
        assert_eq!(st2, 0); // closed
    }

    #[test]
    fn python_triple_string_carries() {
        let hl = highlight::detect(Some(Path::new("x.py")), "");
        let (_, st) = hl.line("s = \"\"\"open", 0);
        assert_ne!(st, 0);
        let (s, st2) = hl.line("closes here\"\"\"", st);
        assert!(matches!(s[0], Style::String));
        assert_eq!(st2, 0);
    }

    #[test]
    fn buffer_insert_delete_undo_roundtrip() {
        let mut b = Buffer::from_str("hello\nworld");
        let end = b.insert((0, 5), " there", (0, 5), Kind::Type);
        assert_eq!(end, (0, 11));
        assert_eq!(b.lines[0], "hello there");
        b.seal();
        b.insert((0, 11), "!", (0, 11), Kind::Type);
        assert_eq!(b.lines[0], "hello there!");
        b.undo();
        assert_eq!(b.lines[0], "hello there");
        b.undo();
        assert_eq!(b.lines[0], "hello");
        b.redo();
        assert_eq!(b.lines[0], "hello there");
    }

    #[test]
    fn buffer_multiline_edit() {
        let mut b = Buffer::from_str("ab\ncd\nef");
        let removed = b.delete((0, 1), (2, 1), (2, 1), (0, 1), Kind::Other);
        assert_eq!(removed, "b\ncd\ne");
        assert_eq!(b.lines, vec!["af"]);
        b.undo();
        assert_eq!(b.lines, vec!["ab", "cd", "ef"]);
    }

    #[test]
    fn newline_split_and_join() {
        let mut b = Buffer::from_str("abcdef");
        let end = b.insert((0, 3), "\n", (0, 3), Kind::Other);
        assert_eq!(end, (1, 0));
        assert_eq!(b.lines, vec!["abc", "def"]);
        b.undo();
        assert_eq!(b.lines, vec!["abcdef"]);
    }

    #[test]
    fn roundtrip_preserves_trailing_newline() {
        assert_eq!(Buffer::from_str("a\nb\n").to_text(), "a\nb\n");
        assert_eq!(Buffer::from_str("a\nb").to_text(), "a\nb");
        assert_eq!(Buffer::from_str("").to_text(), "");
    }

    #[test]
    fn char_widths() {
        use crate::width::char_width;
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('я'), 1); // cyrillic is single-width
        assert_eq!(char_width('日'), 2); // CJK is double-width
        assert_eq!(char_width('😀'), 2); // emoji too
        assert_eq!(char_width('\u{0301}'), 0); // combining acute accent
    }

    #[test]
    fn theme_color_parsing() {
        use crate::theme::{parse_color, parse_value};
        use crossterm::style::Color;
        assert_eq!(
            parse_color("#ff8800"),
            Some(Color::Rgb {
                r: 255,
                g: 136,
                b: 0
            })
        );
        assert_eq!(parse_color("212"), Some(Color::AnsiValue(212)));
        assert_eq!(parse_color("brightblue"), Some(Color::Blue));
        assert_eq!(parse_color("default"), Some(Color::Reset));
        assert_eq!(parse_color("nope"), None);
        assert_eq!(parse_value("green bold"), Some((Color::DarkGreen, true)));
    }

    #[test]
    fn config_applies_settings_and_theme() {
        use crate::config;
        use crate::highlight::Style;
        use crate::theme::Theme;
        let mut cfg = config::Config::default();
        let mut theme = Theme::default();
        // reuse the same key=value machinery the loader uses
        for line in ["tabsize = 8", "mouse = off", "color.keyword = #112233 bold"] {
            let (k, v) = line.split_once('=').unwrap();
            let ok = if let Some(name) = k.trim().strip_prefix("color.") {
                theme.set(name, v.trim())
            } else {
                apply_kv(&mut cfg, k.trim(), v.trim())
            };
            assert!(ok, "line failed: {line}");
        }
        assert_eq!(cfg.tab_size, 8);
        assert!(!cfg.mouse);
        assert_eq!(
            theme.paint(Style::Keyword),
            (
                crossterm::style::Color::Rgb {
                    r: 0x11,
                    g: 0x22,
                    b: 0x33
                },
                true
            )
        );
    }

    // tiny shim so the test can drive the private settings side of the loader
    fn apply_kv(cfg: &mut crate::config::Config, k: &str, v: &str) -> bool {
        match k {
            "tabsize" => {
                cfg.tab_size = v.parse().unwrap();
                true
            }
            "mouse" => {
                cfg.mouse = v == "on" || v == "true";
                true
            }
            _ => false,
        }
    }

    #[test]
    fn config_load_end_to_end() {
        use crate::config;
        use crate::highlight::Style;
        use crate::theme::Theme;
        use crossterm::style::Color;

        // A real config file, read through the real loader via CHIE_CONFIG.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("chie-test-{}.conf", std::process::id()));
        std::fs::write(
            &path,
            "# my config\n\
             tabsize = 3\n\
             backup = on\n\
             cursorline = yes\n\
             scrolloff = 5\n\
             trim_trailing = 1\n\
             ignorecase = off\n\
             color.keyword = #abcdef\n\
             color.cursorline = 236\n\
             nonsense = 1\n",
        )
        .unwrap();
        std::env::set_var("CHIE_CONFIG", &path);

        let mut cfg = config::Config::default();
        let mut theme = Theme::default();
        let loaded = config::load(&mut cfg, &mut theme).expect("config should load");

        assert_eq!(cfg.tab_size, 3);
        assert!(cfg.backup);
        assert!(cfg.cursorline);
        assert_eq!(cfg.scrolloff, 5);
        assert!(cfg.trim_trailing);
        assert!(!cfg.ignorecase);
        assert_eq!(
            theme.paint(Style::Keyword),
            (
                Color::Rgb {
                    r: 0xab,
                    g: 0xcd,
                    b: 0xef
                },
                false
            )
        );
        assert_eq!(theme.cursorline_bg, Color::AnsiValue(236));
        assert_eq!(loaded.warnings.len(), 1); // the 'nonsense' line

        std::env::remove_var("CHIE_CONFIG");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn control_chars_have_caret_width() {
        use crate::width::{caret_letter, char_width, is_control};
        assert!(is_control('\x1b')); // ESC
        assert!(is_control('\x7f')); // DEL
        assert!(!is_control('\t')); // tab is laid out separately
        assert!(!is_control('a'));
        assert_eq!(char_width('\x1b'), 2); // renders as ^[
        assert_eq!(caret_letter('\x1b'), '['); // 0x40 + 0x1b
        assert_eq!(caret_letter('\x00'), '@');
        assert_eq!(caret_letter('\x7f'), '?');
    }

    #[test]
    fn newlines_normalized() {
        use crate::editor::normalize_newlines;
        assert_eq!(normalize_newlines("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(normalize_newlines("plain"), "plain");
    }

    #[test]
    fn shift_col_clamps() {
        use crate::editor::shift_col;
        assert_eq!(shift_col(3, 2), 5);
        assert_eq!(shift_col(3, -2), 1);
        assert_eq!(shift_col(1, -5), 0); // saturates, no underflow
    }

    #[test]
    fn empty_delete_does_not_dirty() {
        let mut b = Buffer::from_str("hello");
        let removed = b.delete((0, 2), (0, 2), (0, 2), (0, 2), Kind::Other);
        assert_eq!(removed, "");
        assert!(!b.dirty);
        assert!(!b.can_undo());
    }

    #[test]
    fn single_newline_file_roundtrips() {
        assert_eq!(Buffer::from_str("\n").to_text(), "\n");
        assert_eq!(Buffer::from_str("\n\n").to_text(), "\n\n");
    }

    #[test]
    fn atomic_write_keeps_original_on_success() {
        use crate::editor::write_atomic;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("chie-atomic-{}.txt", std::process::id()));
        std::fs::write(&path, "before").unwrap();

        write_atomic(&path, "after\nmore", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\nmore");

        let mut bak = path.clone().into_os_string();
        bak.push("~");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "before");

        // no temp file left behind
        let tmp = dir.join(format!(
            ".{}.chie-{}.tmp",
            path.file_name().unwrap().to_str().unwrap(),
            std::process::id()
        ));
        assert!(!tmp.exists());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(std::path::PathBuf::from(bak));
    }
}
