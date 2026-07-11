// Terminal cell width of a character. Most are one column, CJK and emoji take
// two, combining marks take none. This is the usual wcwidth approximation — not
// the whole Unicode database, but enough that the cursor never drifts on real
// text. Tabs are handled by the caller, not here.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u == 0 {
        return 0;
    }
    // Control bytes are shown as caret notation (^[, ^?, …) — two columns.
    if is_control(c) {
        return 2;
    }
    if is_zero_width(u) {
        return 0;
    }
    if is_wide(u) {
        2
    } else {
        1
    }
}

// C0 controls (except tab, laid out separately) and DEL. These are never sent
// to the terminal raw — that would let a file smuggle escape sequences — so the
// renderer prints them as ^X and the width math agrees they take two columns.
pub fn is_control(c: char) -> bool {
    let u = c as u32;
    (u < 0x20 && c != '\t') || u == 0x7f
}

// The letter shown after '^' for a control char: ^@ ^A … ^Z ^[ … ^_ and ^?.
pub fn caret_letter(c: char) -> char {
    let u = c as u32;
    if u == 0x7f {
        '?'
    } else {
        char::from_u32(0x40 + u).unwrap_or('?')
    }
}

fn is_zero_width(u: u32) -> bool {
    matches!(u,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A |
        0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 |
        0x0E31 | 0x0E34..=0x0E3A | 0x0EB1 | 0x0EB4..=0x0EB9 |
        0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F |
        0x200B..=0x200F | 0xFEFF)
}

fn is_wide(u: u32) -> bool {
    matches!(u,
        0x1100..=0x115F | 0x2329..=0x232A | 0x2E80..=0x303E | 0x3041..=0x33FF |
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 |
        0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 |
        0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF |
        0x20000..=0x2FFFD | 0x30000..=0x3FFFD)
}
