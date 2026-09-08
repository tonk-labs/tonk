//! Argument parsing and the one-frame render, kept apart from `main` so
//! the tests can drive the same entry point.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::session::{Direction, Effect, Key, Session};
use crate::theme::{Capability, Theme};
use crate::{paint, pipeline, vocabulary, wiring};

/// What the binary was asked to do.
pub struct Options {
    /// The `tui` facet template. Absent means "no view resolved", so
    /// the notation fallback renders instead.
    pub template: Option<PathBuf>,
    /// Conclusions as JSON: `[{"this": "...", "fields": {...}}, ...]`.
    pub data: Option<PathBuf>,
    /// Viewport size in cells.
    pub width: u16,
    /// Viewport size in cells.
    pub height: u16,
    /// Draw a debug outline around every element — elm-ui's `explain`.
    pub explain: bool,
    /// Emit no styling at all, so output diffs cleanly in a test.
    pub plain: bool,
    /// Colour capability to render for.
    pub capability: Capability,
    /// Print the resolved layout tree instead of painting it.
    pub tree: bool,
    /// Concept name to use as the head of a fallback assertion.
    pub head: String,
    /// The view's binding tables, which is what makes the frame
    /// interactive: without them nothing is focusable, because
    /// focusability is decided from the declarations (§5.1).
    pub bindings: Option<PathBuf>,
    /// A scripted keypress sequence, applied before the frame is
    /// painted. The interaction model with the terminal taken out, so a
    /// snapshot test can assert what a sequence of keys does.
    pub keys: Vec<Key>,
    /// Take over the terminal and loop, instead of printing one frame.
    pub interactive: bool,
    /// Whether `--size` was given, so an interactive run can default to
    /// the real terminal instead of the headless 80x24.
    pub sized: bool,
}

/// Parse `std::env::args` and do what was asked: loop over the terminal,
/// or render one frame and return it.
pub fn run() -> Result<String, String> {
    let mut options = parse(std::env::args().skip(1))?;
    if !options.interactive {
        return render(&options);
    }
    if !options.sized {
        let (width, height) = crossterm::terminal::size().map_err(|error| error.to_string())?;
        options.width = width;
        options.height = height;
    }
    crate::terminal::run(&options)
}

/// Everything one frame needs, resolved once.
///
/// Separated from painting so an interactive loop can hold it across
/// keypresses: the tree, the plan and the binding tables do not change
/// when focus moves, and re-reading them per keystroke would make the
/// loop's cost the pipeline's cost.
pub struct Prepared<'a> {
    options: &'a Options,
    /// The interaction model, when a template resolved.
    pub session: Option<Session>,
    /// The notation dump, when none did — the same ultimate fallback the
    /// browser mounts.
    fallback: Option<tonk_layout::Element>,
}

impl Prepared<'_> {
    /// Apply a keypress.
    pub fn press(&mut self, key: Key) -> Effect {
        match &mut self.session {
            Some(session) => session.press(key),
            None => Effect::Idle,
        }
    }

    /// Paint the current state.
    pub fn frame(&self) -> String {
        let root = match (&self.session, &self.fallback) {
            (Some(session), _) => vocabulary::lower(&session.frame_tree()),
            (None, Some(fallback)) => fallback.clone(),
            (None, None) => tonk_layout::Element::new(tonk_layout::Kind::El),
        };
        let viewport = tonk_layout::Rect::new(0, 0, self.options.width, self.options.height);
        let laid = tonk_layout::layout(&root, viewport);

        if self.options.tree {
            let mut out = String::new();
            write_tree(&mut out, &laid, 0);
            return out;
        }
        let theme = Theme::new(self.options.capability);
        paint::frame(
            &laid,
            viewport,
            &theme,
            self.options.explain,
            self.options.plain,
        )
    }
}

/// Resolve `options` into a paintable, pressable state.
pub fn prepare(options: &Options) -> Result<Prepared<'_>, String> {
    let conclusions = match &options.data {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            pipeline::conclusions_from_json(&json)?
        }
        None => Vec::new(),
    };

    let tables = match &options.bindings {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            wiring::tables(&json)?
        }
        None => wiring::Tables::default(),
    };

    match &options.template {
        Some(path) => {
            let template = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            let tree = pipeline::resolve(&template, &conclusions, &options.head);
            Ok(Prepared {
                options,
                session: Some(Session::new(
                    tree,
                    conclusions,
                    tables.events,
                    tables.commands,
                )),
                fallback: None,
            })
        }
        None => Ok(Prepared {
            options,
            session: None,
            fallback: Some(crate::notation::dump(&conclusions, &options.head)),
        }),
    }
}

