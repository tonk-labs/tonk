//! Browser-parity tests: render a template + frame through
//! `tonk-render` and assert the output matches what a real
//! `<tonk-view>` produced in the browser (captured via Chrome
//! DevTools, see `tests/fixtures/README.md`).
//!
//! The browser golden is normalized to drop the two artifacts the
//! stateful browser renderer leaves but one-shot SSR does not: the
//! `<tonk-view>` host wrapper and the `<!--tonk-repeat-->` /
//! `<!--tonk-iter:FIELD-->` anchor comments. After that the markup is
//! byte-identical.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_render::{Conclusion, collect_bindings, parse_fragment, render};
use tonk_template::{build_plan_nodes, split_plan, this_repeat_root};

/// Render a template + frame through the full tonk-render pipeline.
fn render_template(html: &str, frame: &[Conclusion]) -> String {
    let mut roots = parse_fragment(html);
    let bindings = collect_bindings(&mut roots);
    let repeat_root = this_repeat_root(&bindings);
    let _ = build_plan_nodes(bindings.clone());
    let plan = split_plan(bindings, repeat_root);
    render(&roots, &plan, frame)
}

/// Strip the browser-only artifacts from a captured `<tonk-view>`
/// golden so it can be compared with SSR output:
/// - unwrap the `<tonk-view>...</tonk-view>` host shell;
/// - remove `<!--tonk-repeat-->` and `<!--tonk-iter:...-->` anchors.
fn normalize_golden(golden: &str) -> String {
    let inner = golden
        .strip_prefix("<tonk-view>")
        .and_then(|s| s.strip_suffix("</tonk-view>"))
        .unwrap_or(golden);
    strip_anchor_comments(inner)
}

/// Remove `<!--tonk-repeat-->` / `<!--tonk-iter:...-->` comments.
fn strip_anchor_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<!--") {
        let (before, after_open) = rest.split_at(start);
        let Some(end) = after_open.find("-->") else {
            out.push_str(rest);
            return out;
        };
        let body = &after_open[4..end];
        out.push_str(before);
        // Keep any non-anchor comment verbatim.
        if !(body == "tonk-repeat" || body.starts_with("tonk-iter:")) {
            out.push_str(&after_open[..end + 3]);
        }
        rest = &after_open[end + 3..];
    }
    out.push_str(rest);
    out
}

fn s(v: &str) -> Ipld {
    Ipld::String(v.to_string())
}

fn row(this: &str, fields: &[(&str, Ipld)]) -> Conclusion {
    let mut map = BTreeMap::new();
    for (k, v) in fields {
        map.insert((*k).to_string(), v.clone());
    }
    Conclusion {
        this: this.to_string(),
        fields: map,
    }
}

const LIST: &str = "<ul><li data-id={this}>{name}</li></ul>";

#[test]
fn it_matches_the_browser_for_a_two_row_list() {
    // Golden captured from a real <tonk-view> via Chrome DevTools.
    let golden = "<tonk-view><ul><li data-id=\"a\" with=\"a\">Ann</li><li data-id=\"b\" with=\"b\">Bo</li><!--tonk-repeat--></ul></tonk-view>";
    let out = render_template(
        LIST,
        &[
            row("a", &[("name", s("Ann"))]),
            row("b", &[("name", s("Bo"))]),
        ],
    );
    assert_eq!(out, normalize_golden(golden));
}

#[test]
fn it_matches_the_browser_for_escaping() {
    let golden = "<tonk-view><ul><li data-id=\"x&amp;y\" with=\"x&amp;y\">&lt;b&gt;hi&lt;/b&gt;</li><!--tonk-repeat--></ul></tonk-view>";
    let out = render_template(LIST, &[row("x&y", &[("name", s("<b>hi</b>"))])]);
    assert_eq!(out, normalize_golden(golden));
}

#[test]
fn it_matches_the_browser_for_a_many_valued_iteration() {
    let golden = "<tonk-view><ul><li data-id=\"a\" with=\"a\"><span>red</span><span>blue</span><!--tonk-iter:tags--></li><!--tonk-repeat--></ul></tonk-view>";
    let tags = Ipld::List(vec![s("red"), s("blue")]);
    let out = render_template(
        "<ul><li data-id={this}><span>{tags}</span></li></ul>",
        &[row("a", &[("tags", tags)])],
    );
    assert_eq!(out, normalize_golden(golden));
}

#[test]
fn it_matches_the_browser_for_an_empty_frame() {
    let golden = "<tonk-view><ul><!--tonk-repeat--></ul></tonk-view>";
    let out = render_template(LIST, &[]);
    assert_eq!(out, normalize_golden(golden));
}
