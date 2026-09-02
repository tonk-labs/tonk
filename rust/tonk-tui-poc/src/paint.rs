//! Paint a laid-out tree into a `ratatui` cell buffer, then serialise
//! the buffer to a string.
//!
//! This is the whole of ratatui's role: `Buffer`, `Rect`, `Style`,
//! `Color`, `Modifier`. No `ratatui-widgets` — the vocabulary owns its
//! own boxes, text and chips (`plan/tui-views.md` §6.3, §10).

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect as TermRect;
use ratatui::style::{Color, Modifier, Style as TermStyle};
use tonk_layout::{Kind, Laid, Rect};

use crate::theme::Theme;

/// Paint `laid` into a `viewport`-sized buffer and render it to text.
pub fn frame(laid: &Laid, viewport: Rect, theme: &Theme, explain: bool, plain: bool) -> String {
    let area = TermRect::new(viewport.x, viewport.y, viewport.width, viewport.height);
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, laid, theme, TermStyle::default(), area);
    if explain {
        outline(&mut buffer, laid, 0);
    }
    serialise(&buffer, plain)
}

/// `clip` is the region this element and its descendants may paint in:
/// the viewport at the root, narrowed by every clipping ancestor. It is
/// how a `<box>` keeps its contents inside its own border when they
/// overflow — which they can, because nothing shrinks below its
/// content. Turning that overflow into a scrollable region instead is
/// `<scroll>`, which this proof of concept does not have.
fn draw(buffer: &mut Buffer, laid: &Laid, theme: &Theme, inherited: TermStyle, clip: TermRect) {
    let style = resolve(laid, theme, inherited);
    let rect = term_rect(laid.rect);
    let viewport = clip;
    let visible = rect.intersection(viewport);

    if !visible.is_empty() {
        // A background or a reverse fills the element's whole box,
        // which is how a chip reads as a chip.
        if laid.style.bg.is_some() || laid.style.emphasis.reverse {
            fill(buffer, visible, style);
        }
        if laid.style.border {
            draw_border(buffer, rect, visible, style);
        }
    }

    match &laid.kind {
        Kind::Text(_) | Kind::Paragraph(_) => {
            let inner = inset(rect, &laid.style).intersection(viewport);
            for (row, line) in laid.lines.iter().enumerate() {
                let Some(y) = inner.y.checked_add(row as u16) else {
                    break;
                };
                if y >= inner.bottom() {
                    break;
                }
                buffer.set_stringn(inner.x, y, line, inner.width as usize, style);
            }
        }
        _ => {
            let child_clip = if laid.style.clip {
                inset(rect, &laid.style).intersection(clip)
            } else {
                clip
            };
            for child in &laid.children {
                draw(buffer, child, theme, style, child_clip);
            }
        }
    }
}

/// Style inherits down the tree — a `fg` on a row applies to the text
/// inside it — which is the one piece of cascade a terminal needs and
/// the most a system without selectors should have.
fn resolve(laid: &Laid, theme: &Theme, inherited: TermStyle) -> TermStyle {
    let mut style = inherited;
    if let Some(fg) = laid
        .style
        .fg
        .as_deref()
        .and_then(|name| theme.resolve(name))
    {
        style = style.fg(fg);
    }
    if let Some(bg) = laid
        .style
        .bg
        .as_deref()
        .and_then(|name| theme.resolve(name))
    {
        style = style.bg(bg);
    }
    let emphasis = laid.style.emphasis;
    if emphasis.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if emphasis.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    if emphasis.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if emphasis.reverse {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

fn fill(buffer: &mut Buffer, rect: TermRect, style: TermStyle) {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_style(style);
            }
        }
    }
}

/// Hard corners, per the design language: always `┌┐`, never `╭╮`.
///
/// `rect` is where the border *would* be and decides which cells are
/// corners; `visible` is the part of it inside the viewport. Splitting
/// the two is what lets a partly-clipped box still draw the edges it
/// does have without inventing corners at the clip line.
fn draw_border(buffer: &mut Buffer, rect: TermRect, visible: TermRect, style: TermStyle) {
    if rect.width < 2 || rect.height < 2 || visible.is_empty() {
        return;
    }
    let (left, right) = (rect.x, rect.right() - 1);
    let (top, bottom) = (rect.y, rect.bottom() - 1);
    for x in visible.x..visible.right() {
        put(buffer, x, top, '─', style);
        put(buffer, x, bottom, '─', style);
    }
    for y in visible.y..visible.bottom() {
        put(buffer, left, y, '│', style);
        put(buffer, right, y, '│', style);
    }
    put(buffer, left, top, '┌', style);
    put(buffer, right, top, '┐', style);
    put(buffer, left, bottom, '└', style);
    put(buffer, right, bottom, '┘', style);
}

