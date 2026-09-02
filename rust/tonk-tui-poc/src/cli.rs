//! Argument parsing and the one-frame render, kept apart from `main` so
//! the tests can drive the same entry point.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::theme::{Capability, Theme};
use crate::{paint, pipeline, vocabulary};

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
}

/// Parse `std::env::args`, render one frame, and return it.
pub fn run() -> Result<String, String> {
    let options = parse(std::env::args().skip(1))?;
    render(&options)
}

/// Render one frame under `options`.
pub fn render(options: &Options) -> Result<String, String> {
    let conclusions = match &options.data {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            pipeline::conclusions_from_json(&json)?
        }
        None => Vec::new(),
    };

    let root = match &options.template {
        Some(path) => {
            let template = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            vocabulary::lower(&pipeline::resolve(&template, &conclusions, &options.head))
        }
        // No view resolved: dump the conclusions as highlighted
        // notation, the same ultimate fallback the browser mounts.
        None => crate::notation::dump(&conclusions, &options.head),
    };
    let viewport = tonk_layout::Rect::new(0, 0, options.width, options.height);
    let laid = tonk_layout::layout(&root, viewport);

    if options.tree {
        let mut out = String::new();
        write_tree(&mut out, &laid, 0);
        return Ok(out);
    }

    let theme = Theme::new(options.capability);
    Ok(paint::frame(
        &laid,
        viewport,
        &theme,
        options.explain,
        options.plain,
    ))
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
            }
            "--colour" | "--color" => {
                options.capability = Capability::parse(&value("--colour")?)?;
            }
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
  --size WxH          viewport in cells (default 80x24)
  --colour <level>    truecolor | 256 | ansi | none (default truecolor)
  --explain           outline every element, elm-ui style
  --plain             emit no styling, for snapshots
  --tree              print the resolved layout tree instead of painting";
