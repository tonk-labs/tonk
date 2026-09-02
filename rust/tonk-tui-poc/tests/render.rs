//! End-to-end snapshots: a template and some conclusions in, a cell
//! grid out.
//!
//! These go through the real `tonk-render` pipeline, so they also pin
//! the claim that a terminal vocabulary needs no changes to it.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    // `CARGO_BIN_EXE_<name>` is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_tonk-tui-poc"))
}

fn demo(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("demo")
        .join(name)
}

fn render(template: &str, data: &str, size: &str, extra: &[&str]) -> String {
    let output = Command::new(binary())
        .arg("--template")
        .arg(demo(template))
        .arg("--data")
        .arg(demo(data))
        .args(["--size", size, "--plain"])
        .args(extra)
        .output()
        .expect("running tonk-tui-poc");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 output")
}

#[test]
fn a_directory_view_renders_one_row_per_conclusion() {
    // The `{this}` repeat root clones per conclusion and the chrome —
    // header, box, keybar — renders once, exactly as in the browser.
    let frame = render("todo.tui.html", "todo.json", "56x12", &[]);
    let expected = "
  todo                                          4 open

  ┌──────────────────────────────────────────────────┐
  │ [ ] port the view pipeline                   ada │
  │ [x] measure text in cells                  grace │
  │ [ ] 日本語 のタイトル                      kenji │
  │ [ ] decide pad-x vs pad-y                    ada │
  └──────────────────────────────────────────────────┘

   ↵ open    n new    d done                   q quit

";
    assert_eq!(frame, expected);
}

#[test]
fn a_wide_glyph_row_ends_in_the_same_column_as_an_ascii_one() {
    // The regression this pins: emitting a wide grapheme's covered
    // cells as spaces makes the line visually wider than the cells it
    // was given, which pushes everything after it and reads as a
    // layout bug rather than a text bug.
    let frame = render("todo.tui.html", "todo.json", "56x12", &[]);
    let widths: Vec<usize> = frame
        .lines()
        .filter(|line| line.contains('│'))
        .map(tonk_width)
        .collect();
    assert!(!widths.is_empty());
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "row widths differ: {widths:?}"
    );
}

/// Display width of a rendered line, in cells.
fn tonk_width(line: &str) -> usize {
    usize::from(tonk_layout::text_width(line))
}

#[test]
fn a_paragraph_height_follows_from_the_width_it_is_given() {
    let narrow = render("card.tui.html", "card.json", "40x18", &[]);
    let wide = render("card.tui.html", "card.json", "76x18", &[]);
    let count = |frame: &str| {
        frame
            .lines()
            .filter(|line| line.contains("paragraph") || line.contains("measure function"))
            .count()
    };
    // Same text, different widths: the narrow frame must use more rows.
    assert!(
        narrow.lines().filter(|l| l.contains('│')).count()
            > wide.lines().filter(|l| l.contains('│')).count(),
        "narrow frame should wrap onto more rows"
    );
    assert!(count(&narrow) >= 1 && count(&wide) >= 1);
}

#[test]
fn alignment_pins_children_to_both_edges_and_the_middle() {
    let frame = render("card.tui.html", "card.json", "52x14", &[]);
    let row = frame
        .lines()
        .find(|line| line.contains("left") && line.contains("right"))
        .expect("the alignment row");
    assert!(row.trim_start().starts_with("left"));
    assert!(row.trim_end().ends_with("right"));
    let centre = row.find("centre").expect("centre");
    let left = row.find("left").expect("left");
    let right = row.find("right").expect("right");
    assert!(left < centre && centre < right, "document order preserved");
}

#[test]
fn the_frame_never_exceeds_the_viewport() {
    for size in ["20x6", "40x10", "56x12", "120x30"] {
        let frame = render("todo.tui.html", "todo.json", size, &[]);
        let (width, height) = size.split_once('x').expect("WxH");
        let width: usize = width.parse().expect("width");
        let height: usize = height.parse().expect("height");
        assert_eq!(frame.lines().count(), height, "row count at {size}");
        for line in frame.lines() {
            assert!(
                tonk_width(line) <= width,
                "line overflows at {size}: {line:?}"
            );
        }
    }
}

#[test]
fn rendering_is_deterministic() {
    // An immediate-mode renderer re-solves every frame; if an uneven
    // fill split resolved differently between frames the UI would
    // shimmer.
    let first = render("todo.tui.html", "todo.json", "57x12", &[]);
    for _ in 0..5 {
        assert_eq!(render("todo.tui.html", "todo.json", "57x12", &[]), first);
    }
}

#[test]
fn no_template_falls_back_to_highlighted_notation() {
    // What a terminal shows when no view resolves: the conclusion
    // formatted back into `head!:` source, the same ultimate fallback
    // the browser mounts — and, like the browser's, not a template.
    let output = Command::new(binary())
        .arg("--data")
        .arg(demo("todo.json"))
        .args(["--head", "todo", "--size", "60x9", "--plain"])
        .output()
        .expect("running tonk-tui-poc");
    assert!(output.status.success());
    let frame = String::from_utf8(output.stdout).expect("utf-8");
    assert!(frame.contains("todo!:"), "{frame}");
    assert!(frame.contains("this: id:1"), "{frame}");
    assert!(
        frame.contains("title: \"port the view pipeline\""),
        "{frame}"
    );
}

#[test]
fn notation_highlighting_degrades_to_emphasis_without_colour() {
    // The same `Decoration` mapping serves a full-colour terminal and
    // an ink-only one: under `--colour none` the tokens resolve to
    // nothing and only the SGR emphasis survives. This is the argument
    // for semantic tokens over hex literals, as a test.
    let run = |colour: &str| {
        let output = Command::new(binary())
            .arg("--data")
            .arg(demo("todo.json"))
            .args(["--head", "todo", "--size", "60x4", "--colour", colour])
            .output()
            .expect("running tonk-tui-poc");
        String::from_utf8(output.stdout).expect("utf-8")
    };
    let full = run("truecolor");
    let mono = run("none");
    assert!(full.contains("38;2;"), "truecolor emits rgb: {full:?}");
    assert!(!mono.contains("38;2;"), "mono emits no colour: {mono:?}");
    // `head!` is bold in both — emphasis is orthogonal to colour.
    assert!(full.contains("\u{1b}[0;1mtodo!"), "{full:?}");
    assert!(mono.contains("\u{1b}[0;1mtodo!"), "{mono:?}");
}

#[test]
fn show_output_is_an_ordinary_directory_view() {
    // `tonk show`'s output is already shaped like a directory view:
    // an envelope that renders once, then one notation block per
    // instance. The `{this}` repeat root is the block, and
    // `{dom.notation/source}` is a host-provided field like
    // `{dom.host/model}` — so this needs no new mechanism.
    let frame = render("show.tui.html", "show.json", "58x22", &["--head", "todo"]);
    assert_eq!(frame.matches("todo!:").count(), 2, "one block per row");
    assert_eq!(frame.matches("12 claims").count(), 1, "envelope is chrome");
    assert!(frame.contains("this: id:1") && frame.contains("this: id:2"));
    assert!(frame.contains("e eval"), "keybar is chrome too");
}
