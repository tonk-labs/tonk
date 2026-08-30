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

/// Whether a chunk is a heading (ATX `#` form — what the editor emits).
fn is_heading(chunk: &str) -> bool {
    let trimmed = chunk.trim_start_matches(' ');
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ')
}

/// Split an edited markdown document into blocks, grouping each heading with
/// the content it introduces.
///
/// A heading alone is not a unit of authorship: `## Results` followed by a
/// paragraph is one thought, and the heading exists to title what comes
/// after. Grouping them means moving a section moves its heading with it, and
/// editing under a heading does not make the heading look untouched while its
/// content churns.
///
/// A run of consecutive headings (`# Title` immediately above `## Subtitle`)
/// attaches to the same following content, so a title/subtitle pair plus a
/// code fence is one block.
///
/// The group ends at the first chunk after the content — the next heading
/// starts a new block.
pub fn split(document: &str) -> Vec<String> {
    let chunks = split_chunks(document);
    let mut blocks: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    for chunk in chunks {
        if is_heading(&chunk) {
            // A heading after content closes the previous group and opens a
            // new one; consecutive headings accumulate into the same group.
            if pending.iter().any(|held| !is_heading(held)) {
                blocks.push(pending.join("\n\n"));
                pending.clear();
            }
            pending.push(chunk);
        } else if pending.iter().any(|held| is_heading(held)) {
            // The content a pending heading was waiting for. Take it and
            // close the group.
            pending.push(chunk);
            blocks.push(pending.join("\n\n"));
            pending.clear();
        } else {
            blocks.push(chunk);
        }
    }

    if !pending.is_empty() {
        blocks.push(pending.join("\n\n"));
    }
    blocks
}

/// The bounds an inserted block landed between, as `(after, before)`.
///
/// `keys` is the surviving blocks' positions in document order, `None` where
/// a slot holds a block being created. The bounds are the nearest kept keys
/// on either side — `""` where there is none, which is what
/// `dialog/position` reads as "no bound on this side".
///
/// This is what replaces minting an entity from the document's block count.
/// The count was a position wearing an identity's clothes: delete a block and
/// add another and it repeats, so two blocks collided on one entity.
pub fn bounds_at(keys: &[Option<String>], index: usize) -> (String, String) {
    let after = keys[..index]
        .iter()
        .rev()
        .find_map(|key| key.clone())
        .unwrap_or_default();
    let before = keys
        .get(index + 1..)
        .unwrap_or(&[])
        .iter()
        .find_map(|key| key.clone())
        .unwrap_or_default();
    (after, before)
}

/// The chunk range each block covers, as `(first, count)` over the
/// document's top-level nodes.
///
/// [`split`] returns block TEXT; this returns the same grouping as spans,
/// so a caret sitting in the n-th top-level node can be traced back to the
/// block that contains it. A block is a contiguous run — a heading with the
/// content it introduces — which is why the caret's own node index is not
/// enough on its own.
pub fn block_spans(document: &str) -> Vec<(usize, usize)> {
    let chunks = split_chunks(document);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut start = 0usize;

    for (index, chunk) in chunks.into_iter().enumerate() {
        if is_heading(&chunk) {
            if pending.iter().any(|held| !is_heading(held)) {
                spans.push((start, pending.len()));
                pending.clear();
                start = index;
            }
            if pending.is_empty() {
                start = index;
            }
            pending.push(chunk);
        } else if pending.iter().any(|held| is_heading(held)) {
            pending.push(chunk);
            spans.push((start, pending.len()));
            pending.clear();
        } else {
            spans.push((index, 1));
        }
    }

    if !pending.is_empty() {
        spans.push((start, pending.len()));
    }
    spans
}

/// The span covering the top-level node at `index`, if any.
pub fn span_at(document: &str, index: usize) -> Option<(usize, usize)> {
    block_spans(document)
        .into_iter()
        .find(|(start, len)| index >= *start && index < start + len)
}

/// Split a document at blank lines outside fenced regions — the raw
/// markdown block boundaries, before headings are grouped with their content.
fn split_chunks(document: &str) -> Vec<String> {
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

/// A stored block: its entity and the source it currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The block entity.
    pub entity: String,
    /// The block's markdown source.
    pub source: String,
    /// The block's position in its notebook's `block` sequence: the key
    /// of its entry, a fractional position that sorts in byte order.
    /// `None` for a block that has a source but no entry yet.
    pub key: Option<String>,
}

