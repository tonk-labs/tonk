//! Syntax highlighting for the notation `tonk eval` and `tonk show`
//! already print.
//!
//! The design constraint that makes this safe to turn on by default:
//! **highlighting adds SGR and changes no glyphs**, so with colour off
//! the output is the *same bytes* it has always been. [`notation`]
//! returns a borrowed `Cow` in that case, which is the guarantee stated
//! as a type rather than a promise — and since colour is off whenever
//! stdout is not a terminal, every pipe, redirect and script sees
//! exactly today's output.
//!
//! The tokenizer is the one `<tonk-notation>` uses in the browser
//! ([`tonk_notation::highlight`]), and the `Decoration` -> SGR mapping
//! lives beside its `Decoration` -> CSS-class mapping so the two hosts
//! cannot drift.

use std::borrow::Cow;
use std::io::IsTerminal;

use tonk_notation::highlight::collect_marks;

/// Whether to colour, before looking at the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colour {
    /// Colour when stdout is a terminal that wants it.
    #[default]
    Auto,
    /// Always colour, even into a pipe.
    Always,
    /// Never colour.
    Never,
}

impl Colour {
    /// Resolve against the environment.
    ///
    /// `Auto` says no unless stdout is a terminal, `NO_COLOR` is unset
    /// (the informal standard: *any* value, including empty, disables),
    /// and `TERM` is not `dumb`.
    pub fn enabled(self) -> bool {
        match self {
            Colour::Never => false,
            Colour::Always => true,
            Colour::Auto => {
                std::io::stdout().is_terminal()
                    && std::env::var_os("NO_COLOR").is_none()
                    && !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
            }
        }
    }
}

/// Highlight notation source, or hand it back untouched.
///
/// Only SGR is inserted; no glyph is added, removed or reordered, so
/// stripping the escapes recovers the input exactly.
pub fn notation(text: &str, enabled: bool) -> Cow<'_, str> {
    if !enabled {
        return Cow::Borrowed(text);
    }
    let marks = collect_marks(text);
    if marks.is_empty() {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len() + marks.len() * 9);
    let mut cursor = 0usize;
    for mark in &marks {
        // A tokenizer that returned overlapping or unsorted marks would
        // otherwise panic on the slice; skip rather than trust it.
        if mark.from < cursor || mark.to > text.len() || mark.from > mark.to {
            continue;
        }
        out.push_str(&text[cursor..mark.from]);
        match mark.decoration.sgr() {
            Some(sgr) => {
                out.push_str("\x1b[");
                out.push_str(sgr);
                out.push('m');
                out.push_str(&text[mark.from..mark.to]);
                out.push_str("\x1b[0m");
            }
            None => out.push_str(&text[mark.from..mark.to]),
        }
        cursor = mark.to;
    }
    out.push_str(&text[cursor..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drop every SGR escape, leaving the glyphs.
    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('\x1b') {
            out.push_str(&rest[..start]);
            match rest[start..].find('m') {
                Some(end) => rest = &rest[start + end + 1..],
                None => {
                    rest = "";
                    break;
                }
            }
        }
        out.push_str(rest);
        out
    }

    const SHOW: &str = "todo:\n  this: id:1\n  title: \"port the pipeline\"\n";
    const EVAL: &str = "todo!: &first\n  this: id:1\n  title: \"port the pipeline\"\n";

    #[test]
    fn disabled_returns_the_very_same_bytes() {
        // The property the whole feature rests on: a pipe sees exactly
        // what it saw before. `Cow::Borrowed` makes it structural.
        for sample in [SHOW, EVAL] {
            let out = notation(sample, false);
            assert!(matches!(out, Cow::Borrowed(_)));
            assert_eq!(out, sample);
        }
    }

    #[test]
    fn highlighting_changes_no_glyphs() {
        for sample in [SHOW, EVAL] {
            assert_eq!(strip(&notation(sample, true)), sample);
        }
    }

    #[test]
    fn show_output_gets_marks() {
        let out = notation(SHOW, true);
        assert!(out.contains('\x1b'), "expected SGR in {out:?}");
        // Field names recede.
        assert!(out.contains("\x1b[2mtitle\x1b[0m"), "{out:?}");
    }

    #[test]
    fn eval_output_marks_the_head_and_the_anchor() {
        let out = notation(EVAL, true);
        assert!(out.contains("\x1b[1mtodo!\x1b[0m"), "head is bold: {out:?}");
        assert!(out.contains("\x1b[33m"), "anchor name is coloured: {out:?}");
    }

    #[test]
    fn text_that_does_not_parse_is_returned_untouched() {
        // `tonk eval` prefixes a YAML envelope and a `---`. Whatever
        // the tokenizer makes of a given document, output must never be
        // corrupted — worst case it is simply uncoloured.
        let junk = "revision-before: null\nclaims: 12\n---\n";
        assert_eq!(strip(&notation(junk, true)), junk);
    }

    #[test]
    fn never_and_always_ignore_the_environment() {
        assert!(!Colour::Never.enabled());
        assert!(Colour::Always.enabled());
    }
}
