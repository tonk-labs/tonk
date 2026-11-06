//! Resolve the content a view's templates embed with `with:src`.
//!
//! A template declares an embed in ordinary markup:
//!
//! ```html
//! <link rel=stylesheet with:src=base>
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
//! - A stylesheet is per *document*, not per element, so it belongs in
//!   the document once rather than beside every element that wants it.
//!
//! So the reference is resolved at LOWERING — the analyzer captures
//! which entity each one reads from, into the view's `embeds` field —
//! and at mount the renderer reads that compiled pair, fetches the
//! content, and injects the result into the document head once.
//!
//! Reading the pair rather than re-deriving it is what keeps the
//! analyzer's check and this query asking the same question. While the
//! subject came from the caller, a caller passing the wrong one — a
//! directory row's entity instead of the view's own — produced an
//! unstyled page with no error anywhere, because the check had verified
//! a name against a subject the query never asked about.
//!
//! A view lowered before that field existed carries no `embeds`, so the
//! template text is still scanned as a fallback. Keyed by `(view, name)`, so
//! two views embedding the same style share one node and a re-mount
//! re-uses it rather than stacking duplicates — the same
//! marker-and-skip shape [`crate::component`] uses for module scripts.
//!
//! The content is injected as a `<style>` rather than minted as a blob
//! URL a `<link>` points at. Not because a blob could not work — a
//! relative `url()` fails in a sealed guest either way, so neither form
//! recovers it — but because inline text needs no lifetime management,
//! while a blob URL has to be revoked or it leaks across re-renders.
//! Fonts never want a URL at all: `<font-family>` registers a
//! `FontFace` under a name, and a stylesheet references the family.

use web_sys::Element;

use tonk_host::consumer as host_consumer;
use tonk_schema::conclusion::Conclusion;
use tonk_template::embed::{Embeds, ResolvedEmbed, SRC_ATTRIBUTE, scan};
use tonk_template::fold::{font_content, select_rows, style_content};
use tonk_template::resolve::{font_query, style_query, view_embeds_query};

/// The attribute stamped on an element whose declaration has been
/// resolved, recording which `(view, name)` its content came from.
const MARKER: &str = "data-tonk-embed";

/// The element that registers a font face rather than loading a
/// stylesheet. Named for the CSS descriptor pair it stands in for: a
/// family, and the `src` its bytes come from.
const FONT_ELEMENT: &str = "FONT-FAMILY";

/// The attribute recording a minted blob URL, so one piece of content
/// is minted once per document however many elements embed it.
const MINT_MARKER: &str = "data-tonk-embed-src";

/// The attribute marking a registered font face. Separate from
/// [`MARKER`] because a font is not a `<style>`: the node records
/// that the face is on the document, it does not carry it.
const FONT_MARKER: &str = "data-tonk-embed-font";

/// Resolve every embed a template declares and inject what it names.
///
/// `owner` is the view the template came from. It is used to READ the
/// compiled `embeds` artifact, not to decide what an embed points at —
/// the subjects were resolved at lowering, so a bare `with:src=base`
/// and a cross-view `base@other/view` both arrive already paired with
/// the entity they read from.
///
/// Silent on every failure: by the time a document renders, a missing
/// style is a branch that has not replicated yet. That silence is only
/// honest because the subject is compiled in — while it was supplied by
/// the caller, a wrong subject was indistinguishable from an absent
/// style, and both said nothing.
pub(crate) fn resolve_embeds(
    host: &Element,
    template: &web_sys::HtmlTemplateElement,
    markup: &str,
    owner: &str,
) {
    // Cheap pre-check: a template that embeds nothing costs no query.
    if scan(markup).is_empty() {
        return;
    }
    let host = host.clone();
    let template = template.clone();
    let owner = owner.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let compiled = fetch_embeds(&host, &owner).await;
        // Patch the template AND whatever has already been rendered
        // from it.
        //
        // Resolving is async — two query round-trips — and rendering
        // does not wait for it. `<tonk-view>` takes its own snapshot of
        // the host's children as soon as it connects, so by the time
        // these awaits land the rows are usually already in the
        // document. Patching only the template would then write into an
        // object nobody reads again: `cloneNode` COPIES, so a clone
        // taken before the patch never sees it. That produced a minted
        // blob URL with no element pointing at it, and an unstyled page.
        //
        // Neither side is redundant. The template is what any row
        // cloned LATER inherits from; the live subtree is every row
        // cloned already. Which one wins the race varies by machine.
        for (carrier, embed) in resolvable(&template, &host, compiled, &owner) {
            inject(&host, &carrier, &embed).await;
        }
    });
}