/// Positions for the blocks of a document, in the order given. Each
/// item is a block entity and its current key, `None` when it has
/// none (a block just created, or one whose entry never landed).
///
/// The longest run of keys already in ascending order is kept — so a
/// block moved across its neighbours is the one re-keyed, not the
/// neighbours — and every other block gets a fresh position between
/// its nearest kept neighbours, derived from the block's own entity so
/// the same insert converges to the same key on every replica. Returns
/// only the blocks that need a new key.
pub fn assign_keys(order: &[(String, Option<String>)]) -> Vec<(String, String)> {
    use dialog_artifacts::position::{Bias, Position, insert};

    let current: Vec<Option<Position>> = order
        .iter()
        .map(|(_, key)| key.as_deref().and_then(|key| Position::try_from(key).ok()))
        .collect();
    let mut kept: Vec<Option<Position>> = vec![None; order.len()];
    for index in longest_increasing_run(&current) {
        kept[index] = current[index].clone();
    }

    let mut placed = Vec::new();
    let mut prev: Option<Position> = None;
    for (index, (entity, _)) in order.iter().enumerate() {
        if let Some(position) = &kept[index] {
            prev = Some(position.clone());
            continue;
        }
        let next = kept[index + 1..].iter().find_map(|kept| kept.clone());
        let bias = Bias::derive(entity.as_bytes());
        let position = match (&prev, &next) {
            (Some(after), Some(before)) => insert(&bias, after.clone()..before.clone()),
            (Some(after), None) => insert(&bias, after.clone()..),
            (None, Some(before)) => insert(&bias, ..before.clone()),
            (None, None) => insert(&bias, ..),
        };
        // An exhausted range cannot happen between two positions that
        // sort apart; the neighbours were kept precisely because they
        // do. Leave the block unplaced rather than misplace it.
        let Ok(position) = position else {
            continue;
        };
        placed.push((entity.clone(), position.as_str().to_owned()));
        prev = Some(position);
    }
    placed
}

/// Indices of the longest subsequence of present keys in ascending
/// order. Quadratic, which is nothing at notebook sizes.
fn longest_increasing_run(keys: &[Option<dialog_artifacts::position::Position>]) -> Vec<usize> {
    let n = keys.len();
    let mut best = vec![0usize; n];
    let mut prev = vec![usize::MAX; n];
    let mut end: Option<usize> = None;
    for i in 0..n {
        let Some(key) = &keys[i] else {
            continue;
        };
        best[i] = 1;
        for j in 0..i {
            if let Some(earlier) = &keys[j]
                && earlier.as_str() < key.as_str()
                && best[j] + 1 > best[i]
            {
                best[i] = best[j] + 1;
                prev[i] = j;
            }
        }
        if end.is_none_or(|end| best[i] > best[end]) {
            end = Some(i);
        }
    }
    let mut run = Vec::new();
    let mut cursor = end;
    while let Some(index) = cursor {
        run.push(index);
        cursor = (prev[index] != usize::MAX).then_some(prev[index]);
    }
    run.reverse();
    run
}

/// What an edit did to a notebook's blocks.
///
/// Separating these is the point: a paragraph cut from one place and pasted
/// into another is a *move*, and writing its source again would be a lie —
/// the text never changed, only where it sits. Only `order` moved, and the
/// revision should say so.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Edit {
    /// Blocks whose source changed: `(entity, new source)`. One write each.
    pub changed: Vec<(String, String)>,
    /// Sources with no stored block to attribute them to. The caller mints
    /// an entity per created block.
    pub created: Vec<String>,
    /// Blocks no longer in the document.
    pub removed: Vec<String>,
    /// The new order, as stored entities and placeholders for the created
    /// blocks (`None` marks the n-th entry of `created`, in order).
    pub order: Vec<Option<String>>,
    /// Whether the order differs from the one that was projected.
    pub reordered: bool,
}

