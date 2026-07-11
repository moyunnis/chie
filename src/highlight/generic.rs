use super::{is_word_cont, is_word_start, Highlighter, Style};

// A table-driven highlighter that covers the "and other languages" part. One
// Lang describes how a family looks — its words, comment markers and quotes —
// and the same scanner handles all of them. It won't win a parser contest, but
// for reading code in an editor it's honest and fast.
pub struct Lang {
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    constants: &'static [&'static str],
    line_comment: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    // language uses a leading sigil for preprocessor/attrs, e.g. C's '#'.
    preproc: Option<char>,
}

const IN_BLOCK: u32 = 1;

impl Highlighter for Lang {
    fn line(&self, text: &str, state: u32) -> (Vec<Style>, u32) {
        let chars: Vec<char> = text.chars().collect();
        let mut st = vec![Style::Text; chars.len()];
        let n = chars.len();
        let mut i = 0usize;

        if state == IN_BLOCK {
            if let Some((_, close)) = self.block {
                match self.eat_block(&chars, 0, &mut st, close) {
                    Some(k) => i = k,
                    None => return (st, IN_BLOCK),
                }
            }
        }

        while i < n {
            let c = chars[i];

            if c.is_whitespace() {
                i += 1;
                continue;
            }

            if let Some((open, close)) = self.block {
                if starts_with(&chars, i, open) {
                    for (k, ch) in open.chars().enumerate() {
                        let _ = ch;
                        st[i + k] = Style::Comment;
                    }
                    match self.eat_block(&chars, i + open.chars().count(), &mut st, close) {
                        Some(k) => {
                            i = k;
                            continue;
                        }
                        None => return (st, IN_BLOCK),
                    }
                }
            }

            if self.line_comment.iter().any(|m| starts_with(&chars, i, m)) {
                for s in st.iter_mut().skip(i) {
                    *s = Style::Comment;
                }
                return (st, 0);
            }

            if self.quotes.contains(&c) {
                i = eat_string(&chars, i, &mut st, c);
                continue;
            }

            if let Some(sig) = self.preproc {
                if c == sig && chars[..i].iter().all(|x| x.is_whitespace()) {
                    for s in st.iter_mut().skip(i) {
                        *s = Style::Preproc;
                    }
                    return (st, 0);
                }
            }

            if c.is_ascii_digit()
                || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
            {
                i = eat_number(&chars, i, &mut st);
                continue;
            }

            if is_word_start(c) {
                let start = i;
                i += 1;
                while i < n && is_word_cont(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let style = self.word(&word, &chars, i);
                for s in st.iter_mut().take(i).skip(start) {
                    *s = style;
                }
                continue;
            }

            if "+-*/%^=<>!&|~".contains(c) {
                st[i] = Style::Operator;
            } else if "()[]{},.;:".contains(c) {
                st[i] = Style::Punct;
            }
            i += 1;
        }
        (st, 0)
    }
}

impl Lang {
    fn word(&self, word: &str, chars: &[char], after: usize) -> Style {
        if self.keywords.contains(&word) {
            Style::Keyword
        } else if self.constants.contains(&word) {
            Style::Constant
        } else if self.types.contains(&word) {
            Style::Type
        } else if next_nonspace_is(chars, after, '(') {
            Style::Function
        } else {
            Style::Text
        }
    }

    fn eat_block(
        &self,
        chars: &[char],
        mut i: usize,
        st: &mut [Style],
        close: &str,
    ) -> Option<usize> {
        let n = chars.len();
        while i < n {
            if starts_with(chars, i, close) {
                for k in 0..close.chars().count() {
                    st[i + k] = Style::Comment;
                }
                return Some(i + close.chars().count());
            }
            st[i] = Style::Comment;
            i += 1;
        }
        None
    }
}

fn eat_string(chars: &[char], mut i: usize, st: &mut [Style], q: char) -> usize {
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
        if chars[i] == q {
            return i + 1;
        }
        i += 1;
    }
    i
}

fn eat_number(chars: &[char], mut i: usize, st: &mut [Style]) -> usize {
    let n = chars.len();
    if chars[i] == '0'
        && matches!(
            chars.get(i + 1),
            Some('x') | Some('X') | Some('b') | Some('o')
        )
    {
        st[i] = Style::Number;
        st[i + 1] = Style::Number;
        i += 2;
        while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            st[i] = Style::Number;
            i += 1;
        }
        return i;
    }
    let mut dot = false;
    while i < n {
        let c = chars[i];
        if c.is_ascii_digit() || c == '_' {
            st[i] = Style::Number;
        } else if c == '.' && !dot {
            dot = true;
            st[i] = Style::Number;
        } else if c == 'e' || c == 'E' {
            st[i] = Style::Number;
        } else {
            break;
        }
        i += 1;
    }
    i
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    for (k, pc) in pat.chars().enumerate() {
        if chars.get(i + k) != Some(&pc) {
            return false;
        }
    }
    true
}

fn next_nonspace_is(chars: &[char], from: usize, target: char) -> bool {
    chars[from..]
        .iter()
        .find(|c| !c.is_whitespace())
        .map(|c| *c == target)
        .unwrap_or(false)
}

// --- language tables ------------------------------------------------------

