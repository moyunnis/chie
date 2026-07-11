use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};

use crate::about;
use crate::buffer::{Buffer, Kind, Pos};
use crate::config::Config;
use crate::highlight::{self, Highlighter, Style};
use crate::theme::Theme;
use crate::width::{caret_letter, char_width, is_control};

struct Cell {
    ch: Option<char>, // None marks the trailing column of a wide (2-col) glyph
    extra: String,    // zero-width marks that compose onto this cell's glyph
    style: Style,
    sel: bool,
    mat: bool,
}

pub struct Editor {
    buf: Buffer,
    path: Option<PathBuf>,
    hl: Box<dyn Highlighter>,
    theme: Theme,
    states: Vec<u32>, // states[i] = highlighter state AFTER line i

    cur: Pos,
    goal: usize,
    top: usize,
    left: usize,

    anchor: Option<Pos>,
    mark: bool,
    cut: String,
    prev_cut: bool,
    last_was_cut: bool,

    status: String,
    error: bool,
    quit: bool,

    tab_size: usize,
    use_spaces: bool,
    show_numbers: bool,
    syntax: bool,
    autoindent: bool,
    mouse: bool,
    backup: bool,
    scrolloff: usize,
    cursorline: bool,
    trim_trailing: bool,
    final_newline: bool,
    ignorecase: bool,
    smartcase: bool,

    w: usize,
    h: usize,

    last_query: String,
    match_hi: Option<(Pos, Pos)>,
}

impl Editor {
    pub fn open(path: Option<PathBuf>, cfg: Config, theme: Theme) -> io::Result<Editor> {
        let (buf, hl, status) = match &path {
            Some(p) if p.exists() => {
                let bytes = std::fs::read(p)?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let first = text.lines().next().unwrap_or("");
                let hl = highlight::detect(Some(p), first);
                let n = text.matches('\n').count().max(1);
                (Buffer::from_str(&text), hl, format!("Read {} lines", n))
            }
            Some(p) => {
                let hl = highlight::detect(Some(p), "");
                (Buffer::empty(), hl, "New file".to_string())
            }
            None => (
                Buffer::empty(),
                highlight::plain(),
                "New buffer".to_string(),
            ),
        };

        let (w, h) = terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((80, 24));
        let (w, h) = (w.max(20), h.max(5));

        Ok(Editor {
            buf,
            path,
            hl,
            theme,
            states: Vec::new(),
            cur: (0, 0),
            goal: 0,
            top: 0,
            left: 0,
            anchor: None,
            mark: false,
            cut: String::new(),
            prev_cut: false,
            last_was_cut: false,
            status,
            error: false,
            quit: false,
            tab_size: cfg.tab_size,
            use_spaces: cfg.use_spaces,
            show_numbers: cfg.show_numbers,
            syntax: cfg.syntax,
            autoindent: cfg.autoindent,
            mouse: cfg.mouse,
            backup: cfg.backup,
            scrolloff: cfg.scrolloff,
            cursorline: cfg.cursorline,
            trim_trailing: cfg.trim_trailing,
            final_newline: cfg.final_newline,
            ignorecase: cfg.ignorecase,
            smartcase: cfg.smartcase,
            w,
            h,
            last_query: String::new(),
            match_hi: None,
        })
    }

    // A one-off message shown on the status line at startup (config warnings).
    pub fn note(&mut self, msg: String) {
        self.status = msg;
        self.error = true;
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut out = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            cursor::Show
        )?;
        if self.mouse {
            execute!(out, EnableMouseCapture)?;
        }
        let res = self.event_loop(&mut out);
        if self.mouse {
            let _ = execute!(out, DisableMouseCapture);
        }
        execute!(out, DisableBracketedPaste, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        res
    }

