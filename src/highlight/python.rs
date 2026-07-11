use super::{is_word_cont, is_word_start, Highlighter, Style};

pub struct Python;

// state: 0 normal, 1 inside ''' ..., 2 inside """ ...
const S_NORMAL: u32 = 0;
const S_TRIPLE_S: u32 = 1;
const S_TRIPLE_D: u32 = 2;

const KEYWORDS: &[&str] = &[
    "def", "class", "return", "if", "elif", "else", "for", "while", "break", "continue", "try",
    "except", "finally", "raise", "with", "as", "import", "from", "pass", "lambda", "yield",
    "global", "nonlocal", "assert", "del", "in", "is", "not", "and", "or", "async", "await",
    "match", "case",
];

const CONSTANTS: &[&str] = &["None", "True", "False", "self", "cls", "__name__"];

const BUILTINS: &[&str] = &[
    "print",
    "len",
    "range",
    "int",
    "str",
    "float",
    "list",
    "dict",
    "set",
    "tuple",
    "bool",
    "bytes",
    "type",
    "input",
    "open",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sum",
    "min",
    "max",
    "abs",
    "round",
    "sorted",
    "reversed",
    "isinstance",
    "issubclass",
    "hasattr",
    "getattr",
    "setattr",
    "super",
    "repr",
    "format",
    "any",
    "all",
    "next",
    "iter",
    "vars",
    "dir",
    "id",
    "hash",
    "chr",
    "ord",
    "hex",
    "oct",
    "bin",
    "frozenset",
    "bytearray",
    "callable",
    "staticmethod",
    "classmethod",
    "property",
];

impl Highlighter for Python {
    fn line(&self, text: &str, state: u32) -> (Vec<Style>, u32) {
        let chars: Vec<char> = text.chars().collect();
        let mut st = vec![Style::Text; chars.len()];
        let end_state = scan(&chars, &mut st, state);
        (st, end_state)
    }
}

// Colors one python line into `st`, returns the state to carry into the next.
// Made public so the UniLand highlighter can reuse it inside `python { }`.
pub fn scan(chars: &[char], st: &mut [Style], state: u32) -> u32 {
    let n = chars.len();
    let mut i = 0usize;

    if state == S_TRIPLE_S || state == S_TRIPLE_D {
        let q = if state == S_TRIPLE_S { '\'' } else { '"' };
        match consume_triple(chars, 0, st, q) {
            Some(next) => i = next, // closed on this line, keep scanning
            None => {
                for s in st.iter_mut() {
                    *s = Style::String;
                }
                return state;
            }
        }
    }

    let mut expect_def = false; // just saw `def`
    let mut expect_class = false; // just saw `class`

    while i < n {
        let c = chars[i];

        if c == '#' {
            for s in st.iter_mut().skip(i) {
                *s = Style::Comment;
            }
            return S_NORMAL;
        }

        if c == '@' && is_line_lead(chars, i) {
            st[i] = Style::Preproc;
            i += 1;
            while i < n && (is_word_cont(chars[i]) || chars[i] == '.') {
                st[i] = Style::Preproc;
                i += 1;
            }
            continue;
        }

        if c == '"' || c == '\'' {
            if let Some(carry) = try_string(chars, i, st) {
                match carry {
                    StrEnd::Closed(next) => {
                        i = next;
                        expect_def = false;
                        expect_class = false;
                        continue;
                    }
                    StrEnd::OpenTriple => {
                        return if c == '\'' { S_TRIPLE_S } else { S_TRIPLE_D };
                    }
                }
            }
        }

        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            i = scan_number(chars, i, st);
            continue;
        }

        if is_word_start(c) {
            let start = i;
            i += 1;
            while i < n && is_word_cont(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            // string prefixes: r"" b'' f"" rb"" ...
            if i < n && (chars[i] == '"' || chars[i] == '\'') && is_str_prefix(&word) {
                if let Some(carry) = try_string(chars, i, st) {
                    for s in st.iter_mut().take(i).skip(start) {
                        *s = Style::String;
                    }
                    match carry {
                        StrEnd::Closed(next) => {
                            i = next;
                            continue;
                        }
                        StrEnd::OpenTriple => {
                            return if chars[i] == '\'' {
                                S_TRIPLE_S
                            } else {
                                S_TRIPLE_D
                            };
                        }
                    }
                }
            }

            let style = if expect_class {
                Style::Type
            } else if expect_def {
                Style::Function
            } else {
                classify(&word, chars, i)
            };
            for s in st.iter_mut().take(i).skip(start) {
                *s = style;
            }
            expect_def = word == "def";
            expect_class = word == "class";
            continue;
        }

        if is_operator(c) {
            st[i] = Style::Operator;
        } else if "()[]{},:;.".contains(c) {
            st[i] = Style::Punct;
        }
        i += 1;
    }
    S_NORMAL
}

