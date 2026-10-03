//! Turn an element's `method` dictionary into the JS that installs it.
//!
//! The runtime proper is `assets/element-runtime.js` — a table, one
//! generated wrapper class per tag, and `defineTonkElement(tag,
//! methods)`. This module is the other half: the pure function that
//! renders one element's facts into a call to it.
//!
//! Keeping the split here is deliberate. Everything that touches
//! `customElements` and the DOM lifecycle lives in the JS asset, where
//! it is testable in a browser without a wasm toolchain
//! (`tests/element-runtime.mjs`); everything on this side is string
//! assembly over data, testable natively. Neither half needs the other
//! to be exercised.
//!
//! The author's method sources are interpolated verbatim as JS
//! expressions. That is the same trust boundary a view template
//! already has — the branch — and the same one the older `component`
//! concept had, which carried a whole module. The one new failure mode
//! is syntactic: a malformed method makes its element's module fail to
//! parse, so that element does not register. It is contained to the one
//! element, since each gets its own module.

use std::collections::BTreeMap;

/// The global the runtime asset defines. A generated module is a call
/// to it, so a module that lands before the runtime is a no-op rather
/// than a crash — the guard below skips it.
const ENTRY: &str = "defineTonkElement";

/// Everything one `element!:` declares, as the registry read it back.
///
/// A struct rather than four parameters: they are all
/// `BTreeMap<String, String>`, so positional arguments would let a
/// caller swap two of them with nothing to catch it — and swapping
/// `getters` for `setters` produces a module that parses, registers,
/// and silently reads through the write half.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ElementDefinition {
    /// Lifecycle hooks, the `define` escape hatch, and custom methods.
    pub methods: BTreeMap<String, String>,
    /// Attribute defaults, keyed by attribute name.
    pub attributes: BTreeMap<String, String>,
    /// Property reads, keyed by property name.
    pub getters: BTreeMap<String, String>,
    /// Property writes, keyed by property name.
    pub setters: BTreeMap<String, String>,
}

/// Render the module that installs `tag`'s definition.
///
/// Keys are quoted so a kebab name (`attribute-changed`) is legal JS.
/// The optional-call guard means load order does not matter: an
/// element module evaluated before the runtime asset does nothing
/// rather than throwing, and the registry re-emits it once the runtime
/// is in place.
///
/// The maps are emitted two different ways on purpose. Methods,
/// getters and setters are authored JS and go out as EXPRESSIONS;
/// attribute defaults are data and go out as STRING LITERALS.
pub fn element_module(tag: &str, definition: &ElementDefinition) -> String {
    let mut out = String::new();
    out.push_str("globalThis.");
    out.push_str(ENTRY);
    out.push_str("?.(");
    out.push_str(&js_string(tag));
    out.push_str(", ");
    push_expressions(&mut out, &definition.methods);
    out.push_str(", ");
    push_literals(&mut out, &definition.attributes);
    out.push_str(", ");
    push_expressions(&mut out, &definition.getters);
    out.push_str(", ");
    push_expressions(&mut out, &definition.setters);
    out.push_str(");\n");
    out
}

/// An object literal whose values are the authored JS, each
/// parenthesised so an arrow function is an expression in value
/// position: `{ connected: (self) => {} }` parses, but a source that
/// begins with a newline or a comment would not.
fn push_expressions(out: &mut String, entries: &BTreeMap<String, String>) {
    out.push_str("{\n");
    for (key, source) in entries {
        out.push_str("  ");
        out.push_str(&js_string(key));
        out.push_str(": (");
        out.push_str(source.trim_end());
        out.push_str("),\n");
    }
    out.push('}');
}

/// An object literal whose values are DATA, emitted as string
/// literals. An author writing `color: red` means the three letters,
/// not an identifier to evaluate.
fn push_literals(out: &mut String, entries: &BTreeMap<String, String>) {
    out.push_str("{\n");
    for (name, value) in entries {
        out.push_str("  ");
        out.push_str(&js_string(name));
        out.push_str(": ");
        out.push_str(&js_string(value));
        out.push_str(",\n");
    }
    out.push('}');
}

