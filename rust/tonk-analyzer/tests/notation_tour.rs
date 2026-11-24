//! Demo (notation tour): every ```notation cell of the tour runs.
//!
//! `rust/tonk-analyzer/tests/notation-tour/tour.md` is the notebook a "Notation tour" space
//! opens on. Each cell is run here in order, on one branch, the way the
//! notebook runs it (the cells above as its context), and a cell either
//! runs clean or, when it says `; expect: refused`, is refused. Set
//! `TONK_TOUR_PRINT=1` to print every answer.

#![cfg(not(target_arch = "wasm32"))]

use dialog_peer::helpers::{test_repo, test_session_with_peer};
use tonk_analyzer::notation::tour::{Cell, evaluate_on_branch};

const TOUR: &str = include_str!("notation-tour/tour.md");

/// The ```notation fences of a Markdown document, in order.
fn cells(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in markdown.lines() {
        match &mut current {
            None if line.trim_start().starts_with("```notation") => current = Some(Vec::new()),
            Some(lines) if line.trim_start().starts_with("```") => {
                out.push(lines.join("\n"));
                current = None;
            }
            Some(lines) => lines.push(line),
            None => {}
        }
    }
    out
}

#[dialog_common::test]
async fn every_cell_of_the_tour_runs() {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    let print = std::env::var("TONK_TOUR_PRINT").is_ok();
    let cells = cells(TOUR);
    assert!(cells.len() > 5, "the tour has cells");
    let mut failures = Vec::new();
    for (at, source) in cells.iter().enumerate() {
        let cell = Cell {
            source: source.clone(),
            context: cells[..at].to_vec(),
        };
        let started = std::time::Instant::now();
        let answer = evaluate_on_branch(&branch, &cell, &operator).await;
        let took = started.elapsed();
        let refused =
            answer.error.is_some() || answer.forms.iter().any(|form| form.refused.is_some());
        let expected = source.contains("; expect: refused");
        let json = serde_json::to_string_pretty(&answer).expect("json");
        if print {
            println!(
                "=== cell {} ({took:?})\n{source}\n--- answer\n{json}\n",
                at + 1
            );
        }
        if refused != expected {
            failures.push(format!(
                "cell {} {}:\n{source}\n{json}",
                at + 1,
                if expected {
                    "should be refused and ran"
                } else {
                    "was refused"
                }
            ));
        }
        if source.contains("; expect: warning")
            && !answer.forms.iter().any(|form| !form.warnings.is_empty())
        {
            failures.push(format!("cell {}: no warning\n{source}\n{json}", at + 1));
        }
        // A cell that asks something and runs clean answers something.
        for form in &answer.forms {
            if form.refused.is_none()
                && form.summary.starts_with("0 ")
                && !source.contains("; expect: empty")
            {
                failures.push(format!("cell {}: `{}` answered nothing", at + 1, form.form));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The seed a "Notation tour" space is created from:
/// `rust/tonk-core/assets/library/notation-tour.yaml`, which the app
/// serves at `/library/notation-tour.yaml`. `space/create` seeds a space
/// from one document and a seed cannot include another library document,
/// so it is written out: `notebook.yaml` as it is, then the tour notebook
/// as facts (its title, one block per Markdown block of `tour.md`, the
/// block sequence), then the space's home, which opens the notebook.
///
/// Fails when the checked-in seed is not what `tour.md` and
/// `notebook.yaml` make. Regenerate it with
/// `TONK_TOUR_WRITE_SEED=1 cargo test -p tonk-analyzer --test notation_tour`.
#[dialog_common::test]
fn the_tour_seed_is_current() {
    const NOTEBOOK: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");
    const SEED: &str = include_str!("../../tonk-core/assets/library/notation-tour.yaml");
    let seed = tour_seed(NOTEBOOK, TOUR);
    if std::env::var("TONK_TOUR_WRITE_SEED").is_ok() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tonk-core/assets/library/notation-tour.yaml"
        );
        std::fs::write(path, &seed).expect("write the seed");
        return;
    }
    assert!(
        seed == SEED,
        "notation-tour.yaml is stale: run TONK_TOUR_WRITE_SEED=1 cargo test -p tonk-analyzer --test notation_tour"
    );
}

/// The tour notebook's entity.
const TOUR_NOTEBOOK: &str = "notebook:notation-tour";

/// Blank-line separated chunks of Markdown, a fence kept whole.
fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.trim().is_empty() {
            if !current.is_empty() {
                out.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        current.push(line);
    }
    if !current.is_empty() {
        out.push(current.join("\n"));
    }
    out
}

/// The notebook's blocks, as the notebook splits a document: a heading
/// rides with what follows it.
fn blocks(text: &str) -> Vec<String> {
    let heading = |chunk: &String| chunk.starts_with('#');
    let mut out = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for chunk in chunks(text) {
        if heading(&chunk) {
            if pending.iter().any(|held| !heading(held)) {
                out.push(pending.join("\n\n"));
                pending.clear();
            }
            pending.push(chunk);
        } else if pending.iter().any(heading) {
            pending.push(chunk);
            out.push(pending.join("\n\n"));
            pending.clear();
        } else {
            out.push(chunk);
        }
    }
    if !pending.is_empty() {
        out.push(pending.join("\n\n"));
    }
    out
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("a string")
}

fn tour_seed(notebook: &str, tour: &str) -> String {
    let title = tour
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .expect("the tour has a title")
        .trim();
    let parts = blocks(tour);
    let entries = (1..=parts.len())
        .map(|at| format!("N{at:05}V: id:notation-tour/block-{at}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut doc = vec![
        "# GENERATED from notebook.yaml and rust/tonk-analyzer/tests/notation-tour/tour.md by".to_owned(),
        "# rust/tonk-analyzer/tests/notation_tour.rs. Do not edit; edit those and run".to_owned(),
        "# TONK_TOUR_WRITE_SEED=1 cargo test -p tonk-analyzer --test notation_tour".to_owned(),
        "#".to_owned(),
        "# The notation tour's seed: the notebook library, then the tour".to_owned(),
        "# notebook as facts, then the space's home, which opens it.".to_owned(),
        String::new(),
        notebook.trim_end().to_owned(),
        String::new(),
        "# ---------------------------------------------------------------------------".to_owned(),
        "# The tour notebook".to_owned(),
        "# ---------------------------------------------------------------------------".to_owned(),
        String::new(),
        "notebook/named!:".to_owned(),
        format!("  this: {TOUR_NOTEBOOK}"),
        format!("  title: {}", quote(title)),
        String::new(),
        "notebook/blocks!:".to_owned(),
        format!("  this: {TOUR_NOTEBOOK}"),
        format!("  block: {{{entries}}}"),
        String::new(),
    ];
    for (at, source) in parts.iter().enumerate() {
        doc.extend([
            "notebook/block!:".to_owned(),
            format!("  this: id:notation-tour/block-{}", at + 1),
            format!("  notebook: {TOUR_NOTEBOOK}"),
            format!("  source: {}", quote(source)),
            String::new(),
        ]);
    }
    doc.extend(
        [
            "# The space's home: the tour notebook.",
            "concept!: &notation-tour/home",
            "  this: space:home",
            "  description: The notation tour's home page.",
            "  with:",
            "    subject:",
            "      description: The repository's subject DID.",
            "      the: dialog.replica/subject",
            "      as: entity",
            "",
            "view!:",
            "  this: space:home",
            "  show:",
            "    ui: |",
        ]
        .map(str::to_owned),
    );
    doc.push(format!(
        "      <tonk-display entity={TOUR_NOTEBOOK} model=notebook></tonk-display>"
    ));
    doc.extend(
        [
            "",
            "name!:",
            "  this: id:tonk/space",
            "  entity: space:home",
            "",
        ]
        .map(str::to_owned),
    );
    doc.join("\n")
}
