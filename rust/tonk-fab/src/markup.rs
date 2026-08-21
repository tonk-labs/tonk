//! The bar's markup and its component-local CSS.
//!
//! No DOM imports — this compiles and tests on the native target, like
//! [`crate::logic`] and [`crate::skin`], so the geometry laws are checked by
//! a plain `cargo test` rather than only under wasm.
//!
//! The strings here are the bar's half of the FABB spec; [`crate::bar`] owns
//! the behaviour that drives them. The shared token block they layer over
//! lives in [`crate::skin::SKIN`].
//!
//! ## The absent rung
//!
//! The spec's bar is `[circle 36][space 216][changes 432][share 144][fold
//! 24][mode 18]`. The `changes` rung is omitted here — it drives preview /
//! accept / discard / restore over proposals and history points, and this
//! repo implements neither. See `plan/fabb-conformance.md`.

/// The `.w` state classes and cell geometry, layered over
/// [`crate::skin::SKIN`] in the bar's shadow root.
pub const BAR_CSS: &str = r#"
:host{ display:inline-block;
  transition:left .4s cubic-bezier(0.25,0.46,0.45,0.94), top .4s cubic-bezier(0.25,0.46,0.45,0.94), transform .2s cubic-bezier(0.25,0.46,0.45,0.94); }
:host([dragging]){ transition:none; }
:host([hidden]){ display:none; }
@media (prefers-reduced-motion: reduce){ :host{ transition:none; } }
.w{ position:relative; }
.bar{ position:relative; display:flex; align-items:stretch; height:36px;
  border-radius:100px; overflow:hidden; user-select:none;
  background:var(--_bg); -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  box-shadow:var(--_ring); }
.cell{ display:flex; align-items:flex-end; justify-content:flex-end; gap:8px;
  padding:0 10px 9px 0; font-size:13px; line-height:1; color:var(--_ink);
  white-space:nowrap; overflow:hidden; cursor:pointer; flex:none; }
.cell:hover{ background:var(--_hover); }
.cell:active{ background:var(--_press); }
.chrome{ text-transform:lowercase; }
.fab{ width:36px; align-items:center; justify-content:center; padding:0; cursor:grab; touch-action:none; }
:host([dragging]) .fab{ cursor:grabbing; }
/* fixed cells — widths never follow content (law 7) */
.space{ width:216px; padding-left:12px; text-transform:none; }
.share{ width:144px; }
/* the frost/sachlich triangle: label-size 14px at 500 — smaller or heavier
   and it goes stubby */
.fold{ width:24px; align-items:flex-end; justify-content:center; padding:0 0 9px;
  font-size:14px; font-weight:500; line-height:1; color:var(--_ink); }
.toggle{ width:18px; background:var(--_ink); padding:0; }
.toggle:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_ink); }
.toggle:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_ink); }
.cell + .cell{ border-left:1px solid var(--_sep); }
/* the strip is a sibling of the circle, not a cell, so `.cell + .cell` cannot
   reach across the boundary where they meet — the one seam in the run it
   misses. Unflipped that seam is the strip's leading edge; flipped the strip
   leads the bar (nothing to its left) and the seam moves to the circle. */
.tele > .cell:first-child{ border-left:1px solid var(--_sep); }
.w.flip .tele > .cell:first-child{ border-left:0; }
.w.flip .fab{ border-left:1px solid var(--_sep); }
/* the space cell carries a user word — it passes through untouched (law 4) */
.space .n{ overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  /* descender room inside the clip; the negative margin holds the seat */
  padding-bottom:4px; margin-bottom:-4px; }
/* the alert law: blinking, never a colour. collapsed → the disc blinks;
   expanded → the alerted rung washes. pointing at it calms it. */
:host([alert]) .disc.st{ animation:fabb-blink var(--_blink) var(--_ease) infinite; }
:host([alert]) .share{ animation:fabb-wash var(--_blink) var(--_ease) infinite; }
:host([alert]) .share:hover{ animation:none; }
@media (prefers-reduced-motion: reduce){
  :host([alert]) .disc.st, :host([alert]) .share{ animation:none !important; } }
/* telescope collapse · the cell strip pans when it outgrows its room */
.tele{ display:flex; align-items:stretch; max-width:var(--_telemax, 1100px);
  overflow-x:auto; overflow-y:hidden; transition:max-width .4s var(--_ease);
  scrollbar-width:none; touch-action:pan-x; overscroll-behavior-x:contain; }