/// Diff an edited document's blocks against the stored ones.
///
/// Matching is by content, not by position: an unchanged source keeps its
/// entity wherever it moved to, so cut-and-paste reads as a reorder. Blocks
/// are matched in one pass so duplicates (two identical paragraphs) each
/// claim a distinct stored block rather than both claiming the first.
///
/// What is left over after content matching is paired up positionally —
/// an edited block usually sits where it always did, so an unmatched new
/// source and an unmatched stored block at the same index are the same
/// block, edited. That is what keeps a typo fix from reading as
/// delete-plus-create and orphaning the block's identity.
pub fn reconcile(stored: &[Block], next: &[String]) -> Edit {
    // Content -> stored blocks with that exact source, in order. A repeated
    // source keeps every candidate so identical paragraphs stay distinct.
    let mut by_source: std::collections::HashMap<&str, std::collections::VecDeque<usize>> =
        std::collections::HashMap::new();
    for (index, block) in stored.iter().enumerate() {
        by_source
            .entry(block.source.as_str())
            .or_default()
            .push_back(index);
    }

    // Pass one: claim an untouched stored block for every source that still
    // appears verbatim.
    let mut claimed: Vec<Option<usize>> = vec![None; next.len()];
    let mut taken = vec![false; stored.len()];
    for (slot, source) in next.iter().enumerate() {
        if let Some(candidates) = by_source.get_mut(source.as_str())
            && let Some(index) = candidates.pop_front()
        {
            claimed[slot] = Some(index);
            taken[index] = true;
        }
    }

    // Pass two: pair the leftovers positionally. Walking both in order means
    // an edited block matches the stored block that occupied its place.
    let mut spare = stored
        .iter()
        .enumerate()
        .filter(|(index, _)| !taken[*index])
        .map(|(index, _)| index)
        .collect::<std::collections::VecDeque<_>>();
    for slot in claimed.iter_mut() {
        if slot.is_none()
            && let Some(index) = spare.pop_front()
        {
            *slot = Some(index);
            taken[index] = true;
        }
    }

    let mut edit = Edit::default();
    for (slot, source) in next.iter().enumerate() {
        match claimed[slot] {
            Some(index) => {
                let block = &stored[index];
                if &block.source != source {
                    edit.changed.push((block.entity.clone(), source.clone()));
                }
                edit.order.push(Some(block.entity.clone()));
            }
            None => {
                edit.created.push(source.clone());
                edit.order.push(None);
            }
        }
    }
    for (index, block) in stored.iter().enumerate() {
        if !taken[index] {
            edit.removed.push(block.entity.clone());
        }
    }

    // The order changed unless every surviving block sits where it did and
    // nothing was added or removed.
    let previous: Vec<&str> = stored.iter().map(|b| b.entity.as_str()).collect();
    let settled: Vec<Option<&str>> = edit
        .order
        .iter()
        .map(|slot| slot.as_deref())
        .collect::<Vec<_>>();
    edit.reordered = settled.len() != previous.len()
        || settled
            .iter()
            .zip(previous.iter())
            .any(|(now, before)| *now != Some(*before));

    edit
}

#[cfg(test)]
mod tests {
    // These tests run in the browser: the wasm test runner
    // otherwise looks for Node.js, which CI's web leg has not
    // got. The directive is crate-global, but declaring it per
    // module keeps it from vanishing when one module goes.
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Spans cover the same grouping `split` does: one entry per block,
    /// each naming the run of top-level nodes it occupies. The highlight
    /// reads these to light a whole block rather than one paragraph of it.
    /// An inserted block's bounds are its nearest placed neighbours.
    #[dialog_common::test]
    fn it_bounds_an_insert_by_its_nearest_neighbours() {
        let keys = vec![Some("a0".to_owned()), None, Some("a2".to_owned())];
        assert_eq!(
            bounds_at(&keys, 1),
            ("a0".to_owned(), "a2".to_owned()),
            "a block between two placed blocks is bounded by both"
        );
    }

    /// A block at either end has no neighbour on that side, and says so with
    /// the empty bound rather than omitting it — `dialog/position` reads `""`
    /// as unbounded. Getting this wrong is what would leave the first and
    /// last block of a document unplaced.
    #[dialog_common::test]
    fn it_bounds_an_end_insert_with_an_empty_string() {
        let keys = vec![None, Some("a1".to_owned()), None];
        assert_eq!(
            bounds_at(&keys, 0),
            (String::new(), "a1".to_owned()),
            "the first block has no predecessor"
        );
        assert_eq!(
            bounds_at(&keys, 2),
            ("a1".to_owned(), String::new()),
            "the last block has no successor"
        );
    }

    /// The first block of an empty notebook is unbounded on both sides.
    #[dialog_common::test]
    fn it_bounds_a_lone_insert_on_neither_side() {
        assert_eq!(bounds_at(&[None], 0), (String::new(), String::new()));
    }