/// Render one frame under `options`, after applying any scripted keys.
pub fn render(options: &Options) -> Result<String, String> {
    let mut prepared = prepare(options)?;
    let mut posted: Vec<String> = Vec::new();
    for key in &options.keys {
        match prepared.press(*key) {
            Effect::Post(body) => {
                posted.push(serde_json::to_string(&body).map_err(|error| error.to_string())?)
            }
            Effect::Declined => posted.push("declined".to_string()),
            Effect::Idle | Effect::Moved | Effect::Quit => {}
        }
    }

    let mut out = prepared.frame();
    // What the keys posted, after the frame rather than instead of it:
    // the point of the scripted driver is to see both the transient and
    // the view it came from.
    for body in posted {
        out.push_str(&body);
        out.push('\n');
    }
    Ok(out)
}

/// Read a scripted keypress sequence: whitespace-separated names, or a
/// bare character for itself.
fn parse_keys(raw: &str) -> Result<Vec<Key>, String> {
    raw.split_whitespace()
        .map(|token| match token {
            "tab" => Ok(Key::Tab),
            "backtab" | "shift-tab" => Ok(Key::BackTab),
            "up" => Ok(Key::Arrow(Direction::Up)),
            "down" => Ok(Key::Arrow(Direction::Down)),
            "left" => Ok(Key::Arrow(Direction::Left)),
            "right" => Ok(Key::Arrow(Direction::Right)),
            "enter" | "space" | "activate" => Ok(Key::Activate),
            "quit" => Ok(Key::Quit),
            other => {
                let mut characters = other.chars();
                match (characters.next(), characters.next()) {
                    (Some(character), None) => Ok(Key::Char(character)),
                    _ => Err(format!("unknown key {other:?}")),
                }
            }
        })
        .collect()
}

fn write_tree(out: &mut String, laid: &tonk_layout::Laid, depth: usize) {
    let rect = laid.rect;
    let label = match &laid.kind {
        tonk_layout::Kind::Text(text) => format!("text {text:?}"),
        tonk_layout::Kind::Paragraph(_) => format!("paragraph {:?}", laid.lines),
        other => format!("{other:?}").to_lowercase(),
    };
    let _ = writeln!(
        out,
        "{:indent$}{label} @ {},{} {}x{}",
        "",
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        indent = depth * 2,
    );
    for child in &laid.children {
        write_tree(out, child, depth + 1);
    }
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        template: None,
        data: None,
        width: 80,
        height: 24,
        explain: false,
        plain: false,
        capability: Capability::TrueColor,
        tree: false,
        head: "concept".to_string(),
        bindings: None,
        keys: Vec::new(),
        interactive: false,
        sized: false,
    };
    let mut args = args.peekable();
    let mut saw_template = false;
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--template" => {
                options.template = Some(PathBuf::from(value("--template")?));
                saw_template = true;
            }
            "--head" => options.head = value("--head")?,
            "--data" => options.data = Some(PathBuf::from(value("--data")?)),
            "--bindings" => options.bindings = Some(PathBuf::from(value("--bindings")?)),
            "--keys" => options.keys = parse_keys(&value("--keys")?)?,
            "--size" => {
                let raw = value("--size")?;
                let (width, height) = raw
                    .split_once(['x', 'X'])
                    .ok_or_else(|| format!("--size wants WxH, got {raw:?}"))?;
                options.width = width
                    .parse()
                    .map_err(|_| format!("bad width in --size {raw:?}"))?;
                options.height = height
                    .parse()
                    .map_err(|_| format!("bad height in --size {raw:?}"))?;
                options.sized = true;
            }
            "--colour" | "--color" => {
                options.capability = Capability::parse(&value("--colour")?)?;
            }
            "--interactive" | "-i" => options.interactive = true,
            "--explain" => options.explain = true,
            "--plain" => options.plain = true,
            "--tree" => options.tree = true,
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if !saw_template && !other.starts_with('-') => {
                options.template = Some(PathBuf::from(other));
                saw_template = true;
            }
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    if !saw_template && options.data.is_none() {
        return Err(format!("nothing to render\n\n{USAGE}"));
    }
    Ok(options)
}

const USAGE: &str = "\
usage: tonk-tui-poc [--template <file>] --data <file.json> [options]

  --template <file>   the `tui` facet template to render; omit it to
                      get the notation fallback, as when no view
                      resolves for a model
  --head <name>       concept name for fallback assertion heads
  --data <file.json>  conclusions: [{\"this\": \"...\", \"fields\": {...}}]
  --bindings <file>   the view's binding tables:
                      {\"events\": {...}, \"commands\": {...}}
  --keys \"tab enter\"  drive the interaction model, then paint; posted
                      transients print after the frame
  --size WxH          viewport in cells (default 80x24)
  --colour <level>    truecolor | 256 | ansi | none (default truecolor)
  --interactive, -i   take over the terminal and loop: Tab / Shift-Tab
                      to traverse, arrows inside a `nav=` container,
                      Enter or Space to activate, Esc to leave
  --explain           outline every element, elm-ui style
  --plain             emit no styling, for snapshots
  --tree              print the resolved layout tree instead of painting";
