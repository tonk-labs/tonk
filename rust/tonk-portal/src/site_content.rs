//! Guest HTML a `<tonk-site>` writes into its sealed iframe.
//!
//! Split out as a pure string builder so the pre-stamp placeholders it
//! carries are covered by a native unit test — `site.rs` itself is
//! `wasm32`-only.

/// The guest document a `<tonk-site>` renders in its sealed iframe: the
/// `tonk:site` display for `site`. (The guest runtime installs its relay
/// on the document at `start()`, so no wrapper element is needed.)
///
/// The display carries `slot="loading"` / `slot="no-entity"` placeholders.
/// There is a window (route match + stamp, ~1s) between the iframe booting
/// and the service worker stamping the tab's `tonk:site` during which the
/// site entity exists on the branch but carries none of its required
/// attributes (`path`, `space`, `branch`, `concept`, …). A bare
/// `<tonk-display>` reads that as a concept mismatch and flashes its loud
/// `no-entity` diagnostic (the "Concept mismatch: required attribute
/// missing" dump). Slotting these makes the display project a quiet spinner
/// for that window instead — a provided `slot` suppresses the built-in loud
/// fallback — and the real view renders in place once the stamp lands.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn guest_content(site: &str) -> String {
    format!(
        "<tonk-display entity='{site}' model='tonk:site'>\
         <div slot='loading' hidden class='site-pending'><wa-spinner></wa-spinner></div>\
         <div slot='no-entity' hidden class='site-pending'><wa-spinner></wa-spinner></div>\
         </tonk-display>"
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_mounts_the_site_display_for_the_given_entity() {
        let content = guest_content("site:abc");
        assert!(content.contains("entity='site:abc'"));
        assert!(content.contains("model='tonk:site'"));
    }

    #[dialog_common::test]
    fn it_slots_a_no_entity_placeholder_to_suppress_the_concept_mismatch_flash() {
        // The slot is what suppresses `<tonk-display>`'s loud `no-entity`
        // fallback during the pre-stamp window — without it the site flashes
        // the concept-mismatch dump before its attributes land.
        let content = guest_content("site:abc");
        assert!(content.contains(r#"slot='no-entity'"#));
    }

    #[dialog_common::test]
    fn it_slots_a_loading_placeholder_so_the_whole_window_reads_as_loading() {
        let content = guest_content("site:abc");
        assert!(content.contains(r#"slot='loading'"#));
    }
}
