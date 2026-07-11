use super::python;
use super::{is_word_cont, is_word_start, Highlighter, Style};

pub struct Uniland;

// Carry states. PYBLOCK stashes the brace depth in the high bits so a nested
// `{ }` inside embedded python doesn't close the block early.
const NORMAL: u32 = 0;
const BLOCK_COMMENT: u32 = 1;
const TEMPLATE: u32 = 2;
const PYBLOCK: u32 = 3;

// Straight from the UniLand lexer — only the structural keywords live here.
const KEYWORDS: &[&str] = &[
    "let", "const", "func", "fn", "return", "if", "else", "for", "while", "break", "continue",
    "try", "catch", "finally", "throw", "in", "python",
];

const CONSTANTS: &[&str] = &["true", "false", "null"];

// The standard library. These are plain identifiers at runtime, but coloring
// them makes a UniLand file read the way it does in your head.
const BUILTINS: &[&str] = &[
    "print",
    "output",
    "input",
    "log_info",
    "log_debug",
    "log_warn",
    "log_error",
    "log_fatal",
    "typeof",
    "to_string",
    "to_number",
    "to_boolean",
    "is_number",
    "is_string",
    "is_array",
    "is_object",
    "is_boolean",
    "is_null",
    "is_function",
    "length",
    "split",
    "join",
    "replace",
    "substring",
    "trim",
    "uppercase",
    "lowercase",
    "startswith",
    "endswith",
    "repeat",
    "char_code",
    "from_char_code",
    "includes",
    "indexOf",
    "push",
    "pop",
    "shift",
    "unshift",
    "slice",
    "splice",
    "reverse",
    "sort",
    "map",
    "filter",
    "reduce",
    "find",
    "each",
    "sum",
    "range",
    "keys",
    "values",
    "entries",
    "has",
    "merge",
    "clone",
    "abs",
    "round",
    "floor",
    "ceil",
    "sqrt",
    "pow",
    "min",
    "max",
    "random",
    "random_int",
    "regex_match",
    "regex_findall",
    "regex_replace",
    "read_file",
    "write_file",
    "append_file",
    "file_exists",
    "delete_file",
    "list_dir",
    "create_dir",
    "copy_file",
    "rename_file",
    "file_info",
    "open_file",
    "open_url",
    "zip",
    "unzip",
    "zip_list",
    "read_base64",
    "write_base64",
    "base64_encode",
    "base64_decode",
    "get_time",
    "get_date",
    "sleep",
    "format_time",
    "parse_json",
    "stringify_json",
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "system",
    "env",
    "argv",
    "exit",
    "pyeval",
    "pyexec",
    "py",
    "gui_window",
    "gui_label",
    "gui_input",
    "gui_button",
    "gui_get",
    "gui_set",
    "gui_run",
    "gui_close",
    "gui_on_close",
    "gui_image",
    "gui_image_button",
    "gui_gif",
    "gui_video",
    "pick_file",
    "pick_save",
    "pick_folder",
    "alert",
    "confirm",
    "prompt",
];

impl Highlighter for Uniland {
    fn line(&self, text: &str, state: u32) -> (Vec<Style>, u32) {
        let chars: Vec<char> = text.chars().collect();
        let mut st = vec![Style::Text; chars.len()];

        let mode = state & 0xff;
        let start = match mode {
            BLOCK_COMMENT => match block_comment(&chars, 0, &mut st) {
                Some(k) => k,
                None => return (st, BLOCK_COMMENT),
            },
            TEMPLATE => {
                let (k, closed) = template_body(&chars, 0, &mut st);
                if !closed {
                    return (st, TEMPLATE);
                }
                k
            }
            PYBLOCK => {
                let depth = state >> 8;
                let (k, done, depth) = pyblock(&chars, 0, &mut st, depth);
                if !done {
                    return (st, PYBLOCK | (depth << 8));
                }
                k
            }
            _ => 0,
        };

        let end = scan(&chars, start, &mut st);
        (st, end)
    }
}

fn scan(chars: &[char], mut i: usize, st: &mut [Style]) -> u32 {
    let n = chars.len();
    while i < n {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // // and # both open a line comment in UniLand.
        if c == '#' || (c == '/' && chars.get(i + 1) == Some(&'/')) {
            for s in st.iter_mut().skip(i) {
                *s = Style::Comment;
            }
            return NORMAL;
        }

        if c == '/' && chars.get(i + 1) == Some(&'*') {
            st[i] = Style::Comment;
            st[i + 1] = Style::Comment;
            match block_comment(chars, i + 2, st) {
                Some(k) => {
                    i = k;
                    continue;
                }
                None => return BLOCK_COMMENT,
            }
        }

        if c == '"' {
            i = string(chars, i, st);
            continue;
        }

        if c == '`' {
            st[i] = Style::Template;
            let (j, closed) = template_body(chars, i + 1, st);
            if !closed {
                return TEMPLATE;
            }
            i = j;
            continue;
        }

        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            i = number(chars, i, st);
            continue;
        }

        if is_word_start(c) {
            let start = i;
            i += 1;
            while i < n && is_word_cont(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            // `python {` opens an embedded block — color the rest as python
            // until the matching brace, possibly spilling to later lines.
            if word == "python" {
                let mut k = i;
                while k < n && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < n && chars[k] == '{' {
                    fill(st, start, i, Style::Keyword);
                    st[k] = Style::Punct;
                    let (j, done, depth) = pyblock(chars, k + 1, st, 1);
                    if !done {
                        return PYBLOCK | (depth << 8);
                    }
                    i = j;
                    continue;
                }
            }

            let style = classify(&word, chars, i);
            fill(st, start, i, style);
            continue;
        }

        if is_operator(c) {
            st[i] = Style::Operator;
        } else if is_punct(c) {
            st[i] = Style::Punct;
        }
        i += 1;
    }
    NORMAL
}

