//! Building the asserted-notation documents the inspector's elements
//! evaluate — pure text assembly, no DOM, so it compiles and tests on
//! every target while the elements that submit it stay wasm-only.

/// The notation document one create evaluates: the `notebook/named`
/// assertion plus the draft's blocks, in a single commit.
///
/// Ported verbatim from the worker's retired `CreateNotebook` provider
/// (`create_notebook_inner`), whose two-phase evaluate this collapses
/// into one atomic document. The title is data, and goes in as a quoted
/// scalar so a colon or a quote in a notebook's name cannot change the
/// document's shape.
pub fn create_notation(entity: &str, title: &str, body: &str) -> Result<String, String> {
    let title = serde_json::to_string(title).map_err(|e| format!("unquotable title: {e}"))?;
    let mut document = format!("notebook/named!:\n  this: {entity}\n  title: {title}\n\n");

    // Carry the draft's body over, and always leave at least one block.
    //
    // Everything under the heading is content the author already typed,
    // so the notebook they land in has to open with it — otherwise
    // naming a draft silently discards the writing that prompted the
    // name. A title-only create has no body at all, and a notebook with
    // no block does not satisfy `tonk:notebook` (which requires one), so
    // the page you land on would report a missing attribute instead of
    // rendering. An empty first block is also just what a new document
    // is: somewhere to start typing.
    let mut blocks = draft_blocks(body);
    if blocks.is_empty() {
        blocks.push(String::new());
    }
    // Written back to front and chained forward by `next`, the shape the
    // library's position rules expect: a variable must be bound by an
    // earlier assertion than the one naming it.
    for (index, source) in blocks.iter().enumerate().rev() {
        document.push_str("block/insert!:\n");
        document.push_str(&format!("  this: ?b{index}\n"));
        document.push_str(&format!("  notebook: {entity}\n"));
        document.push_str(&format!("  source: {}\n", yaml_block_scalar(source)));
        if index + 1 < blocks.len() {
            document.push_str(&format!("  next: ?b{}\n", index + 1));
        } else {
            document.push_str("  next: case:none\n");
        }
        document.push_str("  prev: tonk:notebook/edge\n\n");
    }
    Ok(document)
}

/// The draft's blocks, heading and all.
///
/// The heading is KEPT. It also becomes the notebook's title, but a
/// title is metadata: a notebook's document IS its blocks projected, so
/// dropping the heading opens the new notebook without the line the
/// author just wrote — they typed `# Counter` and land on an empty page.
///
/// Blocks are separated by a blank line, which is what
/// prosemirror-markdown emits between top-level blocks.
fn draft_blocks(body: &str) -> Vec<String> {
    body.split("\n\n")
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A source as a YAML block scalar, so markdown with newlines, colons
/// and backticks survives without escaping.
fn yaml_block_scalar(source: &str) -> String {
    let mut out = String::from("|-\n");
    for line in source.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod create_notation_tests {
    use super::*;

    #[test]
    fn it_splits_a_draft_on_blank_lines_and_keeps_the_heading() {
        assert_eq!(
            draft_blocks("# Counter\n\nfirst\nstill first\n\nsecond"),
            vec!["# Counter", "first\nstill first", "second"],
        );
        assert!(
            draft_blocks("\n\n  \n").is_empty(),
            "whitespace is no block"
        );
    }

    #[test]
    fn it_quotes_a_yaml_hostile_title_as_data() {
        let notation = create_notation("notebook:n1", "a: [b] \"c\"", "").unwrap();
        assert!(
            notation.contains("  title: \"a: [b] \\\"c\\\"\""),
            "the title rides as a JSON-quoted scalar: {notation}",
        );
    }

    #[test]
    fn it_chains_blocks_back_to_front_and_seeds_one_for_an_empty_draft() {
        let notation = create_notation("notebook:n1", "T", "one\n\ntwo").unwrap();
        let first = notation
            .find("this: ?b1")
            .expect("last block asserted first");
        let second = notation
            .find("this: ?b0")
            .expect("first block asserted after");
        assert!(
            first < second,
            "back to front, so `next` names a bound variable"
        );
        assert!(
            notation.contains("next: ?b1"),
            "the first block chains forward"
        );
        assert!(
            notation.contains("next: case:none"),
            "the tail ends the chain"
        );

        let empty = create_notation("notebook:n1", "T", "").unwrap();
        assert_eq!(
            empty.matches("block/insert!:").count(),
            1,
            "a title-only create still seeds one (empty) block",
        );
    }

    #[test]
    fn it_indents_a_block_source_as_a_yaml_block_scalar() {
        assert_eq!(yaml_block_scalar("a: b\n`c`"), "|-\n    a: b\n    `c`");
    }
}
