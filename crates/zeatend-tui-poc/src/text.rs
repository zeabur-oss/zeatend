//! Display-width helpers. A CJK character takes two terminal cells.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn width(s: &str) -> usize {
    s.width()
}

/// Cuts `s` to at most `max` cells and ends it with `...` when cut.
/// A wide character that would straddle the edge is dropped, not split.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    let budget = max.saturating_sub(3);
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str("...");
    out
}

/// Pads `s` with spaces to `cells` display cells.
pub(crate) fn pad(s: &str, cells: usize) -> String {
    let mut out = s.to_owned();
    for _ in s.width()..cells {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_cells_not_chars() {
        assert_eq!(truncate("王小明", 6), "王小明");
        assert_eq!(truncate("我的第一台伺服器", 9), "我的第...");
        // 4 cells left for text after "...": two wide chars fit, a third would straddle.
        assert_eq!(truncate("一二三四", 7), "一二...");
        assert_eq!(width(&pad("記憶體", 8)), 8);
    }

    /// Glyphs the screens draw inside text. East Asian Width "A" (ambiguous) glyphs take two
    /// cells in terminals set to treat them as wide, which shifts everything after them.
    #[test]
    fn content_glyphs_are_not_ambiguous_width() {
        for glyph in ['✓', '!', '>', '(', '*', '[', '='] {
            assert_eq!(glyph.width(), glyph.width_cjk(), "{glyph} is ambiguous");
        }
        // Box drawing, the bullet and the ellipsis are ambiguous; they are listed so the README
        // claim stays true.
        for glyph in ['─', '│', '┌', '╭', '●', '…', '·', '→'] {
            assert_ne!(glyph.width(), glyph.width_cjk(), "{glyph} is not ambiguous");
        }
    }
}
