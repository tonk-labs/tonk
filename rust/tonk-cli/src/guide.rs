//! Built-in manuals served by `tonk help <guide>`.

/// Tonk vocabulary and mental model.
pub const GLOSSARY: &str = include_str!("guide-glossary.md");
/// Asserted-notation grammar reference.
pub const NOTATION: &str = include_str!("../../tonk-notation/guide.md");
/// Space selection and binding guide.
pub const SPACES: &str = include_str!("guide-spaces.md");
/// Empty-space workflow tutorial.
pub const TUTORIAL: &str = include_str!("guide-tutorial.md");
/// Remotes and synchronization guide.
pub const SYNC: &str = include_str!("guide-sync.md");
/// Views and display templates guide.
pub const VIEWS: &str = include_str!("guide-views.md");
/// Events, effects, and rules guide.
pub const EVENTS: &str = include_str!("guide-events.md");
/// Workspace authoring guide.
pub const WORKSPACE: &str = include_str!("guide-workspace.md");
/// `<tonk-display>` reference.
pub const ELEMENT_TONK_DISPLAY: &str = include_str!("guide-element-tonk-display.md");
/// `<tonk-prose>` reference.
pub const ELEMENT_TONK_PROSE: &str = include_str!("guide-element-tonk-prose.md");
/// `<tonk-code>` reference.
pub const ELEMENT_TONK_CODE: &str = include_str!("guide-element-tonk-code.md");
/// `<tonk-table>` reference.
pub const ELEMENT_TONK_TABLE: &str = include_str!("guide-element-tonk-table.md");

/// Every guide name, in the order `tonk help -g` presents it.
pub const TOPICS: &[&str] = &[
    "glossary",
    "notation",
    "spaces",
    "tutorial",
    "sync",
    "views",
    "events",
    "workspace",
    "tonk-display",
    "tonk-prose",
    "tonk-code",
    "tonk-table",
];

/// One-line catalogue description for a guide.
pub fn description(name: &str) -> Option<&'static str> {
    match name {
        "glossary" => Some("The words Tonk uses for facts, concepts, and writes"),
        "notation" => Some("Notation syntax, queries, assertions, names, and joins"),
        "spaces" => Some("Space selection, directory bindings, and resolution order"),
        "tutorial" => Some("The discover, define, write, view, and share loop"),
        "sync" => Some("Upstreams, remotes, automatic sync, invites, and joins"),
        "views" => Some("Templates, view resolution, and web components"),
        "events" => Some("Effects, rules, transient concepts, and DOM events"),
        "workspace" => Some("Building sheets for the tonk-ui workspace shell"),
        "tonk-display" => Some("Render an entity through a view"),
        "tonk-prose" => Some("Markdown editor element"),
        "tonk-code" => Some("Code editor element"),
        "tonk-table" => Some("Spreadsheet element"),
        _ => None,
    }
}

/// Resolve a guide name to its embedded body.
pub fn topic(name: &str) -> Option<&'static str> {
    match name {
        "glossary" => Some(GLOSSARY),
        "notation" => Some(NOTATION),
        "spaces" => Some(SPACES),
        "tutorial" => Some(TUTORIAL),
        "sync" => Some(SYNC),
        "views" => Some(VIEWS),
        "events" => Some(EVENTS),
        "workspace" => Some(WORKSPACE),
        "tonk-display" => Some(ELEMENT_TONK_DISPLAY),
        "tonk-prose" => Some(ELEMENT_TONK_PROSE),
        "tonk-code" => Some(ELEMENT_TONK_CODE),
        "tonk-table" => Some(ELEMENT_TONK_TABLE),
        _ => None,
    }
}

/// Every guide concatenated for agent harnesses that prime with `tonk help all`.
pub const GUIDE: &str = concat!(
    include_str!("guide-glossary.md"),
    "\n",
    include_str!("../../tonk-notation/guide.md"),
    "\n",
    include_str!("guide-spaces.md"),
    "\n",
    include_str!("guide-tutorial.md"),
    "\n",
    include_str!("guide-sync.md"),
    "\n",
    include_str!("guide-views.md"),
    "\n",
    include_str!("guide-events.md"),
    "\n",
    include_str!("guide-workspace.md"),
    "\n",
    include_str!("guide-element-tonk-display.md"),
    "\n",
    include_str!("guide-element-tonk-prose.md"),
    "\n",
    include_str!("guide-element-tonk-code.md"),
    "\n",
    include_str!("guide-element-tonk-table.md"),
);