.tele::-webkit-scrollbar{ display:none; }
:host([collapsed]) .tele{ max-width:0; }
/* fold: one triangle reduces the bar to circle · space · fold · mode.
   .folded = the hand; .rfold = the ResizeObserver; .xopen = unfolded over it */
.w.folded .share, .w.rfold:not(.xopen) .share{ display:none; }
/* A missing replica leaves the space cell available as the way out, but
   removes the share control because there is nothing local to share. */
:host([data-unknown-space]) .share{ display:none; }
.w.rd .tele{ max-width:0; }
/* stacks */
.mw{ position:absolute; top:calc(100% + 7px); display:none; z-index:5; }
:host([up]) .mw{ top:auto; bottom:calc(100% + 7px); }
.mw.on{ display:block; }
/* editable space — the terminal block cursor over the last character */
.cell.editing{ gap:0; }
@media (prefers-reduced-motion: reduce){ .tele{ transition:none; } }
"#;

/// The bar's shadow tree.
///
/// `.tele` holds everything that telescopes; the circle and the trailing caps
/// are bookends [`crate::bar::apply_flip`] moves between the ends.
pub const BAR_HTML: &str = r#"<div class="w">
  <div class="bar" part="bar">
    <button class="cell fab" part="fab" title="collapse / expand · drag to move"><span class="disc st"></span></button>
    <div class="tele">
      <button class="cell space" data-cell="space" title="space name" aria-haspopup="true" aria-expanded="false"><span class="n"></span></button>
      <button class="cell share chrome" data-cell="share" title="share with others" aria-haspopup="true" aria-expanded="false">share</button>
      <button class="cell fold" data-cell="fold" title="fold / expand">&#9666;</button>
      <button class="cell toggle" data-cell="toggle" role="switch" title="dark / light" aria-label="dark mode"></button>
    </div>
  </div>
  <div class="mw" part="menus"><slot name="menu"></slot></div>
</div>"#;

/// The gap between the bar and a stack, and between blocks within one — the
/// 7px of pure page that makes a stack many blocks rather than one panel
/// (law 2). One number, referenced by the CSS above and asserted below.
pub const STACK_GAP_PX: i32 = 7;

/// The bar's light-DOM children: its two stacks, and the headless
/// subscribers that feed it.
///
/// The stacks are slotted (`slot="menu"`), so they render; the subscribers
/// are not, so they do not. That is deliberate — an unslotted light child of
/// a shadow host is never rendered, which is exactly what a headless element
/// wants. `<ui-space-name>` and `<ui-sync-status>` subscribe to their space
/// and write `label` and `state` onto the bar, so the bar renders text and a
/// disc it owns rather than hosting foreign elements inside its cells.
///
/// The space stack IS the bar's information architecture:
/// `new · open ▸ · rename`. `open`'s sub-stack is filled by
/// `<ui-space-switcher>`; the share stack's roster by `<ui-member-roster>`.
///
/// The spec's `settings` row is absent for the same reason as the `changes`
/// rung: there are no space settings for it to open. It comes back with the
/// surface it leads to.
///
/// ## Glyphs
///
/// Every mark is geometry, not illustration (see the FABB glyph table): `+`
/// for new, `▸` for open, `↖` for leaving the environment, and a 6×12 ink
/// block for rename — the terminal block cursor again, as a noun. No icon
/// library.
pub const STACKS_HTML: &str = r#"<ui-sync-status headless with="main@{space}"></ui-sync-status>
<ui-space-name headless space="{space}"></ui-space-name>
<tonk-menu slot="menu" data-for="space">
  <tonk-mi chrome data-mi-new>new<span class="g">+</span></tonk-mi>
  <tonk-mi chrome data-mi-open>open<span class="g">&#9656;</span>
    <tonk-menu slot="sub">
      <ui-space-switcher exclude="{space}"></ui-space-switcher>
      <tonk-mi muted chrome data-mi-home title="back to the directory at home">more<span class="g">&#8598;</span></tonk-mi>
    </tonk-menu>
  </tonk-mi>
  <tonk-mi chrome data-mi-rename>rename<span class="g rename-mark" aria-hidden="true"></span></tonk-mi>
</tonk-menu>
<tonk-share headless space="{space}"></tonk-share>
<tonk-menu slot="menu" data-for="share">
  <tonk-mi chrome data-share-link>
    <span class="say say--idle">copy link</span>
    <span class="say say--copying">copying&hellip;</span>
    <span class="say say--copied">copied</span>
    <span class="say say--failed">couldn&rsquo;t copy</span>
  </tonk-mi>
  <ui-member-roster space="{space}"></ui-member-roster>