    /// Consecutive inserts skip each other: an unplaced slot is not a bound,
    /// so both blocks are keyed against the placed blocks around them and the
    /// formula's member bias keeps them apart.
    #[dialog_common::test]
    fn it_skips_unplaced_slots_when_bounding() {
        let keys = vec![Some("a0".to_owned()), None, None, Some("a3".to_owned())];
        assert_eq!(bounds_at(&keys, 1), ("a0".to_owned(), "a3".to_owned()));
        assert_eq!(bounds_at(&keys, 2), ("a0".to_owned(), "a3".to_owned()));
    }

    #[dialog_common::test]
    fn it_spans_the_nodes_each_block_covers() {
        let doc = "intro\n\n# Heading\n\nunder it\n\ntrailing";
        assert_eq!(
            split(doc),
            vec![
                "intro".to_owned(),
                "# Heading\n\nunder it".to_owned(),
                "trailing".to_owned(),
            ]
        );
        assert_eq!(
            block_spans(doc),
            vec![(0, 1), (1, 2), (3, 1)],
            "the heading block covers two nodes; its neighbours one each"
        );
    }

    /// Consecutive headings ride with the same content, so the span covers
    /// all three nodes.
    #[dialog_common::test]
    fn it_spans_a_heading_run_with_its_content() {
        let doc = "# Title\n\n## Subtitle\n\nbody";
        assert_eq!(split(doc).len(), 1, "one block");
        assert_eq!(block_spans(doc), vec![(0, 3)]);
    }

    /// Every node maps back to the block containing it — that is the
    /// lookup the caret uses.
    #[dialog_common::test]
    fn it_finds_the_span_containing_a_node() {
        let doc = "intro\n\n# Heading\n\nunder it\n\ntrailing";
        assert_eq!(span_at(doc, 0), Some((0, 1)));
        assert_eq!(span_at(doc, 1), Some((1, 2)), "the heading itself");
        assert_eq!(span_at(doc, 2), Some((1, 2)), "its content, same block");
        assert_eq!(span_at(doc, 3), Some((3, 1)));
        assert_eq!(span_at(doc, 9), None, "past the end");
    }

