//! Resolve the content a view's templates embed with `with:href`.
//!
//! A template declares an embed in ordinary markup:
//!
//! ```html
//! <link rel=stylesheet with:href=base>
//! ```
//!
//! What that declares is *this view needs its `base` style*, not an
//! element to render. Two facts decide where the content actually
//! lands, and both push it out of the row:
//!
//! - A view's markup is snapshotted as the **row** template
//!   ([`crate::template::snapshot_template`]), so anything left in it
//!   is cloned once per rendered row. A stylesheet cloned per row is
//!   the same rules over and over.
//! - A stylesheet is per *document*, not per element — which is why
//!   `<tonk-sigil>` injects its baseline into `<head>` rather than
//!   beside each sigil.
//!
//! So the declaration is read from the template TEXT at mount, its
//! content is fetched from the view it names, and the result is
//! injected into the document head once. Keyed by `(view, name)`, so
//! two views embedding the same style share one node and a re-mount
//! re-uses it rather than stacking duplicates — the same
//! marker-and-skip shape [`crate::component`] uses for module scripts.
//!
//! Nothing is minted as a blob URL: a `<style>` carries its text
//! directly, and a sealed guest cannot resolve a blob URL's own
//! relative `url()` anyway (which is why fonts travel as bytes to a
//! `FontFace` rather than as a `url()` inside CSS).

use web_sys::Element;

use tonk_host::consumer as host_consumer;
use tonk_schema::conclusion::Conclusion;
use tonk_template::embed::{Embed, scan};
use tonk_template::fold::{select_rows, style_content};
use tonk_template::resolve::style_query;

/// The attribute marking an injected embed, so a second mount of the
/// same view finds its node instead of adding another.
const MARKER: &str = "data-tonk-embed";

/// Resolve every embed a template declares and inject what it names.
///
/// `owner` is the view instance the template came from — the entity a
/// bare `with:href=base` reads its `base` style off. A reference
/// carrying its own entity (`base@other/view`) reads that one instead,
/// which is how two views share one stylesheet.
///
/// Silent on every failure: an embed that does not resolve leaves the
/// page unstyled, which is what the analyzer's `E_UNKNOWN_EMBED` check
/// exists to catch at lowering. By the time a document renders, a
/// missing style is a branch that has not replicated yet, not an
/// authoring mistake worth shouting about.
pub(crate) fn resolve_embeds(host: &Element, template: &str, owner: &str) {
    let embeds = scan(template);
    if embeds.is_empty() {
        return;
    }
    let host = host.clone();
    let owner = owner.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        for embed in embeds {
            inject(&host, &embed, &owner).await;
        }
    });
}

/// Fetch one embed's content and put it in the document head.
async fn inject(host: &Element, embed: &Embed, owner: &str) {
    let view = embed.entity.as_deref().unwrap_or(owner);
    let Some(document) = host.owner_document() else {
        return;
    };
    // Already injected — by an earlier mount of this view, or by
    // another view embedding the same style.
    let marker = format!("style[{MARKER}=\"{view}\u{1e}{}\"]", embed.name);
    if document.query_selector(&marker).ok().flatten().is_some() {
        return;
    }
    let Some(content) = fetch_style(host, view, &embed.name).await else {
        return;
    };
    let Some(head) = document.head() else {
        return;
    };
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute(MARKER, &format!("{view}\u{1e}{}", embed.name));
    style.set_text_content(Some(&content));
    let _ = head.append_child(&style);
}

/// One style's content, read off the view that declares it.
async fn fetch_style(host: &Element, view: &str, name: &str) -> Option<String> {
    let query = style_query(view).ok()?;
    let body = serde_wasm_bindgen::to_value(&query).ok()?;
    let rows = host_consumer::query(host, &body).await.ok()?;
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(rows).unwrap_or_default();
    // One row per key; the fold merges them into the one dictionary
    // `style_content` reads, exactly as a view's `show` folds.
    let folded = select_rows(conclusions).into_iter().next()?;
    style_content(&folded, name).map(str::to_owned)
}

/// Drop every embed this document injected.
///
/// Used by tests to isolate runs. Production has no teardown on
/// purpose: an injected style outlives the slide that asked for it,
/// because the next render of the same view wants it already there
/// rather than flashing unstyled while it is re-fetched.
#[cfg(test)]
pub(crate) fn clear_injected(document: &web_sys::Document) {
    use wasm_bindgen::JsCast as _;
    if let Ok(nodes) = document.query_selector_all(&format!("style[{MARKER}]")) {
        for index in 0..nodes.length() {
            if let Some(element) = nodes
                .item(index)
                .and_then(|node| node.dyn_ref::<Element>().cloned())
            {
                element.remove();
            }
        }
    }
}
