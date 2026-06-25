//! `tonk guide` — the agent's reference, baked into the binary so a
//! sandbox without repo access can still learn the syntax. Served as
//! discrete topics instead of one dump: `tonk guide` prints a short
//! index, `tonk guide <topic>` prints one section, `tonk guide all`
//! prints everything.
//!
//! `tonk-notation/guide.md` is the canonical notation reference (the
//! worker parses the same syntax) and is shared, not forked. The other
//! topic files live beside this module. `include_str!` resolves at
//! compile time relative to `rust/tonk/src/guide.rs`.

/// The one-screen index printed by a bare `tonk guide`.
pub const INDEX: &str = include_str!("guide-index.md");

/// Asserted-notation syntax reference (`tonk guide notation`).
pub const NOTATION: &str = include_str!("../../tonk-notation/guide.md");

/// Display templates and view resolution (`tonk guide views`).
pub const VIEWS: &str = include_str!("guide-views.md");

/// Effects, rules, transients, DOM-event reactivity (`tonk guide events`).
pub const EVENTS: &str = include_str!("guide-events.md");

/// tonk-workspace authoring model (`tonk guide workspace`; app-layer).
pub const WORKSPACE: &str = include_str!("guide-workspace.md");

/// Valid topic names for `tonk guide <topic>`, in display order.
pub const TOPICS: &[&str] = &["notation", "views", "events", "workspace"];

/// The full guide: every topic concatenated. Backs `tonk guide all`.
/// `concat!` needs literal `include_str!` calls, so the paths are
/// repeated here rather than reusing the per-topic constants.
pub const GUIDE: &str = concat!(
    include_str!("../../tonk-notation/guide.md"),
    "\n",
    include_str!("guide-views.md"),
    "\n",
    include_str!("guide-events.md"),
    "\n",
    include_str!("guide-workspace.md"),
);

/// The body for a single named topic, or `None` if `name` isn't a
/// recognized topic.
pub fn topic(name: &str) -> Option<&'static str> {
    match name {
        "notation" => Some(NOTATION),
        "views" => Some(VIEWS),
        "events" => Some(EVENTS),
        "workspace" => Some(WORKSPACE),
        _ => None,
    }
}

/// Resolve a `tonk guide [TOPIC]` argument to the text to print.
///
/// - `None` → the [`INDEX`].
/// - `Some("all")` → the full [`GUIDE`].
/// - `Some(topic)` → that topic's body.
/// - `Some(unknown)` → `Err` naming the valid topics.
pub fn resolve(arg: Option<&str>) -> Result<&'static str, GuideError> {
    match arg {
        None => Ok(INDEX),
        Some("all") => Ok(GUIDE),
        Some(name) => topic(name).ok_or_else(|| GuideError::UnknownTopic(name.to_owned())),
    }
}

/// Why [`resolve`] couldn't map a topic argument to guide text.
#[derive(Debug)]
pub enum GuideError {
    /// The argument was neither `all` nor a known topic.
    UnknownTopic(String),
}

impl std::fmt::Display for GuideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTopic(name) => write!(
                f,
                "unknown guide topic '{name}'; valid topics: {}, all",
                TOPICS.join(", "),
            ),
        }
    }
}

impl std::error::Error for GuideError {}
