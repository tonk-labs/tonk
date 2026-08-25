//! The shared FABB skin — chrome tokens and the frost recipe.
//!
//! One material: frost, in a light and a dark twin. Every component's shadow
//! root opens with this block, so the whole family shares one set of tokens
//! and one easing.
//!
//! ## Why the internal tokens are `--_`-prefixed
//!
//! Custom properties are the one channel that crosses a shadow boundary, and
//! it cuts both ways: a host page's own `--ink` arrives uninvited. The
//! reference implementation records this happening — a document `--ink:
//! #131313` painted primary buttons black with invisible labels. So the
//! public API is the `--fabb-*` set, read once here into `--_*` mirrors, and
//! nothing else is allowed across.
//!
//! ## Scope
//!
//! Every component wraps its content in `.w` and the tokens are declared on
//! that wrapper, not `:host` — `.w.dark` is then a plain class toggle rather
//! than a media query, which is what lets `mode="light|dark"` override the
//! system preference per element.

/// The token block plus the primitives every component shares: the sync disc,
/// the terminal block cursor, the blink keyframes, and the button reset.
///
/// Concatenated into each shadow root's `<style>` ahead of that component's
/// own rules.
pub const SKIN: &str = r#"
*, *::before, *::after{ box-sizing:border-box; }
.w{
  --_ink:   var(--fabb-ink, #131313);
  --_soft:  var(--fabb-ink-soft, #55544f);
  --_on:    var(--fabb-on-ink, #fbfaef);
  --_sep:   var(--fabb-sep, rgba(19,19,19,.28));
  --_hover: var(--fabb-hover, rgba(19,19,19,.06));
  --_press: var(--fabb-press, rgba(19,19,19,.12));
  --_bg:    var(--fabb-bg, rgba(255,255,255,.72));
  --_panel: var(--fabb-panel, rgba(255,255,255,.92));
  --_filter:blur(12px) saturate(1.5);
  --_ring:  0 0 0 1px var(--fabb-ring, rgba(19,19,19,.85));
  --_ringc: var(--fabb-ring, rgba(19,19,19,.85));
  --_ease:  cubic-bezier(0.25,0.46,0.45,0.94);
  --_blink: 2.4s;
  font-family:'IBM Plex Sans Condensed','Bahnschrift','Arial Narrow',system-ui,sans-serif;
  font-weight:600; letter-spacing:.02em;
  -webkit-tap-highlight-color:transparent;
  /* the OS's edges, readable from Rust via computed style */
  --_sat:env(safe-area-inset-top, 0px); --_sar:env(safe-area-inset-right, 0px);
  --_sab:env(safe-area-inset-bottom, 0px); --_sal:env(safe-area-inset-left, 0px);
}
/* selection is chrome too — ink only, never the browser's blue */
.w ::selection{ background:var(--_ink); color:var(--_on); }
.w.dark{
  --_ink:var(--fabb-ink-dark, #e9e6d6);
  --_soft:var(--fabb-ink-soft-dark, #cdcaba);
  --_on:var(--fabb-on-ink-dark, #22221c);
  --_sep:var(--fabb-sep-dark, rgba(233,230,214,.28));
  --_hover:var(--fabb-hover-dark, rgba(251,250,239,.09));
  --_press:var(--fabb-press-dark, rgba(251,250,239,.15));
  --_bg:var(--fabb-bg-dark, rgba(32,32,26,.78));
  --_panel:var(--fabb-panel-dark, rgba(32,32,26,.88));
  --_filter:blur(16px) saturate(1.5);
  --_ring:0 0 0 1px var(--fabb-ring-dark, rgba(233,230,214,.55));
  --_ringc:var(--fabb-ring-dark, rgba(233,230,214,.55));
}
/* the calm blink — alerts pulse, they never take a color */
@keyframes fabb-blink{ 0%,100%{opacity:1} 50%{opacity:.55} }
@keyframes fabb-wash{ 0%,100%{background:transparent} 50%{background:color-mix(in srgb, var(--_ink) 14%, transparent)} }
@keyframes fabb-hardblink{ 0%,49%{opacity:1} 50%,100%{opacity:0} }
button{ font:inherit; letter-spacing:inherit; color:inherit; background:none; border:0; padding:0; cursor:pointer; text-align:inherit; }
:is(button,[tabindex]):focus-visible{ outline:2px solid var(--_ink); outline-offset:-2px; }
.disc{ width:14px; height:14px; border-radius:50%; background:var(--_ink); flex:none; }
.disc.offline{ background:transparent; border:2px solid var(--_ink); }
.disc.paused{ background:linear-gradient(135deg,var(--_ink) 0 50%,transparent 50% 100%); border:1.5px solid var(--_ink); }
.disc.alert{ animation:fabb-blink var(--_blink) var(--_ease) infinite; }
/* the terminal block cursor — shared by bar cells and fields */
.cur{ display:inline-block; width:7px; height:13px; background:var(--_ink); flex:none;
  margin-left:-7px; mix-blend-mode:difference;
  animation:fabb-hardblink 1.05s steps(1,end) infinite; }
.dark .cur, .w.dark .cur{ mix-blend-mode:exclusion; }
.edit{ outline:none !important; caret-color:transparent; min-width:1ch; text-transform:none; user-select:text; }
/* a hidden tab spends nothing — every animation holds its frame */
.w.vispause *, .w.vispause *::before, .w.vispause *::after{ animation-play-state:paused !important; }
@media (prefers-reduced-motion: reduce){ .disc.alert{ animation:none !important; } .cur{ animation:none; } }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_declares_both_mode_twins() {
        // The dark twin must re-state the whole set: a token defined only in
        // the light block leaks the light value into dark mode.
        for token in [
            "--_ink",
            "--_soft",
            "--_on",
            "--_sep",
            "--_hover",
            "--_press",
            "--_bg",
            "--_panel",
            "--_filter",
            "--_ring",
            "--_ringc",
        ] {
            let dark_block = SKIN.split(".w.dark{").nth(1).expect("dark twin present");
            let dark_block = dark_block.split('}').next().expect("dark twin closes");
            assert!(
                dark_block.contains(token),
                "the dark twin must re-state {token}, or the light value leaks into dark mode",
            );
        }
    }

    #[test]
    fn it_never_reads_an_unprefixed_custom_property() {
        // The public API is `--fabb-*`; internals are `--_*`. Anything else
        // crossing the boundary is a host page's variable arriving uninvited.
        for read in SKIN.match_indices("var(--") {
            let tail = &SKIN[read.0 + 4..];
            let name = tail
                .split([',', ')'])
                .next()
                .expect("a var() names something");
            assert!(
                name.starts_with("--_") || name.starts_with("--fabb-"),
                "{name} is neither an internal --_ token nor part of the --fabb-* API",
            );
        }
    }

    #[test]
    fn it_holds_every_animation_in_a_hidden_tab() {
        assert!(SKIN.contains(".w.vispause *"));
        assert!(SKIN.contains("animation-play-state:paused !important"));
    }

    #[test]
    fn it_respects_reduced_motion() {
        let reduced = SKIN
            .split("@media (prefers-reduced-motion: reduce)")
            .nth(1)
            .expect("a reduced-motion block");
        assert!(reduced.contains(".disc.alert"));
        assert!(reduced.contains(".cur"));
    }
}