    fn event_loop(&mut self, out: &mut Stdout) -> io::Result<()> {
        loop {
            self.draw(out)?;
            if self.quit {
                break;
            }
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k)?,
                Event::Mouse(m) => self.on_mouse(m),
                Event::Resize(w, h) => {
                    self.w = (w as usize).max(20);
                    self.h = (h as usize).max(5);
                }
                Event::Paste(s) => self.paste_text(&s),
                _ => {}
            }
        }
        Ok(())
    }

    // --- layout -----------------------------------------------------------

    fn body_rows(&self) -> usize {
        self.h.saturating_sub(4).max(1)
    }
    fn gutter(&self) -> usize {
        if self.show_numbers {
            digits(self.buf.line_count()).max(3) + 1
        } else {
            0
        }
    }
    fn text_width(&self) -> usize {
        self.w.saturating_sub(self.gutter()).max(1)
    }

    fn disp_col(&self, pos: Pos) -> usize {
        let line = &self.buf.lines[pos.0];
        let mut d = 0;
        for (ci, ch) in line.chars().enumerate() {
            if ci >= pos.1 {
                break;
            }
            if ch == '\t' {
                d += self.tab_size - (d % self.tab_size);
            } else {
                d += char_width(ch);
            }
        }
        d
    }

    // Inverse of disp_col: which character does display column `target` land on.
    // Used to turn a mouse click into a cursor position.
    fn char_at_disp(&self, row: usize, target: usize) -> usize {
        let line = &self.buf.lines[row];
        let mut d = 0;
        for (ci, ch) in line.chars().enumerate() {
            let w = if ch == '\t' {
                self.tab_size - (d % self.tab_size)
            } else {
                char_width(ch)
            };
            if d + w > target {
                return ci;
            }
            d += w;
        }
        line.chars().count()
    }

    fn scroll(&mut self) {
        let br = self.body_rows();
        // keep `scrolloff` lines of context above/below the cursor when possible
        let off = self.scrolloff.min(br.saturating_sub(1) / 2);
        if self.cur.0 < self.top + off {
            self.top = self.cur.0.saturating_sub(off);
        }
        if self.cur.0 + off >= self.top + br {
            self.top = (self.cur.0 + off + 1).saturating_sub(br);
        }
        let dc = self.disp_col(self.cur);
        if dc < self.left {
            self.left = dc;
        }
        let tw = self.text_width();
        if dc >= self.left + tw {
            self.left = dc - tw + 1;
        }
    }

    // --- highlight state cache --------------------------------------------

    fn sync_states(&mut self, upto: usize) {
        if !self.syntax {
            return;
        }
        if let Some(r) = self.buf.take_min_dirty() {
            self.states.truncate(r);
        }
        if self.states.len() > self.buf.line_count() {
            self.states.truncate(self.buf.line_count());
        }
        let limit = upto.min(self.buf.line_count());
        while self.states.len() < limit {
            let idx = self.states.len();
            let incoming = if idx == 0 { 0 } else { self.states[idx - 1] };
            let (_, s) = self.hl.line(&self.buf.lines[idx], incoming);
            self.states.push(s);
        }
    }

    fn styles_for(&self, row: usize) -> Vec<Style> {
        let line = &self.buf.lines[row];
        if !self.syntax {
            return vec![Style::Text; line.chars().count()];
        }
        let incoming = if row == 0 {
            0
        } else {
            *self.states.get(row - 1).unwrap_or(&0)
        };
        self.hl.line(line, incoming).0
    }

    // --- drawing ----------------------------------------------------------

    fn draw(&mut self, out: &mut Stdout) -> io::Result<()> {
        self.scroll();
        let br = self.body_rows();
        self.sync_states(self.top + br);

        queue!(out, cursor::Hide, ResetColor, cursor::MoveTo(0, 0))?;
        self.draw_title(out)?;
        for i in 0..br {
            let row = self.top + i;
            queue!(out, cursor::MoveTo(0, (i + 1) as u16))?;
            self.draw_line(out, row)?;
        }
        self.draw_message(out)?;
        self.draw_shortcuts(out)?;

        let gy = 1 + (self.cur.0 - self.top);
        let gx = self.gutter() + (self.disp_col(self.cur) - self.left);
        queue!(out, cursor::MoveTo(gx as u16, gy as u16), cursor::Show)?;
        out.flush()
    }

    fn draw_title(&self, out: &mut Stdout) -> io::Result<()> {
        let name = match &self.path {
            Some(p) => p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string(),
            None => "New Buffer".to_string(),
        };
        let modified = if self.buf.dirty { " *" } else { "" };
        let left = format!(" chie {} ", about::VERSION);
        let mid = format!("{}{}", name, modified);
        let right = format!(
            " {}:{}  {}% ",
            self.cur.0 + 1,
            self.cur.1 + 1,
            if self.buf.line_count() <= 1 {
                100
            } else {
                self.cur.0 * 100 / (self.buf.line_count() - 1)
            }
        );
        let mut bar = vec![' '; self.w];
        put(&mut bar, 0, &left);
        let midp = self.w.saturating_sub(mid.chars().count()) / 2;
        put(&mut bar, midp, &mid);
        put(
            &mut bar,
            self.w.saturating_sub(right.chars().count()),
            &right,
        );
        let s: String = bar.into_iter().collect();
        queue!(
            out,
            SetBackgroundColor(self.theme.bar_bg),
            SetForegroundColor(self.theme.bar_fg),
            Print(s),
            ResetColor
        )
    }

    fn draw_line(&self, out: &mut Stdout, row: usize) -> io::Result<()> {
        if row >= self.buf.line_count() {
            queue!(out, Clear(ClearType::UntilNewLine))?;
            return Ok(());
        }
        let cur_line = row == self.cur.0;
        // Background carried across the whole row when cursorline is on.
        let base_bg = if self.cursorline && cur_line {
            self.theme.cursorline_bg
        } else {
            Color::Reset
        };

        let g = self.gutter();
        if g > 0 {
            let fg = if cur_line {
                self.theme.gutter_current
            } else {
                self.theme.gutter
            };
            let num = format!("{:>w$} ", row + 1, w = g - 1);
            queue!(
                out,
                SetForegroundColor(fg),
                SetBackgroundColor(base_bg),
                Print(num),
                ResetColor
            )?;
        }

        let chars: Vec<char> = self.buf.lines[row].chars().collect();
        let styles = self.styles_for(row);
        let sel = self.selection();

        // One Cell per display column. A wide glyph is its own Cell followed by a
        // ch=None continuation, so horizontal scroll and clicks stay column-exact.
        let mut cells: Vec<Cell> = Vec::with_capacity(chars.len() + 4);
        let mut disp = 0;
        for (ci, &ch) in chars.iter().enumerate() {
            let sel = sel.is_some_and(|(a, b)| in_range(row, ci, a, b));
            let mat = self.match_hi.is_some_and(|(a, b)| in_range(row, ci, a, b));
            let cell = |ch, style| Cell {
                ch,
                extra: String::new(),
                style,
                sel,
                mat,
            };
            if ch == '\t' {
                let wdt = self.tab_size - (disp % self.tab_size);
                for _ in 0..wdt {
                    cells.push(cell(Some(' '), Style::Text));
                    disp += 1;
                }
            } else if is_control(ch) {
                // caret notation, e.g. ESC -> ^[  (never emit the raw byte)
                cells.push(cell(Some('^'), Style::Error));
                cells.push(cell(Some(caret_letter(ch)), Style::Error));
                disp += 2;
            } else if char_width(ch) == 0 {
                // combining mark: compose onto the previous glyph, no column of
                // its own — this keeps disp_col/click math in step with drawing.
                if let Some(last) = cells.iter_mut().rev().find(|c| c.ch.is_some()) {
                    last.extra.push(ch);
                } else {
                    cells.push(cell(Some(ch), Style::Text));
                    disp += 1;
                }
            } else {
                let style = styles.get(ci).copied().unwrap_or(Style::Text);
                let w = char_width(ch);
                cells.push(cell(Some(ch), style));
                for _ in 1..w {
                    cells.push(cell(None, style));
                }
                disp += w;
            }
        }

        let tw = self.text_width();
        let mut run = String::new();
        let mut cur_fg = Color::Reset;
        let mut cur_bg = Color::Reset;
        let mut cur_bold = false;
        let mut started = false;
        for k in 0..tw {
            let idx = self.left + k;
            let cell = cells.get(idx);
            // Continuation of a wide glyph already covered by its lead — draw
            // nothing, unless it's the left edge where the lead scrolled off.
            if let Some(c) = cell {
                if c.ch.is_none() && k != 0 {
                    continue;
                }
            }
            let (mut fg, bold) = match cell {
                Some(c) => self.theme.paint(c.style),
                None => (Color::Reset, false),
            };
            let mut bg = base_bg;
            let mut ch = ' ';
            if let Some(c) = cell {
                if c.mat {
                    bg = self.theme.match_bg;
                    fg = self.theme.match_fg;
                } else if c.sel {
                    bg = self.theme.selection_bg;
                }
                ch = c.ch.unwrap_or(' ');
                // A wide glyph that would spill past the last column: draw a
                // space so it never bleeds into the next line.
                if char_width(ch) == 2 && k + 1 >= tw {
                    ch = ' ';
                }
            }
            if !started || fg != cur_fg || bg != cur_bg || bold != cur_bold {
                if started {
                    flush_run(out, &run)?;
                    run.clear();
                }
                queue!(out, SetAttribute(Attribute::Reset))?;
                if bold {
                    queue!(out, SetAttribute(Attribute::Bold))?;
                }
                queue!(out, SetForegroundColor(fg), SetBackgroundColor(bg))?;
                cur_fg = fg;
                cur_bg = bg;
                cur_bold = bold;
                started = true;
            }
            run.push(ch);
            if let Some(c) = cell {
                run.push_str(&c.extra);
            }
        }
        flush_run(out, &run)?;
        queue!(out, SetAttribute(Attribute::Reset), ResetColor)
    }

    fn draw_message(&self, out: &mut Stdout) -> io::Result<()> {
        let y = self.h.saturating_sub(3);
        queue!(
            out,
            cursor::MoveTo(0, y as u16),
            Clear(ClearType::UntilNewLine)
        )?;
        if self.status.is_empty() {
            return Ok(());
        }
        let mut text = self.status.clone();
        truncate(&mut text, self.w);
        if self.error {
            queue!(
                out,
                SetForegroundColor(Color::Red),
                SetAttribute(Attribute::Bold),
                Print(text),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )
        } else {
            queue!(
                out,
                SetAttribute(Attribute::Bold),
                Print(text),
                SetAttribute(Attribute::Reset)
            )
        }
    }

    fn draw_shortcuts(&self, out: &mut Stdout) -> io::Result<()> {
        let row1 = [
            ("^G", "Help"),
            ("^O", "Save"),
            ("^W", "Search"),
            ("^\\", "Replace"),
            ("^K", "Cut"),
            ("^U", "Paste"),
            ("^Z", "Undo"),
            ("^X", "Exit"),
        ];
        let row2 = [
            ("^A", "Home"),
            ("^E", "End"),
            ("^C", "Copy"),
            ("^R", "InsFile"),
            ("M-G", "GoTo"),
            ("M-A", "About"),
            ("M-E", "Redo"),
            ("^L", "Refresh"),
        ];
        self.draw_shortcut_row(out, self.h.saturating_sub(2), &row1)?;
        self.draw_shortcut_row(out, self.h.saturating_sub(1), &row2)
    }

    fn draw_shortcut_row(
        &self,
        out: &mut Stdout,
        y: usize,
        keys: &[(&str, &str)],
    ) -> io::Result<()> {
        queue!(
            out,
            cursor::MoveTo(0, y as u16),
            Clear(ClearType::UntilNewLine)
        )?;
        let mut used = 0;
        let per = self.w / keys.len().max(1);
        for (k, label) in keys {
            let text = format!("{} {}", k, label);
            if used + text.chars().count() + 1 > self.w {
                break;
            }
            queue!(
                out,
                SetBackgroundColor(self.theme.key_bg),
                SetForegroundColor(self.theme.key_fg),
                SetAttribute(Attribute::Bold),
                Print(*k),
                SetAttribute(Attribute::Reset),
                ResetColor,
                Print(format!(" {}", label))
            )?;
            let cell = text.chars().count() + 1;
            let pad = per.saturating_sub(cell);
            queue!(out, Print(" ".repeat(pad)))?;
            used += per;
        }
        Ok(())
    }

    // --- selection --------------------------------------------------------

    fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cur {
            None
        } else if a <= self.cur {
            Some((a, self.cur))
        } else {
            Some((self.cur, a))
        }
    }

    fn move_to(&mut self, new: Pos, extend: bool) {
        if extend || self.mark {
            if self.anchor.is_none() {
                self.anchor = Some(self.cur);
            }
        } else {
            self.anchor = None;
        }
        self.cur = new;
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            let before = self.cur;
            self.buf.delete(a, b, before, a, Kind::Other);
            self.cur = a;
            self.goal = a.1;
            self.anchor = None;
            self.mark = false;
            true
        } else {
            false
        }
    }

    // --- key handling -----------------------------------------------------

    fn on_key(&mut self, k: KeyEvent) -> io::Result<()> {
        self.status.clear();
        self.error = false;
        self.match_hi = None;
        self.prev_cut = self.last_was_cut;
        self.last_was_cut = false;

        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);

        match k.code {
            KeyCode::Char(c) if ctrl => self.ctrl_key(c)?,
            KeyCode::Char(c) if alt => self.alt_key(c)?,
            KeyCode::Char(c) => {
                self.type_char(c);
            }
            KeyCode::Enter => self.newline(),
            KeyCode::Tab => self.tab(),
            KeyCode::BackTab => self.dedent(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_fwd(),
            KeyCode::Esc => {
                self.anchor = None;
                self.mark = false;
            }

            KeyCode::Left if ctrl || alt => self.mv(self.word_left(), shift),
            KeyCode::Right if ctrl || alt => self.mv(self.word_right(), shift),
            KeyCode::Left => self.mv(self.pos_left(), shift),
            KeyCode::Right => self.mv(self.pos_right(), shift),
            KeyCode::Up => self.mv_v(self.pos_up(), shift),
            KeyCode::Down => self.mv_v(self.pos_down(), shift),
            KeyCode::Home if ctrl => self.mv((0, 0), shift),
            KeyCode::End if ctrl => self.mv(self.doc_end(), shift),
            KeyCode::Home => self.mv(self.line_home(), shift),
            KeyCode::End => self.mv((self.cur.0, self.buf.line_len(self.cur.0)), shift),
            KeyCode::PageUp => self.mv_v(self.page(-1), shift),
            KeyCode::PageDown => self.mv_v(self.page(1), shift),
            KeyCode::F(1) => self.show_help()?,
            _ => {}
        }
        Ok(())
    }

    fn ctrl_key(&mut self, c: char) -> io::Result<()> {
        match c {
            'x' => self.try_quit()?,
            'o' => self.save(true)?,
            's' => self.save(false)?,
            'g' => self.show_help()?,
            'r' => self.insert_file()?,
            'w' => self.search()?,
            '\\' => self.replace()?,
            'k' => self.cut(),
            'u' => self.paste(),
            'c' => self.copy(),
            'z' => self.do_undo(),
            'y' => self.do_redo(),
            'd' => self.delete_fwd(),
            'a' => self.mv(self.line_home(), false),
            'e' => self.mv((self.cur.0, self.buf.line_len(self.cur.0)), false),
            'v' => self.mv_v(self.page(1), false),
            'l' => self.center(),
            '_' | '/' => self.goto()?,
            _ => {}
        }
        Ok(())
    }

    fn alt_key(&mut self, c: char) -> io::Result<()> {
        match c.to_ascii_lowercase() {
            'a' => self.show_about()?,
            'u' => self.do_undo(),
            'e' => self.do_redo(),
            'w' => self.find_next(),
            'g' => self.goto()?,
            'n' => {
                self.show_numbers = !self.show_numbers;
                self.status = format!("Line numbers {}", on_off(self.show_numbers));
            }
            'y' => {
                self.syntax = !self.syntax;
                self.states.clear();
                self.status = format!("Syntax highlight {}", on_off(self.syntax));
            }
            'm' => {
                self.mark = !self.mark;
                if self.mark {
                    self.anchor = Some(self.cur);
                    self.status = "Mark set".into();
                } else {
                    self.anchor = None;
                    self.status = "Mark unset".into();
                }
            }
            '6' => self.copy(),
            'v' => self.mv_v(self.page(-1), false),
            '\\' => self.mv((0, 0), false),
            '/' => self.mv(self.doc_end(), false),
            _ => {}
        }
        Ok(())
    }

    fn mv(&mut self, new: Pos, extend: bool) {
        self.buf.seal();
        self.move_to(new, extend);
        self.goal = self.cur.1;
    }

    // Vertical moves keep the "goal" column so a run of Up/Down tracks a ragged
    // right edge the way every editor you've used does.
    fn mv_v(&mut self, new: Pos, extend: bool) {
        self.buf.seal();
        self.move_to(new, extend);
    }

    // --- movement targets -------------------------------------------------

    fn pos_left(&self) -> Pos {
        let (r, c) = self.cur;
        if c > 0 {
            (r, c - 1)
        } else if r > 0 {
            (r - 1, self.buf.line_len(r - 1))
        } else {
            (r, c)
        }
    }
    fn pos_right(&self) -> Pos {
        let (r, c) = self.cur;
        if c < self.buf.line_len(r) {
            (r, c + 1)
        } else if r + 1 < self.buf.line_count() {
            (r + 1, 0)
        } else {
            (r, c)
        }
    }
    fn pos_up(&self) -> Pos {
        let (r, _) = self.cur;
        if r > 0 {
            (r - 1, self.goal.min(self.buf.line_len(r - 1)))
        } else {
            (0, 0)
        }
    }
    fn pos_down(&self) -> Pos {
        let (r, _) = self.cur;
        if r + 1 < self.buf.line_count() {
            (r + 1, self.goal.min(self.buf.line_len(r + 1)))
        } else {
            (r, self.buf.line_len(r))
        }
    }
    fn line_home(&self) -> Pos {
        // first press: first non-blank; already there: column 0
        let line = &self.buf.lines[self.cur.0];
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if self.cur.1 == indent {
            (self.cur.0, 0)
        } else {
            (self.cur.0, indent)
        }
    }
    fn doc_end(&self) -> Pos {
        let r = self.buf.line_count() - 1;
        (r, self.buf.line_len(r))
    }
    fn page(&self, dir: i64) -> Pos {
        let br = self.body_rows() as i64;
        let r = (self.cur.0 as i64 + dir * br).clamp(0, self.buf.line_count() as i64 - 1) as usize;
        (r, self.goal.min(self.buf.line_len(r)))
    }
    fn word_left(&self) -> Pos {
        let (r, mut c) = self.cur;
        if c == 0 {
            return if r > 0 {
                (r - 1, self.buf.line_len(r - 1))
            } else {
                (0, 0)
            };
        }
        let line: Vec<char> = self.buf.lines[r].chars().collect();
        while c > 0 && line[c - 1].is_whitespace() {
            c -= 1;
        }
        if c > 0 && is_word_char(line[c - 1]) {
            while c > 0 && is_word_char(line[c - 1]) {
                c -= 1;
            }
        } else {
            c = c.saturating_sub(1);
        }
        (r, c)
    }
    fn word_right(&self) -> Pos {
        let (r, mut c) = self.cur;
        let line: Vec<char> = self.buf.lines[r].chars().collect();
        let n = line.len();
        if c >= n {
            return if r + 1 < self.buf.line_count() {
                (r + 1, 0)
            } else {
                (r, c)
            };
        }
        if is_word_char(line[c]) {
            while c < n && is_word_char(line[c]) {
                c += 1;
            }
        } else {
            c += 1;
        }
        while c < n && line[c].is_whitespace() {
            c += 1;
        }
        (r, c)
    }

    fn center(&mut self) {
        let br = self.body_rows();
        self.top = self.cur.0.saturating_sub(br / 2);
        self.status = "Refreshed".into();
    }

    // --- editing ----------------------------------------------------------

    fn type_char(&mut self, c: char) {
        self.delete_selection();
        let before = self.cur;
        let s = c.to_string();
        self.cur = self.buf.insert(self.cur, &s, before, Kind::Type);
        self.goal = self.cur.1;
    }

    fn newline(&mut self) {
        self.delete_selection();
        let mut text = String::from("\n");
        if self.autoindent {
            let line = &self.buf.lines[self.cur.0];
            let head: String = line.chars().take(self.cur.1).collect();
            let indent: String = head.chars().take_while(|c| c.is_whitespace()).collect();
            text.push_str(&indent);
            if head.trim_end().ends_with('{') {
                text.push_str(&self.one_indent());
            }
        }
        let before = self.cur;
        self.cur = self.buf.insert(self.cur, &text, before, Kind::Other);
        self.goal = self.cur.1;
    }

    fn one_indent(&self) -> String {
        if self.use_spaces {
            " ".repeat(self.tab_size)
        } else {
            "\t".to_string()
        }
    }

    fn tab(&mut self) {
        if let Some((a, b)) = self.selection() {
            if a.0 != b.0 {
                self.indent_lines(a.0, b.0, true);
                return;
            }
        }
        let ind = if self.use_spaces {
            let d = self.disp_col(self.cur);
            " ".repeat(self.tab_size - (d % self.tab_size))
        } else {
            "\t".to_string()
        };
        let before = self.cur;
        self.cur = self.buf.insert(self.cur, &ind, before, Kind::Other);
        self.goal = self.cur.1;
    }

    fn dedent(&mut self) {
        if let Some((a, b)) = self.selection() {
            self.indent_lines(a.0, b.0, false);
        } else {
            self.indent_lines(self.cur.0, self.cur.0, false);
        }
    }

    fn indent_lines(&mut self, r1: usize, r2: usize, add: bool) {
        for r in r1..=r2 {
            // How many columns this row shifts by; cursor/anchor on this row move
            // with their text so a Tab on a selection keeps hold of it.
            let shift: isize = if add {
                let ind = self.one_indent();
                let w = ind.chars().count();
                let before = self.cur;
                self.buf.insert((r, 0), &ind, before, Kind::Other);
                w as isize
            } else {
                let line = &self.buf.lines[r];
                let mut n = 0;
                for (k, ch) in line.chars().enumerate() {
                    if k >= self.tab_size {
                        break;
                    }
                    if ch == '\t' {
                        n = 1;
                        break;
                    } else if ch == ' ' {
                        n += 1;
                    } else {
                        break;
                    }
                }
                if n > 0 {
                    self.buf
                        .delete((r, 0), (r, n), self.cur, self.cur, Kind::Other);
                }
                -(n as isize)
            };
            if self.cur.0 == r {
                self.cur.1 = shift_col(self.cur.1, shift);
            }
            if let Some(a) = self.anchor {
                if a.0 == r {
                    self.anchor = Some((a.0, shift_col(a.1, shift)));
                }
            }
        }
        self.cur.1 = self.cur.1.min(self.buf.line_len(self.cur.0));
        if let Some(a) = self.anchor {
            self.anchor = Some((a.0, self.buf.line_len(a.0).min(a.1)));
        }
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cur == (0, 0) {
            return;
        }
        let before = self.cur;
        let start = if self.cur.1 > 0 {
            (self.cur.0, self.cur.1 - 1)
        } else {
            (self.cur.0 - 1, self.buf.line_len(self.cur.0 - 1))
        };
        self.buf.delete(start, self.cur, before, start, Kind::Del);
        self.cur = start;
        self.goal = self.cur.1;
    }

    fn delete_fwd(&mut self) {
        if self.delete_selection() {
            return;
        }
        let (r, c) = self.cur;
        let end = if c < self.buf.line_len(r) {
            (r, c + 1)
        } else if r + 1 < self.buf.line_count() {
            (r + 1, 0)
        } else {
            return;
        };
        self.buf
            .delete(self.cur, end, self.cur, self.cur, Kind::Del);
    }

    // --- cut / copy / paste ----------------------------------------------

    fn cut(&mut self) {
        if let Some((a, b)) = self.selection() {
            let before = self.cur;
            self.cut = self.buf.delete(a, b, before, a, Kind::Other);
            self.cur = a;
            self.goal = a.1;
            self.anchor = None;
            self.mark = false;
            self.status = "Cut selection".into();
            return;
        }
        let row = self.cur.0;
        let before = self.cur;
        let removed = if row + 1 < self.buf.line_count() {
            self.buf
                .delete((row, 0), (row + 1, 0), before, (row, 0), Kind::Other)
        } else {
            let len = self.buf.line_len(row);
            let r = self
                .buf
                .delete((row, 0), (row, len), before, (row, 0), Kind::Other);
            format!("{}\n", r)
        };
        if self.prev_cut {
            self.cut.push_str(&removed);
        } else {
            self.cut = removed;
        }
        self.cur = (row.min(self.buf.line_count() - 1), 0);
        self.goal = 0;
        self.last_was_cut = true;
        self.status = "Cut line".into();
    }

    fn copy(&mut self) {
        if let Some((a, b)) = self.selection() {
            self.cut = self.range_text(a, b);
            self.status = "Copied selection".into();
        } else {
            self.cut = format!("{}\n", self.buf.lines[self.cur.0]);
            self.status = "Copied line".into();
        }
    }

    fn paste(&mut self) {
        if self.cut.is_empty() {
            self.status = "Cut buffer is empty".into();
            return;
        }
        self.delete_selection();
        let before = self.cur;
        let text = self.cut.clone();
        self.cur = self.buf.insert(self.cur, &text, before, Kind::Other);
        self.goal = self.cur.1;
        self.status = "Pasted".into();
    }

    fn paste_text(&mut self, s: &str) {
        self.delete_selection();
        let text = normalize_newlines(s);
        let before = self.cur;
        self.cur = self.buf.insert(self.cur, &text, before, Kind::Other);
        self.goal = self.cur.1;
    }

    // --- mouse ------------------------------------------------------------

    fn on_mouse(&mut self, m: crossterm::event::MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(p) = self.mouse_pos(m.column, m.row) {
                    self.buf.seal();
                    self.cur = p;
                    self.goal = p.1;
                    self.anchor = Some(p); // start of a possible drag
                    self.mark = false;
                    self.status.clear();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(p) = self.mouse_pos(m.column, m.row) {
                    self.cur = p;
                    self.goal = p.1;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // a plain click leaves anchor == cur; drop it so there's no
                // zero-width selection lingering around.
                if self.selection().is_none() {
                    self.anchor = None;
                }
            }
            MouseEventKind::ScrollDown => {
                let new = (self.cur.0 + 3).min(self.buf.line_count() - 1);
                self.mv_v((new, self.goal.min(self.buf.line_len(new))), false);
            }
            MouseEventKind::ScrollUp => {
                let new = self.cur.0.saturating_sub(3);
                self.mv_v((new, self.goal.min(self.buf.line_len(new))), false);
            }
            _ => {}
        }
    }

    // Screen (column, row) → buffer position, or None if the click missed the
    // text body (title, gutter, status/shortcut bars).
    fn mouse_pos(&self, col: u16, row: u16) -> Option<Pos> {
        let (col, row) = (col as usize, row as usize);
        let br = self.body_rows();
        if row < 1 || row > br {
            return None;
        }
        let line = (self.top + (row - 1)).min(self.buf.line_count() - 1);
        let g = self.gutter();
        if col < g {
            return Some((line, 0));
        }
        let target = self.left + (col - g);
        Some((line, self.char_at_disp(line, target)))
    }

    fn range_text(&self, a: Pos, b: Pos) -> String {
        if a.0 == b.0 {
            self.buf.lines[a.0]
                .chars()
                .skip(a.1)
                .take(b.1 - a.1)
                .collect()
        } else {
            let mut out = String::new();
            out.extend(self.buf.lines[a.0].chars().skip(a.1));
            out.push('\n');
            for r in a.0 + 1..b.0 {
                out.push_str(&self.buf.lines[r]);
                out.push('\n');
            }
            out.extend(self.buf.lines[b.0].chars().take(b.1));
            out
        }
    }

    fn do_undo(&mut self) {
        match self.buf.undo() {
            Some(p) => {
                self.cur = clamp_pos(&self.buf, p);
                self.goal = self.cur.1;
                self.anchor = None;
                self.status = "Undo".into();
            }
            None => self.status = "Nothing to undo".into(),
        }
    }
    fn do_redo(&mut self) {
        match self.buf.redo() {
            Some(p) => {
                self.cur = clamp_pos(&self.buf, p);
                self.goal = self.cur.1;
                self.anchor = None;
                self.status = "Redo".into();
            }
            None => self.status = "Nothing to redo".into(),
        }
    }

    // --- files ------------------------------------------------------------

    fn try_quit(&mut self) -> io::Result<()> {
        if !self.buf.dirty {
            self.quit = true;
            return Ok(());
        }
        match self.ask("Save modified buffer? (Y/N/Esc)")? {
            'y' => {
                self.save(false)?;
                if !self.buf.dirty {
                    self.quit = true;
                }
            }
            'n' => self.quit = true,
            _ => self.status = "Cancelled".into(),
        }
        Ok(())
    }

    fn save(&mut self, always_prompt: bool) -> io::Result<()> {
        let target = if self.path.is_none() || always_prompt {
            let cur = self
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            match self.prompt("File Name to Write", &cur)? {
                Some(s) if !s.trim().is_empty() => PathBuf::from(s),
                _ => {
                    self.status = "Cancelled".into();
                    return Ok(());
                }
            }
        } else {
            self.path.clone().unwrap()
        };

        match write_atomic(&target, &self.output_text(), self.backup) {
            Ok(()) => {
                let changed = self.path.as_deref() != Some(target.as_path());
                self.path = Some(target.clone());
                if changed {
                    let first = self.buf.lines.first().cloned().unwrap_or_default();
                    self.hl = highlight::detect(Some(&target), &first);
                    self.states.clear();
                }
                self.buf.dirty = false;
                self.status = format!("Wrote {} lines", self.buf.line_count());
            }
            Err(e) => {
                self.error = true;
                self.status = format!("Error writing file: {}", e);
            }
        }
        Ok(())
    }

    // The bytes actually written to disk. Applies the save-time cleanups without
    // touching the live buffer, so undo history stays exactly as the user left it.
    fn output_text(&self) -> String {
        let joiner = if self.buf.crlf { "\r\n" } else { "\n" };
        let mut lines: Vec<String> = self.buf.lines.clone();
        if self.trim_trailing {
            for l in &mut lines {
                let trimmed = l.trim_end();
                if trimmed.len() != l.len() {
                    *l = trimmed.to_string();
                }
            }
        }
        let mut out = lines.join(joiner);
        // Match Buffer::to_text: a file that is exactly "\n" keeps its newline.
        // Only a genuinely empty buffer (no final newline flag) stays empty.
        let want_final = self.final_newline || self.buf.final_newline;
        if want_final {
            out.push_str(joiner);
        }
        out
    }

    fn insert_file(&mut self) -> io::Result<()> {
        let name = match self.prompt("File to insert", "")? {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                self.status = "Cancelled".into();
                return Ok(());
            }
        };
        match std::fs::read(&name) {
            Ok(bytes) => {
                let text = normalize_newlines(&String::from_utf8_lossy(&bytes));
                let before = self.cur;
                self.cur = self.buf.insert(self.cur, &text, before, Kind::Other);
                self.goal = self.cur.1;
                self.status = format!("Inserted {}", name);
            }
            Err(e) => {
                self.error = true;
                self.status = format!("Error reading file: {}", e);
            }
        }
        Ok(())
    }

    // --- search / replace -------------------------------------------------

    fn search(&mut self) -> io::Result<()> {
        let init = self.last_query.clone();
        match self.prompt("Search", &init)? {
            Some(q) if !q.is_empty() => {
                self.last_query = q;
                self.jump_search(self.cur, true);
            }
            Some(_) if !self.last_query.is_empty() => self.find_next(),
            _ => self.status = "Cancelled".into(),
        }
        Ok(())
    }

    fn find_next(&mut self) {
        if self.last_query.is_empty() {
            self.status = "No previous search".into();
            return;
        }
        let start = self.pos_right();
        self.jump_search(start, true);
    }

    fn jump_search(&mut self, start: Pos, wrap: bool) {
        match self.find(start, wrap) {
            Some((ms, me)) => {
                self.cur = ms;
                self.goal = ms.1;
                self.match_hi = Some((ms, me));
                self.anchor = None;
                self.status = format!("Found \"{}\"", self.last_query);
            }
            None => self.status = format!("\"{}\" not found", self.last_query),
        }
    }

    fn find(&self, start: Pos, wrap: bool) -> Option<(Pos, Pos)> {
        let needle: Vec<char> = self.last_query.chars().collect();
        if needle.is_empty() {
            return None;
        }
        // vim-style: ignorecase off → always sensitive; on + smartcase →
        // sensitive only when the query itself contains an uppercase letter.
        let ci = if !self.ignorecase {
            false
        } else if self.smartcase {
            !self.last_query.chars().any(|c| c.is_uppercase())
        } else {
            true
        };
        let n = self.buf.line_count();
        let rounds = if wrap { n + 1 } else { n - start.0 };
        for k in 0..rounds {
            let row = if wrap { (start.0 + k) % n } else { start.0 + k };
            let line: Vec<char> = self.buf.lines[row].chars().collect();
            let begin = if k == 0 { start.1 } else { 0 };
            if begin > line.len() {
                continue;
            }
            let mut idx = begin;
            while idx + needle.len() <= line.len() {
                if matches_at(&line, idx, &needle, ci) {
                    return Some(((row, idx), (row, idx + needle.len())));
                }
                idx += 1;
            }
        }
        None
    }

    fn replace(&mut self) -> io::Result<()> {
        let init = self.last_query.clone();
        let query = match self.prompt("Search (to replace)", &init)? {
            Some(q) if !q.is_empty() => q,
            _ => {
                self.status = "Cancelled".into();
                return Ok(());
            }
        };
        self.last_query = query;
        let repl = match self.prompt("Replace with", "")? {
            Some(r) => r,
            None => {
                self.status = "Cancelled".into();
                return Ok(());
            }
        };

        let mut count = 0;
        let mut pos = self.cur;
        let mut all = false;
        while let Some(m) = self.find(pos, false) {
            let (ms, me) = m;
            self.cur = ms;
            self.match_hi = Some((ms, me));
            if !all {
                self.scroll();
                let mut out = io::stdout();
                self.draw(&mut out)?;
                match self.ask("Replace? (Y/N/A=all/Esc)")? {
                    'y' => {}
                    'a' => all = true,
                    'n' => {
                        pos = me;
                        continue;
                    }
                    _ => break,
                }
            }
            self.buf.delete(ms, me, ms, ms, Kind::Other);
            let end = self.buf.insert(ms, &repl, ms, Kind::Other);
            count += 1;
            pos = end;
            self.cur = end;
        }
        self.match_hi = None;
        self.status = format!("Replaced {} occurrence(s)", count);
        Ok(())
    }

    fn goto(&mut self) -> io::Result<()> {
        match self.prompt("Go to line[,column]", "")? {
            Some(s) if !s.trim().is_empty() => {
                let mut it = s.split([',', ':']);
                let line: usize = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(1);
                let col: usize = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(1);
                let row = line.saturating_sub(1).min(self.buf.line_count() - 1);
                let c = col.saturating_sub(1).min(self.buf.line_len(row));
                self.cur = (row, c);
                self.goal = c;
                self.anchor = None;
            }
            _ => self.status = "Cancelled".into(),
        }
        Ok(())
    }

    // --- prompts ----------------------------------------------------------

    fn read_key(&mut self) -> io::Result<KeyEvent> {
        loop {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => return Ok(k),
                Event::Resize(w, h) => {
                    self.w = (w as usize).max(20);
                    self.h = (h as usize).max(5);
                }
                _ => {}
            }
        }
    }

    fn prompt(&mut self, label: &str, init: &str) -> io::Result<Option<String>> {
        let mut input: Vec<char> = init.chars().collect();
        let mut pos = input.len();
        let mut out = io::stdout();
        loop {
            self.draw(&mut out)?;
            self.draw_prompt(&mut out, label, &input, pos)?;
            out.flush()?;
            let k = self.read_key()?;
            match k.code {
                KeyCode::Enter => return Ok(Some(input.into_iter().collect())),
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None)
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                    pos = 0;
                }
                KeyCode::Char(c) => {
                    input.insert(pos, c);
                    pos += 1;
                }
                KeyCode::Backspace => {
                    if pos > 0 {
                        input.remove(pos - 1);
                        pos -= 1;
                    }
                }
                KeyCode::Delete => {
                    if pos < input.len() {
                        input.remove(pos);
                    }
                }
                KeyCode::Left => pos = pos.saturating_sub(1),
                KeyCode::Right => pos = (pos + 1).min(input.len()),
                KeyCode::Home => pos = 0,
                KeyCode::End => pos = input.len(),
                _ => {}
            }
        }
    }

    fn draw_prompt(
        &self,
        out: &mut Stdout,
        label: &str,
        input: &[char],
        pos: usize,
    ) -> io::Result<()> {
        let y = self.h.saturating_sub(3);
        let text: String = input.iter().collect();
        let prefix = format!("{}: ", label);
        queue!(
            out,
            cursor::MoveTo(0, y as u16),
            Clear(ClearType::UntilNewLine),
            SetForegroundColor(self.theme.key_fg),
            SetBackgroundColor(self.theme.key_bg),
            SetAttribute(Attribute::Bold),
            Print(prefix.clone()),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Print(text)
        )?;
        let cx = prefix.chars().count() + pos;
        queue!(out, cursor::MoveTo(cx as u16, y as u16), cursor::Show)
    }

    fn ask(&mut self, question: &str) -> io::Result<char> {
        let mut out = io::stdout();
        let y = self.h.saturating_sub(3);
        queue!(
            out,
            cursor::MoveTo(0, y as u16),
            Clear(ClearType::UntilNewLine),
            SetForegroundColor(self.theme.key_fg),
            SetBackgroundColor(self.theme.key_bg),
            SetAttribute(Attribute::Bold),
            Print(format!(" {} ", question)),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        out.flush()?;
        loop {
            let k = self.read_key()?;
            match k.code {
                KeyCode::Char(c) => return Ok(c.to_ascii_lowercase()),
                KeyCode::Esc => return Ok('\x1b'),
                KeyCode::Enter => return Ok('y'),
                _ => {}
            }
        }
    }

    // --- overlays ---------------------------------------------------------

    fn show_help(&mut self) -> io::Result<()> {
        self.overlay("chie — keybindings", &help_text())
    }

    fn show_about(&mut self) -> io::Result<()> {
        self.overlay("about chie", &about_text())
    }

    fn overlay(&mut self, title: &str, lines: &[String]) -> io::Result<()> {
        let mut out = io::stdout();
        let mut top = 0usize;
        loop {
            let view = self.h.saturating_sub(2);
            queue!(out, cursor::Hide, ResetColor, cursor::MoveTo(0, 0))?;
            let mut bar = vec![' '; self.w];
            put(&mut bar, 0, &format!(" {} ", title));
            put(&mut bar, self.w.saturating_sub(20), "(Esc / q to close) ");
            let bs: String = bar.into_iter().collect();
            queue!(
                out,
                SetBackgroundColor(self.theme.bar_bg),
                SetForegroundColor(self.theme.bar_fg),
                Print(bs),
                ResetColor
            )?;
            for i in 0..view {
                queue!(
                    out,
                    cursor::MoveTo(0, (i + 1) as u16),
                    Clear(ClearType::UntilNewLine)
                )?;
                if let Some(line) = lines.get(top + i) {
                    let mut l = line.clone();
                    truncate(&mut l, self.w);
                    queue!(out, Print(l))?;
                }
            }
            queue!(
                out,
                cursor::MoveTo(0, (self.h - 1) as u16),
                Clear(ClearType::UntilNewLine)
            )?;
            out.flush()?;
            let k = self.read_key()?;
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => break,
                KeyCode::Down | KeyCode::Char('j') => {
                    if top + view < lines.len() {
                        top += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => top = top.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    top = (top + view).min(lines.len().saturating_sub(1))
                }
                KeyCode::PageUp => top = top.saturating_sub(view),
                _ => {}
            }
        }
        Ok(())
    }
}

