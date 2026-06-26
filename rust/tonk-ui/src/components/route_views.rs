//! `<tonk-hub>` and `<tonk-join>` — the two profile-meta-branch route views.
//!
//! Both are pure routing context: a `<tonk-display>` over a directory view
//! on the profile repository's meta branch, with no reactivity of their own.
//! The cards, the "New space" form, the join chrome — all of it comes from
//! the `space` / `tonk:join/status` directory views in the standard library.
//! These elements just inject the routing-context markup that mounts the
//! view; everything else is declarative.
//!
//! They exist as custom elements (rather than Leptos components) so the
//! shell carries no framework: the router mounts `<tonk-hub>` / `<tonk-join>`
//! the same way it mounts any element.

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

/// The Tonk Hub at `/`: a `<tonk-display concept="space">` directory over the
/// profile's meta branch. Cards link to `/space/{subject}`.
#[derive(Default)]
pub(crate) struct TonkHubElement;

impl CustomElement for TonkHubElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(ROUTE_VIEW_MARKUP_HUB);
    }

    fn connected_callback(&mut self, _this: &HtmlElement) {}
}

/// The `/join` view: a `<tonk-display concept="tonk:join/status">` directory
/// over the profile's meta branch. Its chrome holds the
/// `<tonk-page onmount=tonk:join>` trigger that fires the join command.
#[derive(Default)]
pub(crate) struct TonkJoinElement;

impl CustomElement for TonkJoinElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(ROUTE_VIEW_MARKUP_JOIN);
    }

    fn connected_callback(&mut self, _this: &HtmlElement) {}
}

// The `profile` flag routes the nested display's queries to the
// profile-as-repository endpoint, so the repository `name` is immaterial;
// `home` stands in. The `.display-route` class + `display-view-slot` are the
// shared bare-route layout (see the display route).
const ROUTE_VIEW_MARKUP_HUB: &str = r#"<main class="hub-route">
  <tonk-repository class="display-route" name="home" profile>
    <tonk-branch name="meta">
      <div class="display-view-slot">
        <tonk-display concept="space"></tonk-display>
      </div>
    </tonk-branch>
  </tonk-repository>
</main>"#;

const ROUTE_VIEW_MARKUP_JOIN: &str = r#"<main class="join-route">
  <tonk-repository class="display-route" name="home" profile>
    <tonk-branch name="meta">
      <div class="display-view-slot">
        <tonk-display concept="tonk:join/status"></tonk-display>
      </div>
    </tonk-branch>
  </tonk-repository>
</main>"#;

/// Register `<tonk-hub>` and `<tonk-join>`. Idempotent.
pub fn register() {
    if !already_registered("tonk-hub") {
        TonkHubElement::define("tonk-hub");
    }
    if !already_registered("tonk-join") {
        TonkJoinElement::define("tonk-join");
    }
}

fn already_registered(name: &str) -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get(name).is_undefined()
}
