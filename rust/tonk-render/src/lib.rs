//! Headless rendering of `tonk-display` view templates to HTML
//! strings, mirroring the browser renderer without a DOM.
//!
//! Spike phase: probing the `tl` HTML parser's child-node model
//! against the DOM child-index addressing the template planner
//! relies on. See the `spike` tests.

#[cfg(test)]
mod spike {
    use tl::{Node, NodeHandle, Parser, VDom};

    /// Render a node's children as a flat list of
    /// `(index, kind, summary)` so we can eyeball whether `tl`
    /// exposes text and element children as indexed DOM-order
    /// siblings (what the template planner's `Vec<usize>` paths
    /// assume) or collapses/omits whitespace text nodes.
    fn dump_children(handle: NodeHandle, parser: &Parser, depth: usize) -> Vec<String> {
        let mut out = Vec::new();
        let Some(node) = handle.get(parser) else {
            return out;
        };
        let children = match node {
            Node::Tag(tag) => tag.children(),
            _ => return out,
        };
        for (i, child_handle) in children.top().iter().enumerate() {
            let indent = "  ".repeat(depth);
            let summary = match child_handle.get(parser) {
                Some(Node::Tag(t)) => format!("Tag <{}>", t.name().as_utf8_str()),
                Some(Node::Raw(bytes)) => {
                    format!("Raw {:?}", bytes.as_utf8_str())
                }
                Some(Node::Comment(_)) => "Comment".to_string(),
                None => "None".to_string(),
            };
            out.push(format!("{indent}[{i}] {summary}"));
            out.extend(dump_children(*child_handle, parser, depth + 1));
        }
        out
    }

    fn first_tag(dom: &VDom, parser: &Parser) -> NodeHandle {
        *dom.children()
            .iter()
            .find(|h| matches!(h.get(parser), Some(Node::Tag(_))))
            .expect("a root tag")
    }

    /// No interior whitespace: `<ul><li ...>{name}</li></ul>`.
    /// Mirrors the render tests' `const LIST`.
    #[test]
    fn probe_tight_template() {
        let src = "<ul><li data-id={this}>{name}</li></ul>";
        let dom = tl::parse(src, tl::ParserOptions::default()).expect("parse");
        let parser = dom.parser();
        let ul = first_tag(&dom, parser);
        let lines = dump_children(ul, parser, 0);
        println!("--- tight ---\n{}", lines.join("\n"));
        // We expect: ul has one child [0] Tag <li>; li has one child
        // [0] Raw "{name}". The data-id attribute carries {this}.
        assert!(lines.iter().any(|l| l.contains("Tag <li>")));
        assert!(lines.iter().any(|l| l.contains("{name}")));
    }

    /// Interior whitespace between elements: the browser keeps
    /// these as text nodes, so child indices shift. Does `tl`?
    #[test]
    fn probe_whitespace_template() {
        let src = "<ul>\n  <li data-id={this}>\n    <span>{name}</span>\n  </li>\n</ul>";
        let dom = tl::parse(src, tl::ParserOptions::default()).expect("parse");
        let parser = dom.parser();
        let ul = first_tag(&dom, parser);
        let lines = dump_children(ul, parser, 0);
        println!("--- whitespace ---\n{}", lines.join("\n"));
        // Print only; the assertion we care about is observational
        // (does index of <li> under <ul> account for the leading
        // whitespace text node, matching DOM child_nodes()?).
        assert!(!lines.is_empty());
    }

    /// Attribute access: can we read `data-id={this}` and an
    /// attribute whose value is a `{field}` placeholder?
    #[test]
    fn probe_attribute_access() {
        let src = r#"<li data-id={this} class="row">x</li>"#;
        let dom = tl::parse(src, tl::ParserOptions::default()).expect("parse");
        let parser = dom.parser();
        let li = first_tag(&dom, parser);
        let Some(Node::Tag(tag)) = li.get(parser) else {
            panic!("expected tag");
        };
        let attrs: Vec<String> = tag
            .attributes()
            .iter()
            .map(|(k, v)| format!("{k}={:?}", v.map(|v| v.to_string())))
            .collect();
        println!("--- attrs ---\n{}", attrs.join("\n"));
        assert!(attrs.iter().any(|a| a.starts_with("data-id")));
        assert!(attrs.iter().any(|a| a.starts_with("class")));
    }
}
