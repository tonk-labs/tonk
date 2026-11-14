//! Built-in manuals served by `tonk help <guide>`.

/// Shared agent context used by MCP, desktop and browser hosts.
pub const AGENT: &str = include_str!("guide-agent.md");
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
/// Routes: pages at paths.
pub const ROUTES: &str = include_str!("guide-routes.md");
/// Events, effects, and rules guide.
pub const EVENTS: &str = include_str!("guide-events.md");
/// `<tonk-display>` reference.
pub const ELEMENT_TONK_DISPLAY: &str = include_str!("guide-element-tonk-display.md");
/// `<prose-editor>` reference.
pub const ELEMENT_PROSE_EDITOR: &str = include_str!("guide-element-prose-editor.md");
/// `<code-editor>` reference.
pub const ELEMENT_CODE_EDITOR: &str = include_str!("guide-element-code-editor.md");
/// `<table-editor>` reference.
pub const ELEMENT_TABLE_EDITOR: &str = include_str!("guide-element-table-editor.md");

/// Every guide name, in the order `tonk help -g` presents it.
pub const TOPICS: &[&str] = &[
    "glossary",
    "agent",
    "notation",
    "spaces",
    "tutorial",
    "sync",
    "views",
    "routes",
    "events",
    "tonk-display",
    "prose-editor",
    "code-editor",
    "table-editor",
];

/// One-line catalogue description for a guide.
pub fn description(name: &str) -> Option<&'static str> {
    match name {
        "glossary" => Some("The words Tonk uses for facts, concepts, and writes"),
        "agent" => Some("Shared Tonk tools, authoring context and verification"),
        "notation" => Some("Notation syntax, queries, assertions, names, and joins"),
        "spaces" => Some("Space selection, directory bindings, and resolution order"),
        "tutorial" => Some("The discover, define, write, view, and share loop"),
        "sync" => Some("Upstreams, remotes, automatic sync, and scoped connections"),
        "views" => Some("Templates, view resolution, and web components"),
        "routes" => Some("Pages at paths: route patterns and route models"),
        "events" => Some("Effects, rules, transient concepts, and DOM events"),
        "tonk-display" => Some("Render an entity through a view"),
        "prose-editor" => Some("Markdown editor element (the prose-editor component)"),
        "code-editor" => Some("Code editor element (the code-editor component)"),
        "table-editor" => Some("Spreadsheet element (the table component)"),
        _ => None,
    }
}

/// Resolve a guide name to its embedded body.
pub fn topic(name: &str) -> Option<&'static str> {
    match name {
        "glossary" => Some(GLOSSARY),
        "agent" => Some(AGENT),
        "notation" => Some(NOTATION),
        "spaces" => Some(SPACES),
        "tutorial" => Some(TUTORIAL),
        "sync" => Some(SYNC),
        "views" => Some(VIEWS),
        "routes" => Some(ROUTES),
        "events" => Some(EVENTS),
        "tonk-display" => Some(ELEMENT_TONK_DISPLAY),
        "prose-editor" => Some(ELEMENT_PROSE_EDITOR),
        "code-editor" => Some(ELEMENT_CODE_EDITOR),
        "table-editor" => Some(ELEMENT_TABLE_EDITOR),
        _ => None,
    }
}

/// Every guide concatenated for agent harnesses that prime with `tonk help all`.
pub const GUIDE: &str = concat!(
    include_str!("guide-glossary.md"),
    "\n",
    include_str!("guide-agent.md"),
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
    include_str!("guide-routes.md"),
    "\n",
    include_str!("guide-events.md"),
    "\n",
    include_str!("guide-element-tonk-display.md"),
    "\n",
    include_str!("guide-element-prose-editor.md"),
    "\n",
    include_str!("guide-element-code-editor.md"),
    "\n",
    include_str!("guide-element-table-editor.md"),
);