/// A JS string literal for `value` — double-quoted, with the
/// characters that would end or reinterpret the literal escaped.
///
/// Method KEYS and tags are validated at authoring time, but a
/// hand-written notation document reaches the runtime without passing
/// through the CLI, so the quoting has to hold for arbitrary text
/// rather than trusting the shape.
fn js_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            // `</script` inside a string would close a classic script
            // element. The runtime is inserted as a module via
            // `textContent`, which the parser never re-scans, but the
            // escape costs nothing and keeps the output safe to embed
            // anywhere.
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Execute `source` under `key`, replacing whatever was last executed
/// under it.
///
/// Distinct from [`execute`]'s content-hash de-duplication, which is
/// wrong for a definition that can change and change BACK: reverting an
/// element to a source already seen would hash-match and be skipped,
/// leaving the realm on the newer definition forever. Keying by tag and
/// replacing means the last write wins, whatever its content.
///
/// Removing the old script does not un-run it — nothing can — but it
/// keeps `<head>` to one script per element and makes the current
/// definition of a tag findable while debugging.
#[cfg(target_arch = "wasm32")]
pub fn execute_keyed(document: &web_sys::Document, key: &str, source: &str) {
    use wasm_bindgen::JsCast;

    let Some(head) = document.head() else {
        return;
    };
    let selector = format!("script[data-tonk-element-tag=\"{key}\"]");
    if let Ok(Some(previous)) = document.query_selector(&selector) {
        previous.remove();
    }
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("type", "module");
    let _ = script.set_attribute("data-tonk-element-tag", key);
    script.set_text_content(Some(source));
    if let Some(script) = script.dyn_ref::<web_sys::HtmlScriptElement>() {
        script.set_async(false);
    }
    let _ = head.append_child(&script);
}

/// Execute `source` in `document`'s realm, once per distinct source.
///
/// Appending a created `<script>` is the one insertion path the HTML
/// spec runs — `innerHTML` and cloned fragments never do, which is why
/// a `<script>` in a view template is inert and why author JS has to
/// come through here. De-duplicated by a content hash keyed in
/// `<head>`, so the same module offered twice (two announcements of a
/// tag, a re-install) executes once.
#[cfg(target_arch = "wasm32")]
pub fn execute(document: &web_sys::Document, source: &str) {
    use wasm_bindgen::JsCast;

    let Some(head) = document.head() else {
        return;
    };
    let hash = format!("{:016x}", fnv1a64(source));
    let marker = format!("script[data-tonk-element=\"{hash}\"]");
    if document.query_selector(&marker).ok().flatten().is_some() {
        return;
    }
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("type", "module");
    let _ = script.set_attribute("data-tonk-element", &hash);
    script.set_text_content(Some(source));
    // Dynamically inserted scripts default to `async`; force document
    // order so the runtime asset is evaluated before the element
    // modules that call into it.
    if let Some(script) = script.dyn_ref::<web_sys::HtmlScriptElement>() {
        script.set_async(false);
    }
    let _ = head.append_child(&script);
}