/// elm-ui's `explain`: outline every element so a template author can
/// see the boxes they cannot inspect with devtools.
fn outline(buffer: &mut Buffer, laid: &Laid, depth: usize) {
    const MARKS: [char; 4] = ['·', '+', '*', '#'];
    let rect = term_rect(laid.rect);
    let style = TermStyle::default().fg(Color::DarkGray);
    let mark = MARKS[depth % MARKS.len()];
    if rect.width >= 2 && rect.height >= 1 {
        put(buffer, rect.x, rect.y, mark, style);
        put(buffer, rect.right() - 1, rect.y, mark, style);
        if rect.height >= 2 {
            put(buffer, rect.x, rect.bottom() - 1, mark, style);
            put(buffer, rect.right() - 1, rect.bottom() - 1, mark, style);
        }
    }
    for child in &laid.children {
        outline(buffer, child, depth + 1);
    }
}

fn put(buffer: &mut Buffer, x: u16, y: u16, symbol: char, style: TermStyle) {
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_char(symbol);
        cell.set_style(style);
    }
}

/// The content box: the element's rect less its border and padding.
fn inset(rect: TermRect, style: &tonk_layout::Style) -> TermRect {
    let border = u16::from(style.border);
    let left = style.pad.left + border;
    let top = style.pad.top + border;
    let horizontal = left + style.pad.right + border;
    let vertical = top + style.pad.bottom + border;
    TermRect::new(
        rect.x.saturating_add(left),
        rect.y.saturating_add(top),
        rect.width.saturating_sub(horizontal),
        rect.height.saturating_sub(vertical),
    )
}

fn term_rect(rect: Rect) -> TermRect {
    TermRect::new(rect.x, rect.y, rect.width, rect.height)
}

/// Serialise the buffer. `plain` drops every escape so a snapshot test
/// diffs on glyphs alone; otherwise real SGR is emitted.
fn serialise(buffer: &Buffer, plain: bool) -> String {
    let area = buffer.area();
    let mut out = String::new();
    for y in area.y..area.bottom() {
        let mut line = String::new();
        let mut active = TermStyle::default();
        let mut x = area.x;
        while x < area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                x += 1;
                continue;
            };
            if !plain {
                let style = cell.style();
                if style != active {
                    line.push_str(&sgr(style));
                    active = style;
                }
            }
            line.push_str(cell.symbol());
            // A wide grapheme occupies one cell and blanks the cells it
            // covers, so those must be skipped rather than emitted as
            // spaces — the same step a real backend takes. Emitting
            // them instead pushes the rest of the line right, which
            // reads as a *layout* bug and is the trap `measure.rs`
            // exists to avoid on the other side of the pipeline.
            x += tonk_layout::text_width(cell.symbol()).max(1);
        }
        if !plain && active != TermStyle::default() {
            line.push_str("\x1b[0m");
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }
    out
}

fn sgr(style: TermStyle) -> String {
    let mut codes = vec!["0".to_string()];
    let modifiers = style.add_modifier;
    if modifiers.contains(Modifier::BOLD) {
        codes.push("1".into());
    }
    if modifiers.contains(Modifier::DIM) {
        codes.push("2".into());
    }
    if modifiers.contains(Modifier::UNDERLINED) {
        codes.push("4".into());
    }
    if modifiers.contains(Modifier::REVERSED) {
        codes.push("7".into());
    }
    if let Some(code) = colour_code(style.fg, 3) {
        codes.push(code);
    }
    if let Some(code) = colour_code(style.bg, 4) {
        codes.push(code);
    }
    format!("\x1b[{}m", codes.join(";"))
}

/// `base` is 3 for foreground, 4 for background.
fn colour_code(colour: Option<Color>, base: u8) -> Option<String> {
    let colour = colour?;
    Some(match colour {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => format!("{base}8;2;{r};{g};{b}"),
        Color::Indexed(index) => format!("{base}8;5;{index}"),
        Color::Black => format!("{}0", base),
        Color::Red => format!("{}1", base),
        Color::Green => format!("{}2", base),
        Color::Yellow => format!("{}3", base),
        Color::Blue => format!("{}4", base),
        Color::Magenta => format!("{}5", base),
        Color::Cyan => format!("{}6", base),
        Color::Gray => format!("{}7", base),
        Color::DarkGray => format!("{}8;5;8", base),
        _ => format!("{}9", base),
    })
}
