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

// Full per-element docs, served by `tonk guide views <element>`. The
// `views` topic carries the short catalog; these are the deep dives.

/// `<tonk-display>` full doc (`tonk guide views tonk-display`).
pub const ELEMENT_TONK_DISPLAY: &str = include_str!("guide-element-tonk-display.md");
/// `<tonk-prose>` full doc (`tonk guide views tonk-prose`).
pub const ELEMENT_TONK_PROSE: &str = include_str!("guide-element-tonk-prose.md");
/// `<tonk-code>` full doc (`tonk guide views tonk-code`).
pub const ELEMENT_TONK_CODE: &str = include_str!("guide-element-tonk-code.md");
/// `<tonk-table>` full doc (`tonk guide views tonk-table`).
pub const ELEMENT_TONK_TABLE: &str = include_str!("guide-element-tonk-table.md");

/// Built-in view elements documented under `tonk guide views <name>`,
/// in catalog order. Keep in sync with the table in `guide-views.md`.
pub const ELEMENTS: &[&str] = &["tonk-display", "tonk-prose", "tonk-code", "tonk-table"];

/// The doc body for a built-in view element, or `None` if `name` isn't
/// a documented element.
pub fn element(name: &str) -> Option<&'static str> {
    match name {
        "tonk-display" => Some(ELEMENT_TONK_DISPLAY),
        "tonk-prose" => Some(ELEMENT_TONK_PROSE),
        "tonk-code" => Some(ELEMENT_TONK_CODE),
        "tonk-table" => Some(ELEMENT_TONK_TABLE),
        _ => None,
    }
}

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

/// Resolve a `tonk guide [TOPIC] [ITEM]` invocation to the text to
/// print.
///
/// - `(None, _)` → the [`INDEX`].
/// - `(Some("all"), _)` → the full [`GUIDE`].
/// - `(Some("views"), Some(element))` → that element's full doc, or an
///   `Err` naming the documented elements.
/// - `(Some(topic), None)` → that topic's body.
/// - `(Some(topic), Some(_))` where the topic takes no item → `Err`.
/// - `(Some(unknown), _)` → `Err` naming the valid topics.
pub fn resolve(topic_arg: Option<&str>, item: Option<&str>) -> Result<&'static str, GuideError> {
    match (topic_arg, item) {
        (None, _) => Ok(INDEX),
        (Some("all"), _) => Ok(GUIDE),
        // `views <element>` drills into one built-in element's full doc.
        (Some("views"), Some(name)) => {
            element(name).ok_or_else(|| GuideError::UnknownElement(name.to_owned()))
        }
        (Some(name), None) => topic(name).ok_or_else(|| GuideError::UnknownTopic(name.to_owned())),
        // A second arg is only meaningful after `views`.
        (Some(name), Some(_)) => {
            // Surface it as an unknown-topic-shaped error if the topic
            // itself is bogus; otherwise say the topic takes no item.
            if topic(name).is_some() {
                Err(GuideError::NoItems(name.to_owned()))
            } else {
                Err(GuideError::UnknownTopic(name.to_owned()))
            }
        }
    }
}

/// Why [`resolve`] couldn't map an invocation to guide text.
#[derive(Debug)]
pub enum GuideError {
    /// The topic argument was neither `all` nor a known topic.
    UnknownTopic(String),
    /// `views <name>` named an element with no documented page.
    UnknownElement(String),
    /// A second argument was given to a topic that takes none.
    NoItems(String),
}

impl std::fmt::Display for GuideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTopic(name) => write!(
                f,
                "unknown guide topic '{name}'; valid topics: {}, all",
                TOPICS.join(", "),
            ),
            Self::UnknownElement(name) => write!(
                f,
                "unknown element '{name}'; documented elements: {}",
                ELEMENTS.join(", "),
            ),
            Self::NoItems(name) => write!(
                f,
                "`tonk guide {name}` takes no second argument \
                 (only `views <element>` does)",
            ),
        }
    }
}

impl std::error::Error for GuideError {}
