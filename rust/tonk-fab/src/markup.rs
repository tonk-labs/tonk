//! The FAB's own markup — the bar plus the create-space wizard dialog.
//!
//! Ported from the seeded `id:tonk:profile/fab/view` template
//! (`rust/tonk-core/assets/library/profile.yaml`), which used to be the ONLY
//! copy of this markup: shipping a FAB fix meant re-seeding it onto every
//! existing space's profile branch, which older spaces never got. Now the
//! element owns its own DOM and this module is the single source of truth.
//!
//! No DOM imports — compiles and tests on the native target, like
//! [`crate::logic`].
//!
//! ## `{dom.host/data-space}` substitution
//!
//! The old view template read the active space's DID through
//! `<tonk-display>`'s `{dom.host/data-space}` escape hatch (the portal host's
//! `data-space` attribute). There is no template engine over this markup
//! anymore, so [`fab_html`] performs that substitution itself, once, at
//! render time — every zone that used to read `{dom.host/data-space}` reads
//! `space_did` here instead.
//!
//! `<ui-sync-status>` is the one exception: its own `with` attribute
//! contract is `"branch@repo"` (see `tonk-workspace::ui_sync_status`), not a
//! bare repo, so its `with` is stamped `main@{space_did}` rather than the
//! bare DID every `<tonk-display>`/`ui-space-name`/`ui-member-roster`
//! attribute below gets.
//!
//! ## Structure is authored, not inferred
//!
//! `element.rs` used to retrofit two pieces of structure onto the
//! view-rendered markup because it never had a chance to shape the DOM
//! itself: `inject_scrim` (the click-away curtain had to be a runtime-created
//! SIBLING of `.fab`, since the view renderer drops empty elements and a
//! nested scrim would be mistaken for the telescope's circle cap) and
//! `wrap_telescope_tiles` (wrapping every `.fab` child after child[0] in a
//! `.fab__tele` div, by inferring that child[0] was the circle cap). Now that
//! this module owns the whole subtree, both are authored directly: the
//! `.fab__scrim` div is a literal sibling of `.fab`, and every collapsible
//! segment is already inside its own `.fab__tele` wrapper with the resting
//! `fab--anim fab--settled` classes stamped on `.fab` itself. The tele tiles
//! are further grouped into `.fab__strip` > `.fab__page` pages (repo alone,
//! then share + account) for the compact scroll-snap pager — `display:
//! contents` on wide viewports, so that grouping renders no boxes there.
//!
//! ## The wizard's `onsubmit` cannot fire yet
//!
//! The wizard form's `onsubmit=space/create` (and `data-close-dialog`) only
//! ever worked because the OLD mount wrapped this markup in a real
//! `<tonk-display model="tonk:profile/fab">`: that element's own render pass
//! rewrites `on<event>=<concept>` attributes to `data-on<event>` and installs
//! a delegation listener (`tonk-display::events::delegate::Delegate`) on
//! itself that resolves the concept's descriptor and dispatches the claim —
//! see `rust/tonk-display/src/events/{preprocess,delegate}.rs`. That
//! delegate is per-`<tonk-display>`-instance (installed via
//! `host.add_event_listener_with_callback`, scoped by `host.contains(...)`)
//! and is never installed on `<tonk-fab>`, which sets this markup via
//! `set_inner_html` directly. Left as a literal `onsubmit=space/create`, the
//! browser would compile it as an inline JS handler (`GlobalEventHandlers`
//! covers `onsubmit` on every element) — `space / create` — which throws a
//! `ReferenceError` and, since `prevent-default` never applies, falls
//! through to the form's default GET submission (a full page reload). To
//! avoid that regression we pre-empt the rewrite ourselves: the attribute is
//! authored here as `data-onsubmit`, matching what
//! `tonk-display::events::preprocess` would have produced, which at least
//! keeps the browser from treating it as inline JS. It still does not
//! dispatch `space/create` — no delegate reads `data-onsubmit` outside a
//! `<tonk-display>`. The profile-name chip's own commit (`<ui-profile-name>`,
//! `profile_name.rs`) and the share button's mint (`<tonk-share>`,
//! `share.rs`) had the identical gap and are now fixed the same way every
//! other Rust-owned command dispatch is: hand-rolled `window.tonk.transact`
//! wiring, matching `ui-space-name::dispatch_rename` and
//! `element.rs::dispatch_pause_from_cap`. This wizard form is the one
//! dispatch still left as a follow-up.

