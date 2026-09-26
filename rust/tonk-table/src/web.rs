use wasm_bindgen::JsValue;

/// Point the branch-resident `<tonk-table>` shell at the grid core.
///
/// The shell is no longer a script to inject: it is an `element!:` in
/// `tonk-core/assets/library/table.yaml`, which the element registry
/// resolves the first time a `<tonk-table>` is rendered. What it
/// cannot get from the branch is the core — a ~4MB IronCalc engine and
/// the program that drives it — so it asks `globalThis.__tonkTableGrid`
/// for a URL to import, and this is what answers.
///
/// `src` is the URL the grid chunk has been copied to by Trunk — pass
/// `/tonk-table/tonk-table-grid.js` if you've used the matching
/// `copy-dir` link in `index.html`. Routing the URL through the caller
/// keeps deployment-path concerns in one place: whichever crate owns
/// `index.html` decides where assets live.
///
/// Calling it more than once is harmless; the last call wins, and the
/// shell caches the resolved module itself. A page that never renders
/// a `<tonk-table>` never fetches anything.
///
/// A sealed portal guest sets the same global to a FUNCTION instead
/// (see `tonk-portal`'s `bridge.rs`), because there the core arrives
/// over `postMessage` and is blob-minted rather than fetched. The shell
/// accepts either shape.
pub fn install(src: &str) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("__tonkTableGrid"),
        &JsValue::from_str(src),
    );
}