    /// A span per block, always — whatever the document.
    #[dialog_common::test]
    fn it_spans_every_block_exactly_once() {
        for doc in [
            "one",
            "one\n\ntwo",
            "# a\n\nb\n\n# c\n\nd",
            "```dialog\nconcept:\n```\n\nafter",
            "# only a heading",
        ] {
            assert_eq!(
                block_spans(doc).len(),
                split(doc).len(),
                "span count matches block count for {doc:?}"
            );
        }
    }

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
        // Blocks as `split` itself would produce them: the heading is
        // already grouped with the paragraph it introduces.
        let blocks = vec![
            "# Title\n\nA paragraph.".to_owned(),
            "```dialog\nperson:\n\n  name: ?name\n```".to_owned(),
            "- a\n- b".to_owned(),
        ];
        assert_eq!(split(&project(&blocks)), blocks);
    }

    /// A heading titles what follows it, so the two are one unit of
    /// authorship: moving the section moves its heading with it.
    #[dialog_common::test]
    fn it_groups_a_heading_with_the_content_it_introduces() {
        let blocks = split("# Title\n\nA paragraph.\n\n# Next\n\nMore.");
        assert_eq!(blocks, vec!["# Title\n\nA paragraph.", "# Next\n\nMore."]);
    }

    /// A title/subtitle pair introduces the same content, so all three are
    /// one block.
    #[dialog_common::test]
    fn it_groups_consecutive_headings_with_one_body() {
        let blocks = split("# Title\n\n## Subtitle\n\nA paragraph.");
        assert_eq!(blocks, vec!["# Title\n\n## Subtitle\n\nA paragraph."]);
    }

    /// The cell case: a heading above a `dialog` fence is one block, so the
    /// heading travels with the cell it names.
    #[dialog_common::test]
    fn it_groups_a_heading_with_a_following_fence() {
        let document = "## Query\n\n```dialog\nconcept:\n```\n\nAfter.";
        let blocks = split(document);
        assert_eq!(
            blocks,
            vec!["## Query\n\n```dialog\nconcept:\n```", "After."]
        );
    }

    /// Only the FIRST chunk after a heading joins it — the rest of the
    /// section stands alone, so editing a later paragraph does not rewrite
    /// the heading.
    #[dialog_common::test]
    fn it_takes_only_the_first_chunk_under_a_heading() {
        let blocks = split("# Title\n\nFirst.\n\nSecond.");
        assert_eq!(blocks, vec!["# Title\n\nFirst.", "Second."]);
    }

    /// A trailing heading with nothing under it yet — the state right after
    /// typing one — is its own block rather than vanishing.
    #[dialog_common::test]
    fn it_keeps_a_trailing_heading_with_no_content() {
        let blocks = split("A paragraph.\n\n# Just typed");
        assert_eq!(blocks, vec!["A paragraph.", "# Just typed"]);
    }

    /// `#hashtag` and `#` without a space are not headings.
    #[dialog_common::test]
    fn it_does_not_read_a_bare_hash_as_a_heading() {
        let blocks = split("#hashtag\n\nA paragraph.");
        assert_eq!(blocks, vec!["#hashtag", "A paragraph."]);
    }

    fn stored(pairs: &[(&str, &str)]) -> Vec<Block> {
        pairs
            .iter()
            .map(|(entity, source)| Block {
                entity: (*entity).to_owned(),
                source: (*source).to_owned(),
                key: None,
            })
            .collect()
    }

    /// Keys already in order are kept; a block without one is placed
    /// between its neighbours, and one out of order is re-keyed.
    #[dialog_common::test]
    fn it_assigns_keys_between_kept_neighbours() {
        let first = assign_keys(&[("id:a".to_owned(), None)]);
        assert_eq!(first.len(), 1, "an only block gets a key");
        let a = first[0].1.clone();

        let appended = assign_keys(&[
            ("id:a".to_owned(), Some(a.clone())),
            ("id:b".to_owned(), None),
        ]);
        assert_eq!(appended.len(), 1, "only the new block is keyed");
        let b = appended[0].1.clone();
        assert!(a < b, "appended after: {a} < {b}");

        let between = assign_keys(&[
            ("id:a".to_owned(), Some(a.clone())),
            ("id:c".to_owned(), None),
            ("id:b".to_owned(), Some(b.clone())),
        ]);
        assert_eq!(between.len(), 1);
        let c = between[0].1.clone();
        assert!(a < c && c < b, "inserted between: {a} < {c} < {b}");

        let moved = assign_keys(&[
            ("id:b".to_owned(), Some(b.clone())),
            ("id:a".to_owned(), Some(a.clone())),
            ("id:c".to_owned(), Some(c.clone())),
        ]);
        assert_eq!(moved.len(), 1, "the block that moved is the one re-keyed");
        assert_eq!(moved[0].0, "id:b");
        assert!(moved[0].1 < a, "moved to the front: {} < {a}", moved[0].1);

        assert!(
            assign_keys(&[
                ("id:a".to_owned(), Some(a.clone())),
                ("id:b".to_owned(), Some(b.clone()))
            ])
            .is_empty(),
            "nothing to do when every key is in order"
        );
        // The keys `notebook.yaml` seeds its scratch blocks under.
        for seed in ["N1", "N5", "N9"] {
            assert!(
                dialog_artifacts::position::Position::try_from(seed).is_ok(),
                "{seed} is a position"
            );
        }
    }

    fn sources(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    /// The point of content matching: a paragraph cut and pasted elsewhere
    /// keeps its entity and writes NO source. Only the order moved.
    #[dialog_common::test]
    fn it_reads_a_moved_block_as_a_reorder_not_a_rewrite() {
        let before = stored(&[("a", "first"), ("b", "second"), ("c", "third")]);
        let edit = reconcile(&before, &sources(&["third", "first", "second"]));

        assert!(edit.changed.is_empty(), "a move rewrites nothing");
        assert!(edit.created.is_empty());
        assert!(edit.removed.is_empty());
        assert!(edit.reordered);
        assert_eq!(
            edit.order,
            vec![
                Some("c".to_owned()),
                Some("a".to_owned()),
                Some("b".to_owned())
            ]
        );
    }

    /// Editing a block in place writes that one block and leaves the order
    /// alone — the common case, and the one write amplification would ruin.
    #[dialog_common::test]
    fn it_writes_only_the_edited_block() {
        let before = stored(&[("a", "first"), ("b", "second"), ("c", "third")]);
        let edit = reconcile(&before, &sources(&["first", "second edited", "third"]));

        assert_eq!(
            edit.changed,
            vec![("b".to_owned(), "second edited".to_owned())]
        );
        assert!(edit.created.is_empty());
        assert!(edit.removed.is_empty());
        assert!(!edit.reordered, "an in-place edit is not a reorder");
    }

    #[dialog_common::test]
    fn it_reports_an_inserted_block_as_created() {
        let before = stored(&[("a", "first"), ("b", "second")]);
        let edit = reconcile(&before, &sources(&["first", "middle", "second"]));

        assert_eq!(edit.created, vec!["middle".to_owned()]);
        assert!(edit.changed.is_empty(), "neighbours are untouched");
        assert_eq!(
            edit.order,
            vec![Some("a".to_owned()), None, Some("b".to_owned())]
        );
        assert!(edit.reordered);
    }

    #[dialog_common::test]
    fn it_reports_a_deleted_block_as_removed() {
        let before = stored(&[("a", "first"), ("b", "second"), ("c", "third")]);
        let edit = reconcile(&before, &sources(&["first", "third"]));

        assert_eq!(edit.removed, vec!["b".to_owned()]);
        assert!(edit.changed.is_empty());
        assert_eq!(edit.order, vec![Some("a".to_owned()), Some("c".to_owned())]);
    }

    /// Two identical paragraphs must claim two different stored blocks, not
    /// both claim the first — otherwise one is spuriously reported removed.
    #[dialog_common::test]
    fn it_keeps_duplicate_sources_distinct() {
        let before = stored(&[("a", "same"), ("b", "same")]);
        let edit = reconcile(&before, &sources(&["same", "same"]));

        assert!(edit.changed.is_empty());
        assert!(edit.removed.is_empty());
        assert!(edit.created.is_empty());
        assert!(!edit.reordered);
    }

    /// A move AND an edit in one pass: the moved block keeps its identity,
    /// and only the genuinely edited one is written.
    #[dialog_common::test]
    fn it_separates_a_move_from_an_edit_in_the_same_pass() {
        let before = stored(&[("a", "first"), ("b", "second"), ("c", "third")]);
        let edit = reconcile(&before, &sources(&["third", "first edited", "second"]));

        assert_eq!(
            edit.changed,
            vec![("a".to_owned(), "first edited".to_owned())],
            "only the edited block is written"
        );
        assert_eq!(
            edit.order,
            vec![
                Some("c".to_owned()),
                Some("a".to_owned()),
                Some("b".to_owned())
            ]
        );
    }

    /// Nothing changed at all: no writes, no reorder. Leaving a block you
    /// only read must be free.
    #[dialog_common::test]
    fn it_reports_no_edit_when_nothing_changed() {
        let before = stored(&[("a", "first"), ("b", "second")]);
        let edit = reconcile(&before, &sources(&["first", "second"]));

        assert_eq!(
            edit,
            Edit {
                changed: Vec::new(),
                created: Vec::new(),
                removed: Vec::new(),
                order: vec![Some("a".to_owned()), Some("b".to_owned())],
                reordered: false,
            }
        );
    }

    /// The invariant a seed must satisfy: blocks written into the store have
    /// to be exactly what `split` produces from their own projection.
    /// Otherwise the first edit re-partitions the document — a heading seeded
    /// apart from its paragraph merges on parse, every later block shifts by
    /// one, and the writes land on the wrong entities.
    #[dialog_common::test]
    fn it_leaves_correctly_partitioned_blocks_alone() {
        // As `notebook.yaml` seeds them: the heading already grouped with the
        // paragraph it introduces.
        let seeded = vec![
            "# Scratch\n\nA `dialog-yaml` fence is a live cell.".to_owned(),
            "```dialog-yaml\nconcept:\n```".to_owned(),
            "Query cells run on their own.".to_owned(),
        ];
        assert_eq!(split(&project(&seeded)), seeded);

        // And the failure it guards: a heading seeded on its own merges with
        // the next block, so three stored blocks parse back as two.
        let ungrouped = vec![
            "# Scratch".to_owned(),
            "A `dialog-yaml` fence is a live cell.".to_owned(),
            "Query cells run on their own.".to_owned(),
        ];
        assert_ne!(split(&project(&ungrouped)), ungrouped);
    }

    #[dialog_common::test]
    fn it_round_trips_an_empty_document() {
        let blocks: Vec<String> = Vec::new();
        assert_eq!(project(&blocks), "");
        assert_eq!(split(""), blocks);
    }
}