</tonk-menu>"#;

/// Styles for the slotted stack content.
///
/// These rules cannot live in a component's shadow CSS: slotted content is
/// styled by the DOCUMENT, and document styles beat `::slotted()`. So the
/// marks a stack row carries are painted here, in the light tree, next to the
/// markup that uses them.
pub const STACKS_CSS: &str = r#"
/* the rename glyph — the block cursor as a noun, at the label's own size */
tonk-fab .rename-mark{ display:inline-block; width:6px; height:12px; background:currentColor; }
/* the headless subscribers render nothing; they are unslotted, but say so */
tonk-fab > ui-sync-status[headless],
tonk-fab > ui-space-name[headless],
tonk-fab > tonk-share[headless]{ display:none; }
/* the row producers render their rows as SIBLINGS (see stack_rows), so they
   hold nothing themselves — laid out they would only add a stack gap where
   they sit */
tonk-fab ui-space-switcher, tonk-fab ui-member-roster{ display:none; }
/* the share row answers in place: one word at a time, the copy state
   choosing which. idle is the default, so a row that has never been used —
   and one whose element never stamped a state — still reads "copy link". */
tonk-fab [data-share-link] .say{ display:none; }
/* idle before the element has ever stamped a state, and whenever it says so.
   `blocked` also reads as idle: a prompt is up asking the user a question,
   and the row behind it is offering the retry, not reporting a failure. */
tonk-fab [data-share-link]:not([data-share-state]) .say--idle,
tonk-fab [data-share-link][data-share-state="idle"] .say--idle,
tonk-fab [data-share-link][data-share-state="blocked"] .say--idle{ display:inline; }
tonk-fab [data-share-link][data-share-state="copying"] .say--copying{ display:inline; }
tonk-fab [data-share-link][data-share-state="copied"] .say--copied{ display:inline; }
tonk-fab [data-share-link][data-share-state="failed"] .say--failed{ display:inline; }
"#;

/// The share flow's repairable sync refusal.
///
/// Every member can share through its own delegation chain. The remaining
/// prompt handles a missing sync remote; `share.rs` rewrites its marked lines
/// per refusal class and drives the dialog's `open` property.
///
/// Mounted on `<body>` rather than inside the bar: these are modals, and an
/// unslotted light-DOM child of a shadow host never renders, so a dialog
/// parked there could not be shown at all.
///
/// The action run reads left-to-right as dismiss-then-commit, and the two
/// fuse flush — the fill boundary between quiet and primary IS the divider
/// (law 3), which is why there is no gap and no separator between them.
pub const REFUSAL_DIALOGS_HTML: &str = r#"<tonk-dialog id="fab-enable-sync" heading="turn on sync?">
  <p data-enable-sync-detail>This spot only exists on this device.</p>
  <p data-enable-sync-action>Turn on sync so the people you share with can open it.</p>
  <tonk-button slot="actions" variant="quiet" data-dialog="close">not now</tonk-button>
  <tonk-button slot="actions" variant="primary" data-enable-sync-confirm>turn on sync &amp; copy link</tonk-button>
</tonk-dialog>
"#;