/// Every element still awaiting its content — in the inert template and
/// in the live subtree rendered from it.
///
/// Keyed by nothing: an element already carrying [`MARKER`] is skipped
/// by [`carriers`], so a second pass over the same node is a no-op
/// rather than a second mint.
#[cfg(target_arch = "wasm32")]
fn resolvable(
    template: &web_sys::HtmlTemplateElement,
    host: &Element,
    compiled: Option<Embeds>,
    owner: &str,
) -> Vec<(Element, ResolvedEmbed)> {
    let mut found = carriers(
        &CarrierRoot::Fragment(template.content()),
        compiled.clone(),
        owner,
    );
    found.extend(carriers(
        &CarrierRoot::Element(host.clone()),
        compiled,
        owner,
    ));
    found
}

/// Where to look for unresolved declarations.
///
/// `query_selector_all` lives on both a fragment and an element but
/// through no shared web-sys trait, so the two roots are named here
/// rather than reached through a generic.
enum CarrierRoot {
    /// An inert `<template>`'s content — what later rows clone from.
    Fragment(web_sys::DocumentFragment),
    /// A live host subtree — the rows already rendered.
    Element(Element),
}

/// Pair each declaring element in the template with the content it
/// resolved to, in document order.
///
/// Keyed by the reference the element itself writes, so two elements
/// naming the same content both get it, and one whose reference did not
/// resolve is simply absent — no positional guessing between the scan
/// and the DOM.
fn carriers(
    root: &CarrierRoot,
    compiled: Option<Embeds>,
    owner: &str,
) -> Vec<(Element, ResolvedEmbed)> {
    use wasm_bindgen::JsCast as _;

    let selector = SRC_ATTRIBUTE.replace(':', "\\:");
    let query = format!("[{selector}]");
    let Ok(found) = (match root {
        CarrierRoot::Fragment(fragment) => fragment.query_selector_all(&query),
        CarrierRoot::Element(element) => element.query_selector_all(&query),
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..found.length() {
        let Some(element) = found
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        // Already resolved — by the other pass in this same call, or by
        // an earlier mount. Injecting again would mint a second blob
        // for content that is already on screen.
        if element.has_attribute(MARKER) {
            continue;
        }
        let Some(written) = element.get_attribute(SRC_ATTRIBUTE) else {
            continue;
        };
        // Parse the value the same way the scan does, so the defaults
        // (`""` -> `ui`, missing entity -> this view) are applied once
        // and in one place.
        let parsed = scan(&format!("<x {SRC_ATTRIBUTE}=\"{written}\"></x>"));
        let Some(declaration) = parsed.into_iter().next() else {
            continue;
        };
        let reference = declaration.reference();
        let resolved = match &compiled {
            // The compiled artifact is authoritative: the subject was
            // resolved at lowering, where the names were in scope.
            Some(compiled) => compiled.embeds.get(&reference).cloned(),
            // No artifact: this view predates the field. A bare
            // reference reads the view being rendered; a named one
            // cannot be resolved from here (see `resolved_embeds`).
            None => match declaration.entity {
                None => Some(ResolvedEmbed {
                    entity: owner.to_owned(),
                    name: declaration.name.clone(),
                }),
                Some(entity) if entity.contains(':') => Some(ResolvedEmbed {
                    entity,
                    name: declaration.name.clone(),
                }),
                Some(_) => None,
            },
        };
        if let Some(resolved) = resolved {
            out.push((element, resolved));
        }
    }
    out
}

/// One font's bytes, read off the view that declares it.
async fn fetch_font(host: &Element, view: &str, name: &str) -> Option<Vec<u8>> {
    let query = font_query(view).ok()?;
    let body = serde_wasm_bindgen::to_value(&query).ok()?;
    let rows = host_consumer::query(host, &body).await.ok()?;
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(rows).unwrap_or_default();
    let folded = select_rows(conclusions).into_iter().next()?;
    font_content(&folded, name).map(<[u8]>::to_vec)
}

/// Wait one macrotask turn.
///
/// Used between retries so the boot work that wires up the bridge can
/// actually run. A microtask yield is not enough: microtasks drain
/// before the event loop hands control back, so a spin on
/// `Promise::resolve` burns every attempt inside one turn and is no
/// more likely to succeed on the last than the first. `setTimeout`
/// yields to the loop itself, which is where the listener gets
/// installed.
async fn yield_frame(delay_ms: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, delay_ms);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Read a view's compiled embeds. `None` when it carries none — either
/// it embeds nothing, or it was lowered before the field existed.
async fn fetch_embeds(host: &Element, owner: &str) -> Option<Embeds> {
    let Ok(query) = view_embeds_query(owner) else {
        return None;
    };
    let Ok(body) = serde_wasm_bindgen::to_value(&query) else {
        return None;
    };
    // An unclaimed event is TRANSPORT, not absence: the bridge that
    // answers a guest's query is wired up as the guest boots, so a
    // query issued in that window comes back "no host claimed the
    // event". Treating that the same as "this view declares no embeds"
    // is how a fresh navigation lands unstyled while a warm one does
    // not — the two are indistinguishable from the result alone, which
    // is the same silent-failure shape this whole path exists to
    // remove. So a transport failure is retried across a few frames,
    // and only a real answer is taken as final.
    let mut rows = None;
    let mut last = None;
    for attempt in 0..8 {
        // A detached host will never have its event claimed — the
        // bridge listens above it in a tree it is no longer part of —
        // so retrying is waiting for something that cannot happen.
        // Checking each turn also stops a retry outliving the view that
        // asked for it.
        if !host.is_connected() {
            return None;
        }
        match host_consumer::query(host, &body).await {
            Ok(found) => {
                rows = Some(found);
                break;
            }
            Err(error) => {
                // Back off a little each time: the first turn covers a
                // listener installed later in the same boot, the later
                // ones cover a bridge still completing its handshake.
                last = Some(error);
                yield_frame(attempt * 25).await;
            }
        }
    }
    let Some(rows) = rows else {
        // Exhausted retries is a transport failure, not an absent
        // artifact, and the two lead somewhere different: this one means
        // the page is unstyled because nothing answered, which no
        // amount of waiting on the branch will fix.
        if let Some(error) = last {
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
                "<tonk-display>: no host answered this view's embeds query ({error:?}); \
                 falling back to the template's own references"
            )));
        }
        return None;
    };
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(rows).unwrap_or_default();
    let folded = select_rows(conclusions).into_iter().next()?;
    let bytes = artifact_bytes(folded.fields.get("embeds")?)?;
    match Embeds::decode(&bytes) {
        Ok(decoded) => Some(decoded),
        Err(error) => {
            // The same containment the bindings path uses: bytes that
            // are not this artifact fall back rather than being read as
            // an empty, authoritative table.
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
                "<tonk-display>: this view's compiled embeds did not decode ({error}); \
                 reading the template's own references instead"
            )));
            None
        }
    }
}