fn classify(word: &str, chars: &[char], after: usize) -> Style {
    if CONSTANTS.contains(&word) {
        Style::Constant
    } else if KEYWORDS.contains(&word) {
        Style::Keyword
    } else if BUILTINS.contains(&word) {
        Style::Builtin
    } else if next_nonspace_is(chars, after, '(') {
        Style::Function
    } else {
        Style::Text
    }
}

// Marks a `"..."` string with escapes. Returns index past the closing quote (or
// end of line if the source is malformed — the editor shouldn't panic on that).
fn string(chars: &[char], mut i: usize, st: &mut [Style]) -> usize {
    let n = chars.len();
    st[i] = Style::String;
    i += 1;
    while i < n {
        if chars[i] == '\\' {
            st[i] = Style::Escape;
            if i + 1 < n {
                st[i + 1] = Style::Escape;
            }
            i += 2;
            continue;
        }
        st[i] = Style::String;
        if chars[i] == '"' {
            return i + 1;
        }
        i += 1;
    }
    i
}

// Backtick template. `{expr}` is interpolation, `{{`/`}}` are literal braces.
// Returns (index, closed) — closed=false means it runs onto the next line.
fn template_body(chars: &[char], mut i: usize, st: &mut [Style]) -> (usize, bool) {
    let n = chars.len();
    while i < n {
        let c = chars[i];
        if c == '\\' {
            st[i] = Style::Escape;
            if i + 1 < n {
                st[i + 1] = Style::Escape;
            }
            i += 2;
            continue;
        }
        if c == '`' {
            st[i] = Style::Template;
            return (i + 1, true);
        }
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                st[i] = Style::Template;
                st[i + 1] = Style::Template;
                i += 2;
                continue;
            }
            st[i] = Style::Interp;
            i += 1;
            while i < n && chars[i] != '}' {
                st[i] = Style::Interp;
                i += 1;
            }
            if i < n {
                st[i] = Style::Interp;
                i += 1;
            }
            continue;
        }
        if c == '}' && chars.get(i + 1) == Some(&'}') {
            st[i] = Style::Template;
            st[i + 1] = Style::Template;
            i += 2;
            continue;
        }
        st[i] = Style::Template;
        i += 1;
    }
    (n, false)
}

// Colors an embedded python block, tracking brace depth so the matching `}`
// closes it. Returns (index, done, depth). done=true means we found the closer.
fn pyblock(chars: &[char], mut i: usize, st: &mut [Style], mut depth: u32) -> (usize, bool, u32) {
    let n = chars.len();
    while i < n {
        let c = chars[i];

        if c == '#' {
            for s in st.iter_mut().skip(i) {
                *s = Style::Comment;
            }
            return (n, false, depth);
        }

        if c == '"' || c == '\'' {
            let q = c;
            st[i] = Style::String;
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    st[i] = Style::Escape;
                    if i + 1 < n {
                        st[i + 1] = Style::Escape;
                    }
                    i += 2;
                    continue;
                }
                st[i] = Style::String;
                if chars[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if c == '{' {
            depth += 1;
            st[i] = Style::Punct;
            i += 1;
            continue;
        }
        if c == '}' {
            depth = depth.saturating_sub(1);
            st[i] = Style::Punct;
            i += 1;
            if depth == 0 {
                return (i, true, 0);
            }
            continue;
        }

        if c.is_ascii_digit() {
            i = number(chars, i, st);
            continue;
        }

        if is_word_start(c) {
            let start = i;
            i += 1;
            while i < n && is_word_cont(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let style = python::word_style(&word, chars, i);
            fill(st, start, i, style);
            continue;
        }

        if "+-*/%=<>!&|^~@".contains(c) {
            st[i] = Style::Operator;
        } else if "()[],:;.".contains(c) {
            st[i] = Style::Punct;
        }
        i += 1;
    }
    (n, false, depth)
}

fn block_comment(chars: &[char], mut i: usize, st: &mut [Style]) -> Option<usize> {
    let n = chars.len();
    while i < n {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            st[i] = Style::Comment;
            st[i + 1] = Style::Comment;
            return Some(i + 2);
        }
        st[i] = Style::Comment;
        i += 1;
    }
    None
}

fn number(chars: &[char], mut i: usize, st: &mut [Style]) -> usize {
    let n = chars.len();
    let mut dot = false;
    while i < n {
        let c = chars[i];
        if c.is_ascii_digit() {
            st[i] = Style::Number;
        } else if c == '.' && !dot {
            dot = true;
            st[i] = Style::Number;
        } else {
            break;
        }
        i += 1;
    }
    i
}

fn fill(st: &mut [Style], from: usize, to: usize, style: Style) {
    for s in st.iter_mut().take(to).skip(from) {
        *s = style;
    }
}

fn is_operator(c: char) -> bool {
    "+-*/%^=<>!&|".contains(c)
}
fn is_punct(c: char) -> bool {
    "()[]{},.;:".contains(c)
}
fn next_nonspace_is(chars: &[char], from: usize, target: char) -> bool {
    chars[from..]
        .iter()
        .find(|c| !c.is_whitespace())
        .map(|c| *c == target)
        .unwrap_or(false)
}