/// Build the FAB's inner markup — the bar and the create-space dialog — for
/// the given space DID. The returned string is meant to be set as
/// `<tonk-fab>`'s `innerHTML`; it does not include the `<tonk-fab>` tags
/// themselves.
pub fn fab_html(space_did: &str) -> String {
    format!(
        r#"<div class="fab__scrim"></div>
<div class="fab fab--anim fab--settled">
  <span class="fab__seg fab__cap-l fab__circle"><ui-sync-status with="main@{space}" onpause="tonk:pause-sync"></ui-sync-status></span>
  <div class="fab__strip">
    <div class="fab__page fab__page--main">
      <div class="fab__tele fab__tele--repo">
        <span class="fab__seg fab__repo">
          <span class="fab__space"><ui-space-name space="{space}"></ui-space-name></span>
          <ui-dropdown class="fab__menu" exclude="{space}">
            <ui-space-switcher exclude="{space}"></ui-space-switcher>
          </ui-dropdown>
        </span>
      </div>
    </div>
    <div class="fab__page fab__page--more">
      <div class="fab__tele fab__tele--share">
        <span class="fab__seg fab__share">
          <tonk-share space="{space}">
            <form class="fab__share-form">
              <button type="submit" class="fab__share-trigger">
                <span class="fab__share-label fab__share-label--idle">share</span>
                <span class="fab__share-label fab__share-label--copying">
                  <span class="fab__share-spinner"></span>copying…
                </span>
                <span class="fab__share-label fab__share-label--copied">
                  <wa-icon name="check"></wa-icon>copied
                </span>
                <span class="fab__share-label fab__share-label--failed">
                  <wa-icon name="triangle-exclamation"></wa-icon>failed
                </span>
              </button>
            </form>
          </tonk-share>
          <nav class="fab__menu fab__share-menu">
            <ui-member-roster space="{space}"></ui-member-roster>
          </nav>
        </span>
      </div>
      <div class="fab__tele fab__tele--account">
        <span class="fab__seg fab__account">
          <span class="fab__name"><ui-profile-name></ui-profile-name></span>
          <a class="fab__account-link" href="/account" aria-label="Open account settings"><wa-icon name="user"></wa-icon></a>
        </span>
      </div>
    </div>
  </div>
  <div class="fab__tele fab__tele--end">
    <span class="fab__seg fab__cap-r fab__end" aria-hidden="true"></span>
    <button type="button" class="fab__seg fab__cap-r fab__more" aria-label="Show more controls"><wa-icon name="chevron-right"></wa-icon></button>
  </div>
</div>
<wa-dialog id="fab-space-create" label="New spot" class="fab__dialog" style="--width: 40rem">
  <form id="fab-space-create-form" class="fab__form wizard" data-onsubmit="space/create" data-close-dialog>
    <input class="wizard__nav" type="radio" name="__wizard" id="wiz-start" checked>
    <input class="wizard__nav" type="radio" name="__wizard" id="wiz-template">
    <input class="wizard__template" type="radio" name="template" value="blank" id="tpl-blank" checked>
    <input class="wizard__template" type="radio" name="template" value="sheets" id="tpl-sheets">
    <input class="wizard__template" type="radio" name="template" value="wiki" id="tpl-wiki">
    <input class="wizard__template" type="radio" name="template" value="board" id="tpl-board">
    <input type="hidden" name="name" value="Untitled">
    <input type="hidden" name="remote" value="">
    <tonk-default-remote field="remote" auto></tonk-default-remote>
    <div class="wizard__screen wizard__screen--start">
      <div class="wizard__cards">
        <label class="wizard__card" for="wiz-template">
          <h3>Start from a template</h3>
          <p>Seed the view with a ready-made layout.</p>
        </label>
        <label class="wizard__card" for="fab-agent-submit">
          <h3>Build with an agent</h3>
          <p>Start blank; hand the spot to an agent from inside it.</p>
        </label>
        <input type="submit" id="fab-agent-submit" form="fab-space-create-form"
               style="position:absolute;width:1px;height:1px;opacity:0;pointer-events:none;">
      </div>
    </div>
    <div class="wizard__screen wizard__screen--template">
      <div class="wizard__cards">
        <label class="wizard__card" for="tpl-wiki">
          <div class="wizard__art wizard__art--wiki">
            <span class="title"></span><i></i><i></i><i></i><i></i>
          </div>
          <h3>Wiki</h3>
          <p>Linked pages of editable blocks, with comments.</p>
        </label>
        <label class="wizard__card" for="tpl-sheets">
          <div class="wizard__art wizard__art--sheets"><span class="grid"></span></div>
          <h3>Sheets</h3>
          <p>A tabbed workspace of artifacts.</p>
        </label>
        <label class="wizard__card" for="tpl-board">
          <div class="wizard__art wizard__art--board">
            <div class="col"><i class="tall"></i><i></i></div>
            <div class="col"><i></i><i></i><i></i></div>
            <div class="col"><i></i><i class="tall"></i></div>
          </div>
          <h3>Board</h3>
          <p>Columns of cards on a pannable canvas.</p>
        </label>
        <label class="wizard__card" for="tpl-blank">
          <h3>Blank</h3>
          <p>An empty canvas to build by hand or with an agent.</p>
        </label>
      </div>
      <wa-button type="submit" form="fab-space-create-form" variant="primary">Create spot</wa-button>
      <label for="wiz-start" class="wizard__card" style="text-align:center;">&lsaquo; back to start options</label>
    </div>
  </form>
  <wa-button slot="footer" variant="neutral" appearance="plain" data-dialog="close">Cancel</wa-button>
</wa-dialog>
<wa-dialog id="fab-enable-sync" label="Turn on sync?" class="fab__dialog" style="--width: 28rem">
  <p class="fab__prompt" data-enable-sync-detail>This spot only exists on this device.</p>
  <p class="fab__prompt">Turn on sync so the people you share with can open it.</p>
  <wa-button slot="footer" variant="primary" data-enable-sync-confirm>Turn on sync &amp; copy link</wa-button>
  <wa-button slot="footer" variant="neutral" appearance="plain" data-dialog="close">Not now</wa-button>
</wa-dialog>"#,
        space = space_did
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_stamps_the_space_onto_every_cross_branch_child() {
        let html = fab_html("did:key:z6Mk");
        // Each ui- child must carry its OWN space: resolve_with reads the
        // element's own attribute and never walks ancestors.
        assert!(html.contains(r#"<ui-space-name space="did:key:z6Mk""#));
        assert!(html.contains(r#"<ui-member-roster space="did:key:z6Mk""#));
        assert!(html.contains(r#"with="main@did:key:z6Mk""#));
    }

    #[test]
    fn it_emits_telescope_wrappers_and_the_scrim_directly() {
        let html = fab_html("did:key:z6Mk");
        // Structure is authored now, not inferred from child order.
        assert!(html.contains("fab__tele"));
        assert!(html.contains("fab__scrim"));
    }

    #[test]
    fn it_carries_the_untitled_sentinel_in_the_wizard() {
        let html = fab_html("did:key:z6Mk");
        // Must be non-empty: the extractor omits blank fields, so a blank
        // name would store no fact and the create command would never fire.
        assert!(html.contains(r#"<input type="hidden" name="name" value="Untitled">"#));
    }

    #[test]
    fn it_mounts_no_tonk_display_for_deleted_views() {
        // Regression guard: `tonk:profile/name-view`, `tonk:view/fab-invite`,
        // and `tonk:repository/fab-share` are all deleted from the stdlib
        // (see this crate's module doc). A `<tonk-display>` mounting any of
        // them resolves nothing and renders a callout — "No view for
        // tonk:repository" / "Model not found" — which is the exact bug this
        // crate exists to fix. The FAB must not mount ANY `<tonk-display>`:
        // it depends on nothing seeded into a space's database.
        let html = fab_html("did:key:z6Mk");
        assert!(
            !html.contains("tonk-display"),
            "the FAB must not mount any <tonk-display> — it renders its own \
             markup from raw attributes, not a seeded view: {html}"
        );
        // Named explicitly too, so this test still catches a reintroduction
        // even if some future `<tonk-display>` use were legitimate elsewhere
        // in this markup.
        assert!(!html.contains("tonk:profile/name"));
        assert!(!html.contains("tonk:view/fab-invite"));
        assert!(!html.contains(r#"model="tonk:repository""#));
    }

    #[test]
    fn it_groups_the_tiles_into_compact_pages() {
        let html = fab_html("did:key:z6Mk");
        // Page 1 holds the space name + switcher; page 2 share then account.
        // The strip and pages are `display: contents` on wide viewports, so
        // this grouping is invisible there; compact mode makes them the
        // scroll-snap pager.
        let strip = html.find("fab__strip").expect("strip present");
        let main = html.find("fab__page--main").expect("main page present");
        let more = html.find("fab__page--more").expect("more page present");
        let repo = html.find("fab__tele--repo").expect("repo tile present");
        let share = html.find("fab__tele--share").expect("share tile present");
        let account = html
            .find("fab__tele--account")
            .expect("account tile present");
        assert!(
            strip < main && main < more,
            "strip wraps the pages in order"
        );
        assert!(
            main < repo && repo < more,
            "the repo tile is page 1's content"
        );
        assert!(
            more < share && share < account,
            "page 2 is share, then account"
        );
    }

    #[test]
    fn it_links_the_account_control_to_the_top_document_route() {
        let html = fab_html("did:key:z6Mk");
        assert!(html.contains(r#"class="fab__account-link" href="/account""#));
    }

    #[test]
    fn it_authors_the_chevron_beside_the_end_nub() {
        let html = fab_html("did:key:z6Mk");
        // Both live in the end tile, OUTSIDE the strip: the chevron is a
        // fixed right cap (like the circle on the left), never scrolled away
        // with the pages. CSS shows exactly one of the pair per mode.
        let end_tile = html.find("fab__tele--end").expect("end tile present");
        let nub = html.find("fab__end").expect("nub present");
        let more = html.find("fab__more").expect("chevron present");
        assert!(end_tile < nub && end_tile < more);
        assert!(html.contains(r#"<button type="button" class="fab__seg fab__cap-r fab__more""#));
    }

    #[test]
    fn it_authors_the_share_button_markup_directly() {
        // The share button used to live in the deleted `tonk:repository/
        // fab-share` view; it is now authored here instead. The classes must
        // match exactly — `fab.css` styles these selectors directly.
        let html = fab_html("did:key:z6Mk");
        assert!(html.contains("fab__share-trigger"));
        assert!(html.contains("fab__share-label--idle"));
        assert!(html.contains("fab__share-label--copying"));
        assert!(html.contains("fab__share-label--copied"));
        assert!(html.contains("fab__share-label--failed"));
        // `<tonk-share>` must carry its own `space` so it can subscribe to
        // this space's minted invite link and dispatch the mint itself.
        assert!(html.contains(r#"<tonk-share space="did:key:z6Mk">"#));
    }

    #[test]
    fn it_renders_the_enable_sync_prompt() {
        let html = fab_html("did:key:z6Mk");
        assert!(html.contains(r#"id="fab-enable-sync""#));
        assert!(html.contains("data-enable-sync-detail"));
        assert!(html.contains("data-enable-sync-confirm"));
        assert!(html.contains("Turn on sync &amp; copy link"));
        assert!(html.contains("Not now"));
    }
}