// --- free helpers ---------------------------------------------------------

fn flush_run(out: &mut Stdout, run: &str) -> io::Result<()> {
    if run.is_empty() {
        Ok(())
    } else {
        queue!(out, Print(run))
    }
}

// Write via a sibling temp file and rename over the target. If anything fails
// mid-write, the original is still whole — the rename is the only step that
// touches it, and rename is atomic on every platform we run on. Permissions of
// an existing file are carried over; with `backup`, a `file~` copy is kept.
pub(crate) fn write_atomic(path: &Path, data: &str, backup: bool) -> io::Result<()> {
    use std::fs;

    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("chie");
    let tmp_name = format!(".{}.chie-{}.tmp", name, std::process::id());
    let tmp = match dir {
        Some(d) => d.join(tmp_name),
        None => PathBuf::from(tmp_name),
    };

    if backup && path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push("~");
        let _ = fs::copy(path, PathBuf::from(bak));
    }

    // Best-effort: clean up the temp file if a later step fails.
    let result = (|| {
        fs::write(&tmp, data)?;
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// Strip carriage returns so pasted or inserted CRLF/CR text doesn't leave stray
// `\r` bytes inside lines (they'd render as raw control chars and save verbatim).
pub(crate) fn shift_col(col: usize, delta: isize) -> usize {
    if delta >= 0 {
        col + delta as usize
    } else {
        col.saturating_sub((-delta) as usize)
    }
}

pub(crate) fn normalize_newlines(s: &str) -> String {
    if s.contains('\r') {
        s.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        s.to_string()
    }
}

fn in_range(row: usize, ci: usize, a: Pos, b: Pos) -> bool {
    let p = (row, ci);
    a <= p && p < b
}

fn matches_at(line: &[char], idx: usize, needle: &[char], ci: bool) -> bool {
    if idx + needle.len() > line.len() {
        return false;
    }
    for j in 0..needle.len() {
        let (a, b) = (line[idx + j], needle[j]);
        let eq = if ci {
            a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        };
        if !eq {
            return false;
        }
    }
    true
}

fn clamp_pos(buf: &Buffer, p: Pos) -> Pos {
    let row = p.0.min(buf.line_count() - 1);
    (row, p.1.min(buf.line_len(row)))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn digits(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn put(bar: &mut [char], at: usize, s: &str) {
    for (i, c) in s.chars().enumerate() {
        if at + i < bar.len() {
            bar[at + i] = c;
        }
    }
}

fn truncate(s: &mut String, w: usize) {
    if s.chars().count() > w {
        *s = s.chars().take(w).collect();
    }
}

fn help_text() -> Vec<String> {
    let raw = "
  Movement
    Arrows            move by character / line
    Ctrl/Alt + <->    jump one word left or right
    Home / Ctrl+A     start of line (smart: indent, then column 0)
    End  / Ctrl+E     end of line
    PageUp / PageDown scroll a screen
    Ctrl+Home / End   top / bottom of the file
    Alt+\\ / Alt+/     first / last line
    Shift + move      extend the selection

  Editing
    Tab / Shift+Tab   indent / unindent (works on a whole selection)
    Ctrl+K            cut selection, or the whole line
    Alt+6 / Ctrl+C    copy selection or line
    Ctrl+U            paste
    Ctrl+D / Delete   delete the character ahead
    Ctrl+Z / Alt+U    undo
    Alt+E             redo
    Alt+M             set / drop the selection mark

  Files
    Ctrl+O            write out (asks for a name)
    Ctrl+S            save
    Ctrl+R            insert another file at the cursor
    Ctrl+X            exit (offers to save)

  Search
    Ctrl+W            find
    Alt+W             find next
    Ctrl+\\            search and replace
    Alt+G / Ctrl+_    go to line[,column]

  Mouse
    Click             move the cursor there
    Drag              select text
    Wheel             scroll

  View
    Alt+N             toggle line numbers
    Alt+Y             toggle syntax highlighting
    Ctrl+L            recenter / refresh

  Config
    ~/.config/chie/config sets behaviour and your own colors.
    Run 'chie --config' to see the exact path.

  Help
    Ctrl+G / F1       this screen
    Alt+A             about chie
";
    raw.lines().map(|l| l.to_string()).collect()
}

fn about_text() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    v.push(String::new());
    for l in about::LOGO {
        v.push(format!("    {}", l));
    }
    v.push(String::new());
    v.push(format!(
        "    chie {}  —  {}",
        about::VERSION,
        about::TAGLINE
    ));
    v.push(String::new());
    v.push("    A terminal text editor built to out-nano nano, with".into());
    v.push("    first-class highlighting for UniLand, Python and more.".into());
    v.push(String::new());
    v.push(format!("    Author:    {}", about::AUTHOR));
    v.push(format!("    Telegram:  {}", about::TELEGRAM));
    v.push(String::new());
    v.push("    Source (mirrors):".into());
    for r in about::REPOS {
        v.push(format!("      • {}", r));
    }
    v.push(String::new());
    v.push("    License:   MIT".into());
    v.push(String::new());
    v
}