/// FNV-1a 64-bit over the source — stable, dependency-free, and
/// collision-safe enough for "have I run this exact text".
#[cfg(any(target_arch = "wasm32", test))]
fn fnv1a64(source: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_hashes_distinct_sources_distinctly() {
        assert_ne!(fnv1a64("a"), fnv1a64("b"));
        assert_eq!(fnv1a64("same"), fnv1a64("same"));
    }
    fn methods(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// A definition with nothing but methods — the common shape.
    fn definition(pairs: &[(&str, &str)]) -> ElementDefinition {
        ElementDefinition {
            methods: methods(pairs),
            ..Default::default()
        }
    }

    #[test]
    fn it_renders_a_call_with_every_method() {
        let source = element_module(
            "tally-widget",
            &definition(&[
                ("connected", "(self) => { self.textContent = 'hi'; }"),
                ("attribute-changed", "(self, name, before, after) => {}"),
            ]),
        );
        assert!(source.starts_with("globalThis.defineTonkElement?.(\"tally-widget\", {\n"));
        assert!(source.contains("\"connected\": ((self) => { self.textContent = 'hi'; }),\n"));
        // A kebab key has to be quoted to be legal JS.
        assert!(source.contains("\"attribute-changed\": ((self, name, before, after) => {}),\n"));
        assert!(source.ends_with("});\n"));
    }

    #[test]
    fn it_guards_the_entry_point_so_load_order_cannot_throw() {
        let source = element_module("x-y", &definition(&[("connected", "(s) => {}")]));
        assert!(
            source.contains("defineTonkElement?.("),
            "an element module landing before the runtime must be a no-op: {source}",
        );
    }

    #[test]
    fn it_escapes_a_tag_or_key_that_would_break_out_of_its_literal() {
        // Neither shape survives the CLI's validation, but a
        // hand-written notation document never sees it.
        let source = element_module(
            "x\"-y",
            &definition(&[("a\\b", "(s) => {}"), ("c<d", "(s) => {}")]),
        );
        assert!(source.contains(r#""x\"-y""#), "{source}");
        assert!(source.contains(r#""a\\b""#), "{source}");
        // `<` becomes a unicode escape so the literal is safe to
        // embed in markup as well as in a module.
        assert!(source.contains(r#""c\u003cd""#), "{source}");
    }

    #[test]
    fn it_parenthesises_each_source_so_an_arrow_is_a_value() {
        let source = element_module("x-y", &definition(&[("connected", "(s) => {}\n")]));
        assert!(source.contains("\"connected\": ((s) => {}),"), "{source}");
    }

    #[test]
    fn it_renders_a_default_as_data_and_a_method_as_an_expression() {
        let source = element_module(
            "x-y",
            &ElementDefinition {
                methods: methods(&[("connected", "(s) => {}")]),
                attributes: methods(&[("color", "red"), ("size", "")]),
                ..Default::default()
            },
        );
        // A method is JS, parenthesised so an arrow is legal in value
        // position; a default is TEXT, so it goes out as a string
        // literal. `red` emitted bare would be an undefined identifier.
        assert!(source.contains("\"connected\": ((s) => {}),"), "{source}");
        assert!(source.contains("\"color\": \"red\","), "{source}");
        assert!(source.contains("\"size\": \"\","), "{source}");
    }

    #[test]
    fn it_renders_accessors_as_expressions_in_their_own_maps() {
        let source = element_module(
            "x-y",
            &ElementDefinition {
                methods: methods(&[("connected", "(s) => {}")]),
                getters: methods(&[("value", "(s) => s.textContent")]),
                setters: methods(&[("value", "(s, next) => { s.textContent = next; }")]),
                ..Default::default()
            },
        );
        // An accessor is JS, like a method, not data like a default.
        assert!(
            source.contains("\"value\": ((s) => s.textContent),"),
            "{source}"
        );
        assert!(
            source.contains("\"value\": ((s, next) => { s.textContent = next; }),"),
            "{source}"
        );
        // The getter map comes before the setter map. Swapping them
        // yields a module that parses, registers, and silently reads
        // through the write half — which is why the renderer takes one
        // struct rather than four positional maps.
        let getter = source.find("s.textContent),").expect("getter present");
        let setter = source.find("next; }),").expect("setter present");
        assert!(getter < setter, "{source}");
    }

    #[test]
    fn it_passes_every_map_even_when_only_methods_are_declared() {
        let source = element_module("x-y", &definition(&[("connected", "(s) => {}")]));
        // All four are always passed, so the runtime can CLEAR a tag's
        // defaults or accessors by re-authoring without them rather
        // than having to tell "none" from "not passed".
        assert!(source.ends_with("}, {\n}, {\n}, {\n});\n"), "{source}");
    }

    #[test]
    fn it_renders_methods_in_key_order() {
        let source = element_module(
            "x-y",
            &definition(&[("zeta", "(s) => {}"), ("alpha", "(s) => {}")]),
        );
        let alpha = source.find("alpha").expect("alpha present");
        let zeta = source.find("zeta").expect("zeta present");
        assert!(
            alpha < zeta,
            "output should be stable across runs: {source}"
        );
    }
}