/// The bytes of a `Record`-typed field, however the wire spelled them.
///
/// A `Record` is stored as bytes, but the projection that reaches a
/// SEALED GUEST round-trips through JSON — and JSON has no byte string,
/// so the payload arrives as a list of numbers rather than as
/// `Ipld::Bytes`. The host-side path never sees this because it decodes
/// the response directly.
///
/// Accepting both spellings is what makes the artifact readable from
/// inside a guest at all. Matching only `Bytes` is why a correct
/// artifact — right claim, right query, right payload — still resolved
/// to nothing, and did it silently.
fn artifact_bytes(value: &ipld_core::ipld::Ipld) -> Option<Vec<u8>> {
    use ipld_core::ipld::Ipld;
    match value {
        Ipld::Bytes(bytes) => Some(bytes.clone()),
        Ipld::List(items) => items
            .iter()
            .map(|item| match item {
                Ipld::Integer(number) => u8::try_from(*number).ok(),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// Fetch one embed's content and give it to the element that asked.
///
/// The ELEMENT decides what the content is for, which is the only
/// honest reading: `<link rel=stylesheet>` wants a stylesheet,
/// `<font-family>` wants a face. Guessing from which dictionary the
/// name happens to live in would make `<link with:src=body>` silently
/// register a font, and the author wrote `<link>`.
async fn inject(host: &Element, carrier: &Element, embed: &ResolvedEmbed) {
    let view = embed.entity.as_str();
    let Some(document) = host.owner_document() else {
        return;
    };
    if carrier.tag_name().eq_ignore_ascii_case(FONT_ELEMENT) {
        let Some(bytes) = fetch_font(host, view, &embed.name).await else {
            return;
        };
        // The family is what the element says, falling back to the
        // embed's own name — so `<font-family with:src=body>` registers
        // `body`, and `name=` overrides it when a stylesheet wants a
        // different spelling.
        let family = carrier
            .get_attribute("name")
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| embed.name.clone());
        register_font(&document, view, &family, &bytes);
        return;
    }
    // Everything else is a stylesheet: the content becomes a blob URL
    // and the element loads it the way it would load any other. The
    // author wrote a `<link>`; it stays a `<link>`.
    let key = format!("{view}\u{1e}{}", embed.name);
    // One blob per `(view, name)`, however many elements embed it and
    // however often the view re-mounts. Minting per element would mean
    // a fresh URL on every render, and the browser refetching and
    // reapplying the same stylesheet for each one.
    let url = match minted(&document, &key) {
        Some(url) => url,
        None => {
            let Some(content) = fetch_style(host, view, &embed.name).await else {
                return;
            };
            let Some(url) = style_object_url(&content) else {
                return;
            };
            remember(&document, &key, &url);
            url
        }
    };
    let _ = carrier.set_attribute("href", &url);
    // The URL outlives this element on purpose: the row template is
    // cloned per row, and every clone carries the same `href`, so the
    // browser fetches it once and applies one stylesheet. Which is why
    // revocation cannot hang off one element's teardown —
    // [`revoke_unreferenced`] asks the document whether ANY element
    // still carries the key instead.
    let _ = carrier.set_attribute(MARKER, &key);
}

/// The blob URL already minted for this content, if any.
///
/// Recorded in the document rather than in Rust state because the
/// lifetime that matters is the DOCUMENT's: every display in it shares
/// one URL per content, and a fresh document starts over.
fn minted(document: &web_sys::Document, key: &str) -> Option<String> {
    document
        .query_selector(&format!("link[{MINT_MARKER}=\"{key}\"]"))
        .ok()
        .flatten()
        .and_then(|node| node.get_attribute("href"))
}

/// Record a minted URL so the next element embedding the same content
/// reuses it.
fn remember(document: &web_sys::Document, key: &str, url: &str) {
    use wasm_bindgen::JsCast as _;

    let Some(head) = document.head() else {
        return;
    };
    let Ok(record) = document.create_element("link") else {
        return;
    };
    // `rel=preload` rather than a bare marker: the URL is a stylesheet
    // this document is about to use, so saying so is both true and
    // useful, and keeps the record from looking like debris.
    let _ = record.set_attribute("rel", "preload");
    let _ = record.set_attribute("as", "style");
    let _ = record.set_attribute("href", url);
    let _ = record.set_attribute(MINT_MARKER, key);
    let _ = head.append_child(record.unchecked_ref::<web_sys::Node>());
}

/// Wrap stylesheet text in an object URL a `<link>` can load.
fn style_object_url(content: &str) -> Option<String> {
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(content));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/css");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Register font bytes as a family the document can use.
///
/// Bytes rather than a `url()`: the font never needs one. `@font-face`
/// points at a file because a stylesheet has no way to carry bytes; we
/// do, so the face is built directly and a stylesheet just names the
/// family.
fn register_font(document: &web_sys::Document, view: &str, family: &str, bytes: &[u8]) {
    use wasm_bindgen::JsCast as _;

    let key = format!("{view}\u{1e}{family}");
    // Already registered by an earlier mount, or by another view
    // naming the same family.
    if document
        .query_selector(&format!("meta[{FONT_MARKER}=\"{key}\"]"))
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let source = js_sys::Uint8Array::from(bytes);
    let Ok(face) = web_sys::FontFace::new_with_array_buffer_view(family, &source) else {
        web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
            "<font-family>: `{family}` could not be read as a font face"
        )));
        return;
    };
    let _ = document.fonts().add(&face);
    if let Some(head) = document.head()
        && let Ok(marker) = document.create_element("meta")
    {
        let _ = marker.set_attribute(FONT_MARKER, &key);
        let _ = head.append_child(marker.unchecked_ref::<web_sys::Node>());
    }
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

/// Revoke every minted URL nothing in this document still points at.
///
/// A blob URL is a document-lifetime resource the browser will not
/// collect on its own, and the one minted for a style is shared: every
/// row cloned from a template carries the same `href`, and a second
/// view embedding the same content reuses it. So the question is not
/// "was an element removed" but "is anything still referring to this",
/// which the DOM can answer directly — each referring element carries
/// the key on [`MARKER`].
///
/// Counted by query rather than by a maintained refcount: a counter
/// has to be decremented from exactly the places elements die, and a
/// row cloned out of a template dies in places this module never sees.
/// Asking the document cannot drift from it.
///
/// Called when a display tears down. A URL still in use by another
/// display's rows survives, because those rows are still in the
/// document and still carry the key.
pub(crate) fn revoke_unreferenced(document: &web_sys::Document) {
    use wasm_bindgen::JsCast as _;

    let Ok(records) = document.query_selector_all(&format!("link[{MINT_MARKER}]")) else {
        return;
    };
    for index in 0..records.length() {
        let Some(record) = records
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let Some(key) = record.get_attribute(MINT_MARKER) else {
            continue;
        };
        let referents = document
            .query_selector_all(&format!("[{MARKER}=\"{key}\"]"))
            .map(|found| found.length())
            .unwrap_or(0);
        if referents > 0 {
            continue;
        }
        if let Some(url) = record.get_attribute("href") {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
        // The record IS the cache entry, so dropping it is what lets a
        // later mount mint afresh rather than handing out a URL that
        // has been revoked.
        record.remove();
    }
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
    if let Ok(nodes) =
        document.query_selector_all(&format!("meta[{FONT_MARKER}], link[{MINT_MARKER}]"))
    {
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

#[cfg(test)]
mod fallback_tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A `Record` arriving as a LIST of numbers decodes.
    ///
    /// The projection that reaches a sealed guest round-trips through
    /// JSON, which has no byte string, so the artifact arrives as
    /// numbers rather than as `Ipld::Bytes`. Matching only `Bytes` made
    /// a correct artifact — right claim, right query, right payload —
    /// resolve to nothing, silently, which is exactly the failure this
    /// path exists to remove.
    #[dialog_common::test]
    fn an_artifact_decodes_from_either_wire_spelling() {
        use ipld_core::ipld::Ipld;

        let encoded = Embeds::new(std::collections::BTreeMap::from([(
            "ui@space".to_string(),
            ResolvedEmbed {
                entity: "tonk:space".into(),
                name: "ui".into(),
            },
        )]))
        .encode()
        .expect("artifact encodes");

        let as_bytes =
            artifact_bytes(&Ipld::Bytes(encoded.clone())).expect("the byte spelling is read");
        assert_eq!(as_bytes, encoded);

        let as_list = artifact_bytes(&Ipld::List(
            encoded
                .iter()
                .map(|byte| Ipld::Integer(i128::from(*byte)))
                .collect(),
        ))
        .expect("the JSON-projected list spelling is read");
        assert_eq!(as_list, encoded, "both spellings yield the same bytes");

        let decoded = Embeds::decode(&as_list).expect("and it decodes");
        assert_eq!(
            decoded.embeds.get("ui@space").map(|e| e.entity.as_str()),
            Some("tonk:space"),
        );
    }

    /// Anything that is not an artifact reads as absent rather than
    /// decoding to an empty table taken as authoritative.
    #[dialog_common::test]
    fn a_non_artifact_value_reads_as_absent() {
        use ipld_core::ipld::Ipld;

        assert!(artifact_bytes(&Ipld::String("nope".into())).is_none());
        assert!(
            artifact_bytes(&Ipld::List(vec![Ipld::String("nope".into())])).is_none(),
            "a list that is not numbers is not an artifact",
        );
        assert!(
            artifact_bytes(&Ipld::List(vec![Ipld::Integer(999)])).is_none(),
            "a number outside a byte is not an artifact",
        );
    }
}