const BASE: Lang = Lang {
    keywords: &[],
    types: &[],
    constants: &[],
    line_comment: &[],
    block: None,
    quotes: &['"'],
    preproc: None,
};

macro_rules! lang {
    ($($f:ident : $v:expr),* $(,)?) => {
        // json/css specify every field, so the update is redundant there — but
        // most languages don't, and one macro beats fifteen hand-written structs.
        #[allow(clippy::needless_update)]
        Box::new(Lang { $( $f: $v, )* ..BASE })
    };
}

pub fn rust() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["as","break","const","continue","crate","dyn","else","enum",
            "extern","fn","for","if","impl","in","let","loop","match","mod","move",
            "mut","pub","ref","return","self","Self","static","struct","super",
            "trait","type","unsafe","use","where","while","async","await","macro"],
        types: &["i8","i16","i32","i64","i128","isize","u8","u16","u32","u64",
            "u128","usize","f32","f64","bool","char","str","String","Vec","Option",
            "Result","Box","Rc","Arc","HashMap","HashSet"],
        constants: &["true","false","None","Some","Ok","Err"],
        line_comment: &["//"],
        block: Some(("/*","*/")),
        quotes: &['"'],
    }
}

pub fn clike() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["auto","break","case","const","continue","default","do","else",
            "enum","extern","for","goto","if","inline","register","return","sizeof",
            "static","struct","switch","typedef","union","volatile","while","class",
            "namespace","template","public","private","protected","virtual","new",
            "delete","this","using","try","catch","throw","operator"],
        types: &["void","char","short","int","long","float","double","signed",
            "unsigned","bool","size_t","wchar_t","int8_t","int16_t","int32_t",
            "int64_t","uint8_t","uint16_t","uint32_t","uint64_t"],
        constants: &["true","false","NULL","nullptr"],
        line_comment: &["//"],
        block: Some(("/*","*/")),
        quotes: &['"','\''],
        preproc: Some('#'),
    }
}

pub fn javascript() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["var","let","const","function","return","if","else","for",
            "while","do","break","continue","switch","case","default","try","catch",
            "finally","throw","new","delete","typeof","instanceof","in","of","class",
            "extends","super","this","import","from","export","async","await","yield",
            "void","interface","type","enum","implements","public","private","readonly"],
        types: &["number","string","boolean","object","any","unknown","never","bigint","symbol"],
        constants: &["true","false","null","undefined","NaN","Infinity"],
        line_comment: &["//"],
        block: Some(("/*","*/")),
        quotes: &['"','\'','`'],
    }
}

pub fn go() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["break","case","chan","const","continue","default","defer",
            "else","fallthrough","for","func","go","goto","if","import","interface",
            "map","package","range","return","select","struct","switch","type","var"],
        types: &["bool","string","int","int8","int16","int32","int64","uint",
            "uint8","uint16","uint32","uint64","uintptr","byte","rune","float32",
            "float64","complex64","complex128","error"],
        constants: &["true","false","nil","iota"],
        line_comment: &["//"],
        block: Some(("/*","*/")),
        quotes: &['"','`'],
    }
}

pub fn json() -> Box<dyn Highlighter> {
    lang! {
        constants: &["true","false","null"],
        line_comment: &["//"],
        block: Some(("/*","*/")),
        quotes: &['"'],
    }
}

pub fn shell() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["if","then","else","elif","fi","case","esac","for","select",
            "while","until","do","done","in","function","time","coproc","return",
            "export","local","readonly","declare","source","alias","unset"],
        constants: &["true","false"],
        line_comment: &["#"],
        quotes: &['"','\''],
    }
}

pub fn make() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["ifeq","ifneq","ifdef","ifndef","else","endif","define",
            "endef","include","export","override","unexport"],
        line_comment: &["#"],
        quotes: &['"','\''],
    }
}

pub fn dockerfile() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["FROM","RUN","CMD","LABEL","EXPOSE","ENV","ADD","COPY",
            "ENTRYPOINT","VOLUME","USER","WORKDIR","ARG","ONBUILD","STOPSIGNAL",
            "HEALTHCHECK","SHELL","AS"],
        line_comment: &["#"],
        quotes: &['"','\''],
    }
}

pub fn toml() -> Box<dyn Highlighter> {
    lang! {
        constants: &["true","false"],
        line_comment: &["#"],
        quotes: &['"','\''],
    }
}

pub fn yaml() -> Box<dyn Highlighter> {
    lang! {
        constants: &["true","false","null","yes","no","on","off"],
        line_comment: &["#"],
        quotes: &['"','\''],
    }
}

pub fn markup() -> Box<dyn Highlighter> {
    lang! {
        block: Some(("<!--","-->")),
        quotes: &['"','\''],
    }
}

pub fn css() -> Box<dyn Highlighter> {
    lang! {
        block: Some(("/*","*/")),
        quotes: &['"','\''],
    }
}

pub fn lua() -> Box<dyn Highlighter> {
    lang! {
        keywords: &["and","break","do","else","elseif","end","for","function",
            "goto","if","in","local","not","or","repeat","return","then","until","while"],
        constants: &["true","false","nil"],
        line_comment: &["--"],
        block: Some(("--[[","]]")),
        quotes: &['"','\''],
    }
}
