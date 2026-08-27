//! Projecting an ordered set of blocks into one markdown document, and
//! recovering the blocks from an edited document.
//!
//! A notebook stores **blocks**, not one markdown string. Blocks rarely
//! change, so storing the document whole would make every keystroke supersede
//! every block: a one-character edit rewrites the lot, and each revision then
//! claims all of them changed. That is wrong on its own terms, and it guts the
//! checkpoint model, which rests on revisions saying what actually moved.
//!
//! So the document the editor shows is a *projection*:
//!
//! - [`project`] joins ordered blocks into the markdown one `<tonk-prose>`
//!   edits.
//! - [`split`] takes the edited markdown back apart into blocks.
//!
//! Round-tripping is what makes this safe: `split(project(blocks)) == blocks`
//! for any blocks this module produced, so a load-edit-save cycle cannot
//! silently reshape a document the user did not touch.
//!
//! # Why a line scan rather than a parser
//!
//! prosemirror-markdown separates top-level blocks with a blank line
//! (`state.closeBlock`, `tonk-prose/src-js/editor/markdown.ts:74`), so finding
//! block boundaries is a scan for blank lines, not a parse. The one thing a
//! naive scan gets wrong is a fenced code block: blank lines *inside* a fence
//! belong to the fence. Since a notebook's cells are exactly fenced blocks,
//! that case is the whole point, and [`split`] tracks fence state.
//!
//! Nothing here needs a markdown AST, which is fortunate: the document only
//! reaches Rust as a serialized string.

/// A fence delimiter is three or more backticks or tildes. Tracking both
/// matters because a ``` fence can legally contain ~~~ and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fence {
    Backtick(usize),
    Tilde(usize),
}

impl Fence {
    /// The fence a line opens, if it opens one. The delimiter run must be at
    /// the line's start (allowing up to three spaces of indent, per CommonMark).
    fn opening(line: &str) -> Option<Fence> {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            return None;
        }
        let run = |ch: char| trimmed.chars().take_while(|c| *c == ch).count();
        match trimmed.chars().next() {
            Some('`') if run('`') >= 3 => Some(Fence::Backtick(run('`'))),
            Some('~') if run('~') >= 3 => Some(Fence::Tilde(run('~'))),
            _ => None,
        }
    }

    /// Whether `line` closes this fence: the same delimiter, at least as long,
    /// and carrying nothing else.
    fn closes(self, line: &str) -> bool {
        let trimmed = line.trim_start_matches(' ').trim_end();
        let (ch, opened) = match self {
            Fence::Backtick(n) => ('`', n),
            Fence::Tilde(n) => ('~', n),
        };
        let run = trimmed.chars().take_while(|c| *c == ch).count();
        run >= opened && trimmed.len() == run
    }
}

/// Join ordered blocks into the markdown document the editor shows.
///
/// The separator is one blank line — what prosemirror-markdown emits between
/// top-level blocks, so the projection is the same text the editor would
/// serialize from an equivalent document.
pub fn project(blocks: &[String]) -> String {
    blocks
        .iter()
        .map(|block| block.trim_matches('\n'))
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Split an edited markdown document back into blocks.
///
/// Boundaries are blank lines outside a fenced region; blank lines inside a
/// fence stay with the fence, which is what keeps a `dialog` cell whole.
pub fn split(document: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut fence: Option<Fence> = None;

    for line in document.lines() {
        match fence {
            // Inside a fence: every line belongs to this block, and only a
            // matching closer ends the fenced region.
            Some(open) => {
                current.push(line);
                if open.closes(line) {
                    fence = None;
                }
            }
            None => {
                if line.trim().is_empty() {
                    // A blank line closes the current block (and runs of
                    // blank lines collapse, since an empty block is dropped).
                    if !current.is_empty() {
                        blocks.push(current.join("\n"));
                        current.clear();
                    }
                } else {
                    fence = Fence::opening(line);
                    current.push(line);
                }
            }
        }
    }

    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_splits_paragraphs_on_blank_lines() {
        let blocks = split("first\n\nsecond\n\nthird");
        assert_eq!(blocks, vec!["first", "second", "third"]);
    }

    /// The case the whole module exists for: a `dialog` cell's body is full of
    /// blank lines, and every one of them belongs to the cell.
    #[dialog_common::test]
    fn it_keeps_a_fenced_block_whole_despite_inner_blank_lines() {
        let document = "intro\n\n```dialog\nperson:\n\n  name: ?name\n```\n\nafter";
        let blocks = split(document);
        assert_eq!(
            blocks,
            vec!["intro", "```dialog\nperson:\n\n  name: ?name\n```", "after",]
        );
    }

    /// A multi-line block that is not fenced (a list, a quote) stays one block:
    /// its lines are adjacent, so no blank line separates them.
    #[dialog_common::test]
    fn it_keeps_adjacent_lines_in_one_block() {
        let blocks = split("- one\n- two\n- three\n\nafter");
        assert_eq!(blocks, vec!["- one\n- two\n- three", "after"]);
    }

    #[dialog_common::test]
    fn it_collapses_runs_of_blank_lines() {
        let blocks = split("first\n\n\n\nsecond");
        assert_eq!(blocks, vec!["first", "second"]);
    }

    /// A tilde fence containing backticks must not end early, and vice versa.
    #[dialog_common::test]
    fn it_does_not_close_a_fence_on_a_different_delimiter() {
        let document = "~~~\n```\n\nstill inside\n~~~\n\nafter";
        let blocks = split(document);
        assert_eq!(blocks, vec!["~~~\n```\n\nstill inside\n~~~", "after"]);
    }

    /// A closer must be at least as long as its opener, so a shorter run
    /// inside the fence is content.
    #[dialog_common::test]
    fn it_requires_the_closer_to_match_the_opener_length() {
        let document = "````\n```\n\ninside\n````\n\nafter";
        let blocks = split(document);
        assert_eq!(blocks, vec!["````\n```\n\ninside\n````", "after"]);
    }

    /// An unterminated fence runs to the end rather than splitting — a
    /// document mid-typing must not fragment into blocks.
    #[dialog_common::test]
    fn it_carries_an_unclosed_fence_to_the_end() {
        let blocks = split("intro\n\n```dialog\nperson:\n\nstill typing");
        assert_eq!(blocks, vec!["intro", "```dialog\nperson:\n\nstill typing"]);
    }

    #[dialog_common::test]
    fn it_projects_blocks_with_one_blank_line_between() {
        let blocks = vec!["first".to_owned(), "second".to_owned()];
        assert_eq!(project(&blocks), "first\n\nsecond");
    }

    /// The invariant the projection rests on: splitting a projection recovers
    /// exactly the blocks it was built from.
    #[dialog_common::test]
    fn it_round_trips_blocks_through_a_document() {
        let blocks = vec![
            "# Title".to_owned(),
            "A paragraph.".to_owned(),
            "```dialog\nperson:\n\n  name: ?name\n```".to_owned(),
            "- a\n- b".to_owned(),
        ];
        assert_eq!(split(&project(&blocks)), blocks);
    }

    #[dialog_common::test]
    fn it_round_trips_an_empty_document() {
        let blocks: Vec<String> = Vec::new();
        assert_eq!(project(&blocks), "");
        assert_eq!(split(""), blocks);
    }
}
