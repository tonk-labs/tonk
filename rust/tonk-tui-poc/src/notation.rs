//! Notation as a renderable block.
//!
//! `<tonk-display>` has two distinct defaults and they are worth not
//! conflating. `tonk:_` carries a *template* — the wildcard `directory`
//! facet, seeded in `core.yaml` as a carousel — and a terminal wants
//! its own `tui` entry there, which is an ordinary view definition. But
//! when a model has no view at all, the browser mounts something that
//! is not a template: `element.rs`'s `mount_notation_fallback` formats
//! the conclusion back into `head!:` source and highlights it. That is
//! host-side Rust in the browser, and it is host-side Rust here too.
//!
//! Both halves of it are now shared. `tonk_notation::format` turns a
//! conclusion into source; `tonk_notation::highlight` walks the parsed
//! syntax tree and returns byte-range [`Mark`]s. The browser maps each
//! [`Decoration`] onto a CSS class, and this maps the same enum onto
//! foreground tokens and SGR emphasis. Neither is CodeMirror — the
//! editor element's Lezer grammar is browser-only, which is exactly
//! why this tokenizer exists and why it ports.

use tonk_layout::{Element, Emphasis, Length, Style};
use tonk_notation::highlight::{Decoration, Mark, collect_marks};
use tonk_render::Conclusion;

/// Format one conclusion as notation source.
///
/// The host injects this per conclusion as `dom.notation/source`, so a
/// template can interpolate it into a `<notation>` element the same way
/// it interpolates any other field. That is what lets `tonk show`'s
/// output be a *view* rather than a special case: the envelope is
/// chrome and the per-instance notation is the repeat body.
pub fn source(conclusion: &Conclusion, head: &str) -> String {
    tonk_notation::format::format(&conclusion.this, &conclusion.fields, head, None)
}

/// A block of notation source, highlighted, as a column of lines.
pub fn block(source: &str) -> Element {
    let marks = collect_marks(source);
    let lines: Vec<Element> = split_lines(source, &marks)
        .iter()
        .map(|runs| line_element(runs))
        .collect();
    Element::column(lines).width(Length::Fill(1))
}

/// The whole-frame notation dump: what a terminal shows when no view
/// resolves at all.
///
/// `tonk:_` carries a *template*, but this is not one — the browser's
/// equivalent (`tonk-display/src/element.rs`'s
/// `mount_notation_fallback`) is host-side Rust too.
pub fn dump(conclusions: &[Conclusion], head: &str) -> Element {
    let mut blocks: Vec<Element> = Vec::new();
    for conclusion in conclusions {
        blocks.push(block(&source(conclusion, head)));
        blocks.push(Element::text(""));
    }
    if blocks.is_empty() {
        blocks.push(Element::text("no instances").style(dim()));
    }
    Element::column(blocks)
        .width(Length::Fill(1))
        .height(Length::Fill(1))
        .style(Style {
            width: Length::Fill(1),
            height: Length::Fill(1),
            pad: tonk_layout::Edges::xy(2, 1),
            ..Default::default()
        })
}

/// One source line as a row of independently styled runs. A `row` with
/// no spacing is how a terminal gets several styles on one line, and it
/// costs nothing: each run is measured and placed like any other
/// element.
fn line_element(runs: &[Run]) -> Element {
    if runs.is_empty() {
        return Element::text("");
    }
    Element::row(
        runs.iter()
            .map(|run| Element::text(run.text.clone()).style(style_for(run.decoration)))
            .collect(),
    )
    .width(Length::Fill(1))
}

/// A run of text carrying one decoration.
struct Run {
    text: String,
    decoration: Decoration,
}

/// Map a decoration onto a foreground token and emphasis.
///
/// Under a colourless theme every token resolves to nothing and only
/// the emphasis survives, so the same mapping serves both a full-colour
/// terminal and an ink-only one — which is the argument for tokens over
/// literals in one function.
fn style_for(decoration: Decoration) -> Style {
    let (fg, emphasis) = match decoration {
        Decoration::Plain => (None, Emphasis::default()),
        Decoration::Effect => (
            None,
            Emphasis {
                bold: true,
                ..Default::default()
            },
        ),
        Decoration::Key => (Some("muted"), Emphasis::default()),
        Decoration::NameSigil => (
            Some("muted"),
            Emphasis {
                dim: true,
                ..Default::default()
            },
        ),
        Decoration::Name => (
            Some("accent"),
            Emphasis {
                bold: true,
                ..Default::default()
            },
        ),
        Decoration::Entity => (
            Some("muted"),
            Emphasis {
                dim: true,
                ..Default::default()
            },
        ),
        Decoration::Variable => (Some("accent"), Emphasis::default()),
    };
    Style {
        fg: fg.map(str::to_string),
        emphasis,
        ..Default::default()
    }
}

fn dim() -> Style {
    Style {
        fg: Some("muted".to_string()),
        emphasis: Emphasis {
            dim: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Turn `(source, marks)` into lines of decorated runs.
///
/// Marks are sorted, non-overlapping UTF-8 byte ranges over `source`,
/// so the gaps between them are plain text. Newlines can fall inside
/// either, hence the split on every emitted segment rather than on the
/// source up front.
fn split_lines(source: &str, marks: &[Mark]) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = vec![Vec::new()];
    let push = |text: &str, decoration: Decoration, lines: &mut Vec<Vec<Run>>| {
        for (index, piece) in text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if piece.is_empty() {
                continue;
            }
            lines.last_mut().expect("a current line").push(Run {
                text: piece.to_string(),
                decoration,
            });
        }
    };

    let mut cursor = 0usize;
    for mark in marks {
        if mark.from > cursor {
            push(&source[cursor..mark.from], Decoration::Plain, &mut lines);
        }
        push(&source[mark.from..mark.to], mark.decoration, &mut lines);
        cursor = mark.to;
    }
    if cursor < source.len() {
        push(&source[cursor..], Decoration::Plain, &mut lines);
    }
    lines
}
