//! Text measured in terminal cells. A CJK character takes two cells.

use unicode_linebreak::BreakOpportunity;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

/// The cells `text` takes.
pub(crate) fn width(text: &str) -> usize {
    text.width()
}

/// `text` cut to at most `max` cells, ending with `...` when cut. A wide character that would
/// cross the edge is left out, not split.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let budget = max.saturating_sub(3);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
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

/// `text` with spaces after it up to `cells` cells.
pub(crate) fn pad(text: &str, cells: usize) -> String {
    format!("{text}{}", " ".repeat(cells.saturating_sub(text.width())))
}

/// `text` in lines of at most `cells` cells, broken where Unicode line breaking (UAX #14)
/// allows: between words, and between most CJK characters but not before `。` or `、`. A word
/// longer than a line is split.
pub(crate) fn wrap(text: &str, cells: usize) -> Vec<String> {
    let cells = cells.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut start = 0;
    for (end, opportunity) in unicode_linebreak::linebreaks(text) {
        let segment = text.get(start..end).unwrap_or_default();
        start = end;
        let mandatory = opportunity == BreakOpportunity::Mandatory;
        let segment = if mandatory {
            segment.trim_end_matches(['\n', '\r'])
        } else {
            segment
        };
        if !line.is_empty() && line.width() + segment.trim_end().width() > cells {
            lines.push(line.trim_end().to_owned());
            line.clear();
        }
        for c in segment.chars() {
            if !line.is_empty()
                && !c.is_whitespace()
                && line.width() + c.width().unwrap_or(0) > cells
            {
                lines.push(line.trim_end().to_owned());
                line.clear();
            }
            line.push(c);
        }
        if mandatory {
            lines.push(line.trim_end().to_owned());
            line.clear();
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line.trim_end().to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_cells_not_characters() {
        assert_eq!(truncate("王小明", 6), "王小明");
        assert_eq!(truncate("我的第一台伺服器", 9), "我的第...");
        assert_eq!(truncate("一二三四", 7), "一二...");
        assert_eq!(width(&pad("記憶體", 8)), 8);
    }

    #[test]
    fn lines_break_between_words_and_keep_cjk_punctuation_on_its_line() {
        assert_eq!(
            wrap("Fix what failed, then run it again.", 16),
            ["Fix what failed,", "then run it", "again."]
        );
        let text = "這台伺服器只有內網位址。你的電腦用哪個位址連到它？";
        for cells in 4..30 {
            let lines = wrap(text, cells);
            assert_eq!(lines.concat(), text);
            for line in &lines {
                assert!(width(line) <= cells, "{cells}: {line}");
                assert!(
                    !line.starts_with(['。', '、', '，', '？', '：']),
                    "{cells}: {lines:?}"
                );
            }
        }
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
    }

    /// Glyphs the screens draw inside text. An East Asian Width "A" (ambiguous) glyph takes two
    /// cells in a terminal set to treat them as wide, which moves everything after it.
    #[test]
    fn the_screens_draw_no_ambiguous_width_glyph_in_text() {
        for glyph in ['✓', '!', '>', '(', '*', 'x', '-', '+', '|'] {
            assert_eq!(glyph.width(), glyph.width_cjk(), "{glyph} is ambiguous");
        }
        // These are ambiguous, so the screens leave them out of text.
        for glyph in ['●', '…', '·', '→', '█'] {
            assert_ne!(glyph.width(), glyph.width_cjk(), "{glyph} is not ambiguous");
        }
    }
}
