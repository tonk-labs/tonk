//! `slide guide` — the agent's one-shot reference, baked into
//! the binary so a sandbox without repo access can still
//! discover the syntax by running one command. Three sections
//! glued together at compile time:
//!
//! 1. `tonk-notation/guide.md` — the canonical notation
//!    reference (parsed by the worker too; same syntax slide
//!    consumes).
//! 2. `slide/src/guide-views.md` — slide-specific addendum
//!    covering the view convention that `slide share view`
//!    relies on. Kept out of the upstream notation guide so the
//!    notation reference stays a clean syntax doc.
//! 3. `slide/src/guide-views-dynamic.md` — companion to the views
//!    addendum that documents the declarative reactivity surface:
//!    `on<event>=<concept>` template attributes, the `dom.event`
//!    namespace transient concepts read from, and `rule!:` heads
//!    that turn those transients into downstream state.
//! 4. `tonk-concept/SPEC.md` — the `<tonk-concept>` custom
//!    element reference: source-attribute grammar, template
//!    detection, `{field}` substitution. Relevant whenever an
//!    agent authors an HTML view body that embeds a live
//!    concept; the slide-side write/share machinery doesn't
//!    care about it, but the agent does.
//!
//! `include_str!` resolves at compile time relative to the
//! source file. Paths follow the workspace layout from
//! `rust/slide/src/guide.rs`. `concat!` is happy to glue
//! `include_str!` outputs because both expand to string
//! literals at compile time.

/// The full bundled guide.
pub const GUIDE: &str = concat!(
    include_str!("../../tonk-notation/guide.md"),
    "\n",
    include_str!("guide-views.md"),
    "\n",
    include_str!("guide-views-dynamic.md"),
    "\n",
    include_str!("../../tonk-concept/SPEC.md"),
);