/// Stamp the space DID into [`STACKS_HTML`].
///
/// Each cross-branch child carries its OWN `space` / `with`: the routing
/// helpers read the element's own attribute and never walk ancestors, so a
/// child left unstamped is pointed at nothing rather than inheriting.
///
/// `<ui-sync-status>` is the one exception to the bare-DID form — its `with`
/// contract is `"branch@repo"` (see `tonk-workspace::ui_sync_status`), so it
/// is stamped `main@{did}`, which the template already spells out.
pub fn stacks_html(space_did: &str) -> String {
    STACKS_HTML.replace("{space}", space_did)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_omits_the_changes_rung() {
        // Deliberate — see the module docs and plan/fabb-conformance.md.
        // This guards against it reappearing as dead chrome.
        assert!(!BAR_HTML.contains("data-cell=\"changes\""));
        assert!(!BAR_CSS.contains(".changes"));
    }

    #[test]
    fn it_fixes_every_cell_width() {
        // Law 7: widths never follow content; breakpoints swap whole rungs.
        for (cell, width) in [
            (".fab", "36px"),
            (".space", "216px"),
            (".share", "144px"),
            (".fold", "24px"),
            (".toggle", "18px"),
        ] {
            assert!(
                BAR_CSS.contains(&format!("{cell}{{ width:{width}")),
                "{cell} must be fixed at {width}",
            );
        }
    }

    #[test]
    fn it_seats_the_stack_one_gap_off_the_bar() {
        assert!(BAR_CSS.contains(&format!("top:calc(100% + {STACK_GAP_PX}px)")));
        assert!(BAR_CSS.contains(&format!("bottom:calc(100% + {STACK_GAP_PX}px)")));
    }

    #[test]
    fn it_lets_the_user_word_through_untouched() {
        // Law 4: chrome is lowercase; names and spaces pass through. The
        // share cell is chrome, the space cell carries a user word and must
        // opt out of the transform.
        assert!(BAR_CSS.contains("text-transform:none"));
        assert!(BAR_HTML.contains(r#"class="cell share chrome""#));
        assert!(
            !BAR_HTML.contains(r#"class="cell space chrome""#),
            "the space cell must not lowercase the name it shows",
        );
    }

    #[test]
    fn it_alerts_without_a_colour() {
        // Law 5: ink only. Alerts blink or wash; pointing at one calms it.
        assert!(BAR_CSS.contains(":host([alert]) .disc.st{ animation:fabb-blink"));
        assert!(BAR_CSS.contains(":host([alert]) .share{ animation:fabb-wash"));
        assert!(BAR_CSS.contains(":host([alert]) .share:hover{ animation:none; }"));
    }

    #[test]
    fn it_keeps_the_way_out_but_hides_share_for_an_unknown_space() {
        assert!(BAR_CSS.contains(":host([data-unknown-space]) .share{ display:none; }"));
        assert!(BAR_HTML.contains(r#"data-cell="space""#));
    }

    #[test]
    fn it_draws_the_seam_the_run_rule_cannot_reach() {
        // `.cell + .cell` stops at the strip boundary, and that boundary
        // swaps ends with the flip. Both sides need saying, or the bar shows
        // one missing separator in one orientation and a doubled one in the
        // other.
        assert!(
            BAR_CSS.contains(".tele > .cell:first-child{ border-left:1px solid var(--_sep); }")
        );
        assert!(BAR_CSS.contains(".w.flip .tele > .cell:first-child{ border-left:0; }"));
        assert!(BAR_CSS.contains(".w.flip .fab{ border-left:1px solid var(--_sep); }"));
    }

    #[test]
    fn it_holds_the_bookends_outside_the_telescope() {
        // The circle and the trailing caps must not scroll away with the
        // cells: collapse has to stay reachable at every width.
        let tele = BAR_HTML.find(r#"<div class="tele">"#).expect("a telescope");
        let circle = BAR_HTML.find("cell fab").expect("the circle");
        assert!(
            circle < tele,
            "the circle is a bookend, not a telescoped cell"
        );
    }

    #[test]
    fn it_stamps_the_space_onto_every_cross_branch_child() {
        let html = stacks_html("did:key:z6Mk");
        // Each child must carry its OWN space: the routing helpers read the
        // element's own attribute and never walk ancestors, so an unstamped
        // child is pointed at nothing.
        assert!(html.contains(r#"<ui-space-name headless space="did:key:z6Mk""#));
        assert!(html.contains(r#"<ui-member-roster space="did:key:z6Mk""#));
        assert!(html.contains(r#"<ui-space-switcher exclude="did:key:z6Mk""#));
        // The sync disc's contract is branch@repo, not a bare DID.
        assert!(html.contains(r#"with="main@did:key:z6Mk""#));
        assert!(!html.contains("{space}"), "every slot must be substituted");
    }

    #[test]
    fn it_carries_the_bars_information_architecture() {
        // The space stack IS the IA: new · open ▸ · rename, in that order.
        let html = stacks_html("did:key:z6Mk");
        let order: Vec<usize> = ["data-mi-new", "data-mi-open", "data-mi-rename"]
            .iter()
            .map(|hook| html.find(hook).unwrap_or_else(|| panic!("{hook} present")))
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the space stack must read new · open · rename",
        );
        assert!(
            !html.contains("data-mi-settings"),
            "no settings row until there are settings to open",
        );
    }

    #[test]
    fn it_offers_the_way_back_out_of_the_space() {
        // `more ↖` is the only route from a space back to the directory, and
        // it must sit at the END of the spaces sub-stack, after the spaces
        // the switcher fills in.
        let html = stacks_html("did:key:z6Mk");
        let switcher = html.find("ui-space-switcher").expect("the switcher");
        let more = html.find("data-mi-home").expect("the way home");
        assert!(switcher < more, "more ↖ comes after the spaces");
        assert!(html.contains("&#8598;"), "↖ marks leaving the environment");
    }

    #[test]
    fn it_keeps_the_subscribers_headless() {
        // They write `label` and `state` onto the bar; they must render
        // nothing themselves. Unslotted is what achieves that — a light child
        // with no slot never renders inside a shadow host — and the CSS says
        // so out loud.
        let html = stacks_html("did:key:z6Mk");
        for headless in ["ui-sync-status", "ui-space-name", "tonk-share"] {
            let tag = html
                .split(&format!("<{headless}"))
                .nth(1)
                .expect("the subscriber");
            let tag = tag.split('>').next().expect("the tag closes");
            assert!(
                tag.contains("headless"),
                "{headless} must be marked headless"
            );
            assert!(
                !tag.contains("slot="),
                "{headless} must not be slotted, or it would render",
            );
            assert!(
                STACKS_CSS.contains(&format!("tonk-fab > {headless}[headless]")),
                "{headless} must be hidden explicitly, not only by being unslotted",
            );
        }
    }

    #[test]
    fn it_answers_the_share_in_place() {
        // One word at a time. Every state the element can stamp needs a
        // word, or the row goes blank mid-copy.
        for state in ["idle", "copying", "copied", "failed", "blocked"] {
            assert!(
                STACKS_CSS.contains(&format!(r#"[data-share-state="{state}"]"#)),
                "the share row must have a word for {state}",
            );
        }
        // And before the element has stamped anything at all.
        assert!(STACKS_CSS.contains(r#"[data-share-link]:not([data-share-state]) .say--idle"#));
    }

    #[test]
    fn it_leaves_remote_selection_to_the_worker() {
        let html = stacks_html("did:key:z6Mk");
        assert!(!html.contains("tonk-default-remote"));
        assert!(!html.contains(r#"name="remote""#));
        assert!(!html.contains(r#"name="revocation""#));
    }

    #[test]
    fn it_draws_its_marks_as_geometry() {
        // No icon library: circles, blocks, triangles, hairlines. The
        // settings mark is the one bespoke SVG, and it must stay on
        // currentColor so it follows the ink in both modes.
        let html = stacks_html("did:key:z6Mk");
        assert!(
            html.contains(r#"<span class="g">+</span>"#),
            "new is a plus"
        );
        assert!(html.contains("&#9656;"), "open is the ▸ triangle");
        assert!(html.contains("rename-mark"), "rename is the block cursor");
        assert!(!html.contains("<wa-icon"), "no icon library in the chrome");
        assert!(!html.contains("<svg"), "and no illustration either");
    }

    #[test]
    fn it_gives_the_sync_refusal_its_prompt() {
        assert!(REFUSAL_DIALOGS_HTML.contains(r#"id="fab-enable-sync""#));
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-confirm"));
        // The line share.rs rewrites per refusal class. Without the hook the
        // prompt is silently stuck on the `not-synced` wording.
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-detail"));
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-action"));
        assert!(!REFUSAL_DIALOGS_HTML.contains("fab-join-first"));
    }

    #[test]
    fn it_offers_a_way_out_of_every_prompt() {
        // Each cluster needs a dismiss; `data-dialog=close` is what the
        // dialog's own handler acts on.
        assert_eq!(
            REFUSAL_DIALOGS_HTML
                .matches(r#"data-dialog="close""#)
                .count(),
            1
        );
    }

    #[test]
    fn it_keeps_prompt_chrome_lowercase() {
        // Law 4. The prompt BODIES carry sentences and stay as written; the
        // headings and the action labels are chrome.
        assert!(REFUSAL_DIALOGS_HTML.contains(r#"heading="turn on sync?""#));
        for label in ["not now", "turn on sync &amp; copy link"] {
            assert!(REFUSAL_DIALOGS_HTML.contains(&format!(">{label}</tonk-button>")));
        }
    }

    #[test]
    fn it_respects_reduced_motion_on_every_transition() {
        // Each animated property needs its own opt-out; the telescope's
        // max-width transition is the one that reads as motion.
        let blocks: Vec<&str> = BAR_CSS
            .match_indices("@media (prefers-reduced-motion: reduce)")
            .map(|(index, _)| &BAR_CSS[index..])
            .collect();
        assert!(
            blocks.iter().any(|b| b.contains(":host{ transition:none;")),
            "the host's own glide must stop",
        );
        assert!(
            blocks.iter().any(|b| b.contains(".tele{ transition:none;")),
            "the telescope must stop",
        );
    }
}
