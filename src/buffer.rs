// Text storage plus undo. Lines never carry their trailing newline; the newline
// only exists between them. Everything the editor does eventually turns into an
// insert or a delete here, and every insert/delete records how to undo itself.

pub type Pos = (usize, usize); // (row, char-column)

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Type,
    Del,
    Other,
}

enum Prim {
    Ins(Pos, String),
    Del(Pos, String),
}

struct Group {
    edits: Vec<Prim>,
    before: Pos,
    after: Pos,
    kind: Kind,
}

pub struct Buffer {
    pub lines: Vec<String>,
    pub dirty: bool,
    pub crlf: bool,
    pub final_newline: bool,
    undo: Vec<Group>,
    redo: Vec<Group>,
    // Lowest line whose highlight state may have changed since last synced.
    min_dirty: Option<usize>,
}

impl Buffer {
    pub fn from_str(text: &str) -> Buffer {
        let crlf = text.contains("\r\n");
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let final_newline = text.ends_with('\n');
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        // split leaves a trailing "" for a final newline — drop it, we remember it.
        if final_newline && lines.len() > 1 {
            lines.pop();
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        Buffer {
            lines,
            dirty: false,
            crlf,
            final_newline,
            undo: Vec::new(),
            redo: Vec::new(),
            min_dirty: None,
        }
    }

    pub fn empty() -> Buffer {
        Buffer::from_str("")
    }

    #[allow(dead_code)] // used by tests; the editor writes via output_text()
    pub fn to_text(&self) -> String {
        let joiner = if self.crlf { "\r\n" } else { "\n" };
        let mut out = self.lines.join(joiner);
        if self.final_newline {
            out.push_str(joiner);
        }
        out
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map(|l| l.chars().count()).unwrap_or(0)
    }

    pub fn take_min_dirty(&mut self) -> Option<usize> {
        self.min_dirty.take()
    }

    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    #[allow(dead_code)]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // --- public edits (recorded) ------------------------------------------

    pub fn insert(&mut self, at: Pos, text: &str, cursor_before: Pos, kind: Kind) -> Pos {
        let end = self.raw_insert(at, text);
        self.dirty = true;
        self.record(Prim::Ins(at, text.to_string()), cursor_before, end, kind);
        end
    }

    pub fn delete(
        &mut self,
        a: Pos,
        b: Pos,
        cursor_before: Pos,
        cursor_after: Pos,
        kind: Kind,
    ) -> String {
        let (a, b) = order(a, b);
        if a == b {
            return String::new(); // nothing to remove — don't dirty or record
        }
        let removed = self.raw_delete(a, b);
        self.dirty = true;
        self.record(
            Prim::Del(a, removed.clone()),
            cursor_before,
            cursor_after,
            kind,
        );
        removed
    }

    pub fn undo(&mut self) -> Option<Pos> {
        let g = self.undo.pop()?;
        for prim in g.edits.iter().rev() {
            match prim {
                Prim::Ins(at, t) => {
                    self.raw_delete(*at, advance(*at, t));
                }
                Prim::Del(at, t) => {
                    self.raw_insert(*at, t);
                }
            }
        }
        self.dirty = true;
        let cur = g.before;
        self.redo.push(g);
        Some(cur)
    }

    pub fn redo(&mut self) -> Option<Pos> {
        let g = self.redo.pop()?;
        for prim in g.edits.iter() {
            match prim {
                Prim::Ins(at, t) => {
                    self.raw_insert(*at, t);
                }
                Prim::Del(at, t) => {
                    self.raw_delete(*at, advance(*at, t));
                }
            }
        }
        self.dirty = true;
        let cur = g.after;
        self.undo.push(g);
        Some(cur)
    }

    // Force the next edit to start a fresh undo group. Called whenever the
    // cursor moves on its own or the command changes, so typing coalesces but
    // "type, move, type" stays three sensible undo steps.
    pub fn seal(&mut self) {
        if let Some(g) = self.undo.last_mut() {
            g.kind = Kind::Other;
        }
    }

    // --- recording --------------------------------------------------------

    fn record(&mut self, prim: Prim, before: Pos, after: Pos, kind: Kind) {
        self.redo.clear();
        if kind != Kind::Other {
            if let Some(g) = self.undo.last_mut() {
                if g.kind == kind && can_merge(g, &prim) {
                    g.edits.push(prim);
                    g.after = after;
                    return;
                }
            }
        }
        self.undo.push(Group {
            edits: vec![prim],
            before,
            after,
            kind,
        });
    }

    // --- raw mutation (no undo) -------------------------------------------

    fn touch(&mut self, row: usize) {
        self.min_dirty = Some(match self.min_dirty {
            Some(r) => r.min(row),
            None => row,
        });
    }

    fn raw_insert(&mut self, at: Pos, text: &str) -> Pos {
        let (row, col) = at;
        self.touch(row);
        if !text.contains('\n') {
            let b = byte_at(&self.lines[row], col);
            self.lines[row].insert_str(b, text);
            return (row, col + text.chars().count());
        }
        let line = self.lines[row].clone();
        let b = byte_at(&line, col);
        let head = &line[..b];
        let tail = &line[b..];
        let parts: Vec<&str> = text.split('\n').collect();
        let mut fresh: Vec<String> = Vec::with_capacity(parts.len());
        fresh.push(format!("{}{}", head, parts[0]));
        for p in &parts[1..parts.len() - 1] {
            fresh.push((*p).to_string());
        }
        let last = parts[parts.len() - 1];
        let last_col = last.chars().count();
        fresh.push(format!("{}{}", last, tail));
        let end_row = row + fresh.len() - 1;
        self.lines.splice(row..row + 1, fresh);
        (end_row, last_col)
    }

    fn raw_delete(&mut self, a: Pos, b: Pos) -> String {
        let (r1, c1) = a;
        let (r2, c2) = b;
        self.touch(r1);
        if r1 == r2 {
            let line = &mut self.lines[r1];
            let bs = byte_at(line, c1);
            let be = byte_at(line, c2);
            let removed = line[bs..be].to_string();
            line.replace_range(bs..be, "");
            return removed;
        }
        let first = self.lines[r1].clone();
        let last = self.lines[r2].clone();
        let bs = byte_at(&first, c1);
        let be = byte_at(&last, c2);
        let mut removed = String::new();
        removed.push_str(&first[bs..]);
        removed.push('\n');
        for r in r1 + 1..r2 {
            removed.push_str(&self.lines[r]);
            removed.push('\n');
        }
        removed.push_str(&last[..be]);
        let merged = format!("{}{}", &first[..bs], &last[be..]);
        self.lines.splice(r1..=r2, std::iter::once(merged));
        removed
    }
}

fn order(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn advance(start: Pos, text: &str) -> Pos {
    let nl = text.matches('\n').count();
    if nl == 0 {
        (start.0, start.1 + text.chars().count())
    } else {
        let last = text.rsplit('\n').next().unwrap_or("");
        (start.0 + nl, last.chars().count())
    }
}

// Two edits merge into one undo step when they're the natural continuation of
// each other: typing forward, or backspacing / deleting in place.
fn can_merge(g: &Group, prim: &Prim) -> bool {
    let last = g.edits.last();
    match (last, prim, g.kind) {
        (Some(Prim::Ins(at, t)), Prim::Ins(nat, nt), Kind::Type) => {
            !t.contains('\n') && !nt.contains('\n') && advance(*at, t) == *nat
        }
        (Some(Prim::Del(at, t)), Prim::Del(nat, nt), Kind::Del) => {
            // backspace: new deletion ends where the last one began
            let backspace = nat.0 == at.0 && nat.1 + nt.chars().count() == at.1;
            // delete-forward: both at the same spot
            let forward = nat == at;
            (backspace || forward) && !t.contains('\n') && !nt.contains('\n')
        }
        _ => false,
    }
}

fn byte_at(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(b, _)| b).unwrap_or(s.len())
}
