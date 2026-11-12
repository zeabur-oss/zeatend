//! Writes a ratatui buffer as SVG, one positioned glyph per cell. A wide character takes two
//! cells and the cell after it is not drawn. Colours are CSS classes, so one file follows the
//! reader's light or dark setting.

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier};

use crate::text::width;

const CELL_W: f32 = 9.0;
const LINE_H: f32 = 17.5;
const PAD: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Theme {
    /// Light, switching to dark with `prefers-color-scheme`.
    Auto,
    Light,
    Dark,
}

/// Name, light value, dark value. `fg` and `bg` are the terminal defaults.
const PALETTE: [(&str, &str, &str); 14] = [
    ("fg", "#1f2328", "#e6edf3"),
    ("bg", "#ffffff", "#0d1117"),
    ("black", "#24292f", "#484f58"),
    ("red", "#cf222e", "#ff7b72"),
    ("green", "#116329", "#3fb950"),
    ("yellow", "#9a6700", "#d29922"),
    ("blue", "#0969da", "#58a6ff"),
    ("magenta", "#8250df", "#bc8cff"),
    ("cyan", "#1b7c83", "#39c5cf"),
    ("gray", "#6e7781", "#8b949e"),
    ("darkgray", "#57606a", "#6e7681"),
    ("white", "#6e7781", "#b1bac4"),
    ("lightcyan", "#1b7c83", "#56d4dd"),
    ("lightgreen", "#116329", "#56d364"),
];

const fn color_name(color: Color, default: &'static str) -> &'static str {
    match color {
        Color::Black => "black",
        Color::Red | Color::LightRed => "red",
        Color::Green => "green",
        Color::LightGreen => "lightgreen",
        Color::Yellow | Color::LightYellow => "yellow",
        Color::Blue | Color::LightBlue => "blue",
        Color::Magenta | Color::LightMagenta => "magenta",
        Color::Cyan => "cyan",
        Color::LightCyan => "lightcyan",
        Color::Gray => "gray",
        Color::DarkGray => "darkgray",
        Color::White => "white",
        _ => default,
    }
}

fn rules(out: &mut String, dark: bool) {
    for (name, light, dark_value) in PALETTE {
        let value = if dark { dark_value } else { light };
        let _ = write!(out, ".f-{name}{{fill:{value}}}.b-{name}{{fill:{value}}}");
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(PartialEq)]
struct Style {
    fg: &'static str,
    bg: &'static str,
    bold: bool,
    dim: bool,
}

const fn cell_style(fg: Color, bg: Color, modifier: Modifier) -> Style {
    let mut fg = color_name(fg, "fg");
    let mut bg = color_name(bg, "bg");
    if modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    Style {
        fg,
        bg,
        bold: modifier.contains(Modifier::BOLD),
        dim: modifier.contains(Modifier::DIM),
    }
}

pub(crate) fn render(buf: &Buffer, cursor: Option<Position>, theme: Theme, lang: &str) -> String {
    let cols = buf.area.width;
    let rows = buf.area.height;
    let w = 2.0_f32.mul_add(PAD, f32::from(cols) * CELL_W);
    let h = 2.0_f32.mul_add(PAD, f32::from(rows) * LINE_H);

    let mut css = String::from(
        "svg{font:15px ui-monospace,SFMono-Regular,Menlo,'PingFang TC','PingFang SC','Hiragino Sans','Noto Sans Mono CJK TC',monospace}.bold{font-weight:700}.dim{opacity:.6}",
    );
    match theme {
        Theme::Light => rules(&mut css, false),
        Theme::Dark => rules(&mut css, true),
        Theme::Auto => {
            rules(&mut css, false);
            css.push_str("@media (prefers-color-scheme:dark){");
            rules(&mut css, true);
            css.push('}');
        }
    }

    let mut out = String::new();
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xml:lang=\"{lang}\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><style>{css}</style><rect class=\"b-bg\" width=\"100%\" height=\"100%\"/>"
    );

    for y in 0..rows {
        let top = f32::mul_add(f32::from(y), LINE_H, PAD);
        let baseline = LINE_H.mul_add(0.75, top);
        let mut x = 0;
        while x < cols {
            let first = &buf[(x, y)];
            let style = cell_style(first.fg, first.bg, first.modifier);
            let start = x;
            let mut glyphs = String::new();
            while x < cols {
                let cell = &buf[(x, y)];
                if cell_style(cell.fg, cell.bg, cell.modifier) != style {
                    break;
                }
                let symbol = cell.symbol();
                let cells = u16::try_from(width(symbol).max(1)).unwrap_or(1);
                if symbol != " " && !symbol.is_empty() {
                    // One element per cell: not every renderer supports a list of x positions.
                    let _ = write!(
                        glyphs,
                        "<text x=\"{:.1}\" y=\"{baseline:.1}\">{}</text>",
                        f32::mul_add(f32::from(x), CELL_W, PAD),
                        escape(symbol)
                    );
                }
                x += cells;
            }
            if style.bg != "bg" {
                let _ = write!(
                    out,
                    "<rect class=\"b-{}\" x=\"{:.1}\" y=\"{top:.1}\" width=\"{:.1}\" height=\"{LINE_H}\"/>",
                    style.bg,
                    f32::mul_add(f32::from(start), CELL_W, PAD),
                    f32::from(x - start) * CELL_W
                );
            }
            if !glyphs.is_empty() {
                let mut class = format!("f-{}", style.fg);
                if style.bold {
                    class.push_str(" bold");
                }
                if style.dim {
                    class.push_str(" dim");
                }
                let _ = write!(out, "<g class=\"{class}\">{glyphs}</g>");
            }
        }
    }

    if let Some(at) = cursor {
        let _ = write!(
            out,
            "<rect class=\"b-fg\" x=\"{:.1}\" y=\"{:.1}\" width=\"2\" height=\"{LINE_H}\"/>",
            f32::mul_add(f32::from(at.x), CELL_W, PAD),
            f32::mul_add(f32::from(at.y), LINE_H, PAD)
        );
    }
    out.push_str("</svg>\n");
    out
}
