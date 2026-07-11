use std::path::Path;

mod generic;
mod python;
mod uniland;

// Semantic token classes. The theme turns these into colors; highlighters only
// ever speak in these terms so a new theme never touches the scanners.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Text,
    Keyword,
    Builtin,
    Type,
    Function,
    String,
    Template,
    Interp,
    Number,
    Constant,
    Comment,
    Operator,
    Punct,
    Escape,
    Preproc,
    #[allow(dead_code)]
    Error,
}

// A highlighter colors one line at a time. `state` is an opaque carry value:
// the highlighter defines its own meaning (in a block comment, inside a triple
// string, how deep a python block is, ...). The editor only threads it through
// and never inspects it. The returned Vec has exactly one Style per char.
pub trait Highlighter {
    fn line(&self, text: &str, state: u32) -> (Vec<Style>, u32);
}

struct Plain;
impl Highlighter for Plain {
    fn line(&self, text: &str, _state: u32) -> (Vec<Style>, u32) {
        (vec![Style::Text; text.chars().count()], 0)
    }
}

pub fn plain() -> Box<dyn Highlighter> {
    Box::new(Plain)
}

// Pick a highlighter for a file. Extension first, then a couple of well-known
// filenames, then the shebang. Unknown files get the plain one — better no
// colors than wrong colors.
pub fn detect(path: Option<&Path>, first_line: &str) -> Box<dyn Highlighter> {
    if let Some(p) = path {
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            match name {
                "Makefile" | "makefile" | "GNUmakefile" => return generic::make(),
                "Dockerfile" => return generic::dockerfile(),
                "Cargo.toml" | "Cargo.lock" => return generic::toml(),
                _ => {}
            }
        }
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if let Some(h) = by_ext(&ext.to_ascii_lowercase()) {
                return h;
            }
        }
    }
    if let Some(h) = by_shebang(first_line) {
        return h;
    }
    plain()
}

fn by_ext(ext: &str) -> Option<Box<dyn Highlighter>> {
    Some(match ext {
        "uni" => Box::new(uniland::Uniland),
        "py" | "pyw" | "pyi" => Box::new(python::Python),
        "rs" => generic::rust(),
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => generic::clike(),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => generic::javascript(),
        "go" => generic::go(),
        "json" | "jsonc" => generic::json(),
        "sh" | "bash" | "zsh" | "ksh" => generic::shell(),
        "toml" => generic::toml(),
        "yml" | "yaml" => generic::yaml(),
        "html" | "htm" | "xml" => generic::markup(),
        "css" | "scss" => generic::css(),
        "lua" => generic::lua(),
        _ => return None,
    })
}

fn by_shebang(first: &str) -> Option<Box<dyn Highlighter>> {
    let first = first.trim_start();
    if !first.starts_with("#!") {
        return None;
    }
    if first.contains("python") {
        Some(Box::new(python::Python))
    } else if first.contains("bash") || first.contains("/sh") || first.contains("zsh") {
        Some(generic::shell())
    } else if first.contains("node") {
        Some(generic::javascript())
    } else {
        None
    }
}

// Shared helpers the scanners lean on.
pub fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
pub fn is_word_cont(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