// Exposed so the UniLand highlighter can color identifiers inside `python { }`.
pub fn word_style(word: &str, chars: &[char], after: usize) -> Style {
    classify(word, chars, after)
}

fn classify(word: &str, chars: &[char], after: usize) -> Style {
    if KEYWORDS.contains(&word) {
        Style::Keyword
    } else if CONSTANTS.contains(&word) {
        Style::Constant
    } else if BUILTINS.contains(&word) {
        Style::Builtin
    } else if next_nonspace_is(chars, after, '(') {
        Style::Function
    } else {
        Style::Text
    }
}

enum StrEnd {
    Closed(usize),
    OpenTriple,
}

// i points at a quote. Colors the string; returns whether it closed on this line
// or opened a triple that spills over. Returns None only if i wasn't a quote.
fn try_string(chars: &[char], i: usize, st: &mut [Style]) -> Option<StrEnd> {
    let n = chars.len();
    let q = chars[i];
    if q != '"' && q != '\'' {
        return None;
    }
    let triple = i + 2 < n && chars[i + 1] == q && chars[i + 2] == q;
    if triple {
        st[i] = Style::String;
        st[i + 1] = Style::String;
        st[i + 2] = Style::String;
        match consume_triple(chars, i + 3, st, q) {
            Some(next) => Some(StrEnd::Closed(next)),
            None => Some(StrEnd::OpenTriple),
        }
    } else {
        let mut j = i;
        st[j] = Style::String;
        j += 1;
        while j < n {
            if chars[j] == '\\' {
                st[j] = Style::Escape;
                if j + 1 < n {
                    st[j + 1] = Style::Escape;
                }
                j += 2;
                continue;
            }
            st[j] = Style::String;
            if chars[j] == q {
                j += 1;
                break;
            }
            j += 1;
        }
        Some(StrEnd::Closed(j))
    }
}

// Colors from `i` until a closing triple `qqq`. Returns index past it, or None
// if the line ends first (string continues).
fn consume_triple(chars: &[char], mut i: usize, st: &mut [Style], q: char) -> Option<usize> {
    let n = chars.len();
    while i < n {
        if chars[i] == q && chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
            st[i] = Style::String;
            st[i + 1] = Style::String;
            st[i + 2] = Style::String;
            return Some(i + 3);
        }
        st[i] = Style::String;
        i += 1;
    }
    None
}

fn scan_number(chars: &[char], mut i: usize, st: &mut [Style]) -> usize {
    let n = chars.len();
    let hex = i + 1 < n && chars[i] == '0' && (chars[i + 1] == 'x' || chars[i + 1] == 'X');
    if hex {
        st[i] = Style::Number;
        st[i + 1] = Style::Number;
        i += 2;
        while i < n && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
            st[i] = Style::Number;
            i += 1;
        }
        return i;
    }
    while i < n {
        let c = chars[i];
        let ok = c.is_ascii_digit()
            || c == '.'
            || c == '_'
            || c == 'e'
            || c == 'E'
            || ((c == '+' || c == '-') && i > 0 && (chars[i - 1] == 'e' || chars[i - 1] == 'E'));
        if !ok {
            break;
        }
        st[i] = Style::Number;
        i += 1;
    }
    i
}

fn is_str_prefix(w: &str) -> bool {
    let w = w.to_ascii_lowercase();
    matches!(
        w.as_str(),
        "r" | "b" | "f" | "u" | "rb" | "br" | "fr" | "rf" | "bf"
    )
}

fn is_operator(c: char) -> bool {
    "+-*/%=<>!&|^~@".contains(c)
}

fn is_line_lead(chars: &[char], i: usize) -> bool {
    chars[..i].iter().all(|c| c.is_whitespace())
}

fn next_nonspace_is(chars: &[char], from: usize, target: char) -> bool {
    chars[from..]
        .iter()
        .find(|c| !c.is_whitespace())
        .map(|c| *c == target)
        .unwrap_or(false)
}
