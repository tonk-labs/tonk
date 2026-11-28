//! Demo (notation tour): one notebook cell written in the notation,
//! evaluated against a space's branch, with each stage of the pipeline
//! kept to show.
//!
//! The cell runs through the notation's own driver
//! (`dialog_notation::apply`): `stage` writes what it asserts and the
//! rules it declares into the caller's transaction, each `when` reading
//! what the forms before it wrote, the caller commits once, and `answer`
//! asks its queries after. What a notebook adds is the context:
//! the analyzer keeps definitions in memory only, so the notation cells
//! above this one are read into a fresh analyzer first, for their
//! definitions (nothing of theirs is written again).
//!
//! The answer says, for each top-level form, what it did, and for the
//! cell as a whole what each stage of the pipeline gave:
//!
//! - `read`: the syntax trees the text parses to, printed, and each tree
//!   as data (`Syntax::value`, the record the notation keeps a tree as);
//! - `expanded`: the analyzed trees, every one a rule of the kernel;
//! - `analysis`: what the cell declares, and every diagnostic with its
//!   place;
//! - `compiled`: the steps, with Dialog-DB's own descriptors as JSON.
//!
//! Everything here is demo scaffolding for the notation tour; it is
//! reached from the worker's `notation_tour` module only.

use std::collections::BTreeMap;

pub use dialog_notation::apply::Stage as Staged;
use dialog_notation::apply::{self, Did, DocumentReport, Installation, Writer};
use dialog_notation::live::{Live, Reader};
use dialog_notation::{
    Analyzer, Checked, Declaration, Rendered, Severity, Step, Syntax, check_forms, parse_document,
};
use dialog_query::concept::descriptor::ConceptStatement;
use dialog_repository::Branch;
use serde::Serialize;

/// What a cell asks of the worker.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Cell {
    /// The cell's text.
    pub source: String,
    /// The notation cells above it, in order: read for their definitions.
    #[serde(default)]
    pub context: Vec<String>,
}

/// The worker's answer to a cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Answer {
    /// Each top-level form, and what it did.
    pub forms: Vec<FormOutcome>,
    /// The pipeline's stages over the cell.
    pub stages: Vec<Stage>,
    /// Why nothing ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether the cell wrote to the branch.
    pub wrote: bool,
    /// How many of its forms were kept on the branch as data
    /// (`syntax.form/is`).
    pub kept_forms: usize,
}

/// One top-level form of a cell and what it did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FormOutcome {
    /// The form as written.
    pub form: String,
    /// What kind of form it is.
    pub kind: String,
    /// One line on what it did.
    pub summary: String,
    /// The columns of the rows it answered, for a query.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// The rows, a value per column.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<String>>,
    /// Answers as the notation prints them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub printed: Vec<String>,
    /// The markup a view rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Why the notation refused it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<Refusal>,
    /// What the analyzer warns about it, which refuses nothing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A refusal and where it is.
#[derive(Debug, Clone, Serialize)]
pub struct Refusal {
    /// What the notation says.
    pub message: String,
    /// The line, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The column, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// The line of the text it is on, with a caret under the place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

/// One stage of the pipeline.
#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    /// `read`, `expanded`, `analysis` or `compiled`.
    pub name: String,
    /// One line on what the stage is.
    pub note: String,
    /// What it gave.
    pub text: String,
}

/// An analyzer that holds the definitions of `context`.
fn analyzer_after(context: &[String]) -> Result<Analyzer, String> {
    let mut analyzer = Analyzer::new().map_err(|error| error.to_string())?;
    for text in context {
        // Form by form, so a cell above that is refused on purpose does
        // not take its neighbours' definitions with it.
        let _ = check_forms(&mut analyzer, text);
    }
    Ok(analyzer)
}

/// The top-level forms of `text` as written, with their kinds.
fn forms_of(text: &str) -> Vec<(String, String)> {
    super::split(text)
        .into_iter()
        .map(|part| {
            let written = strip_comments(&part);
            let kind = match parse_document(&written).trees.first() {
                Some(tree) => kind_of(tree),
                None => "comment".to_owned(),
            };
            (written, kind)
        })
        .filter(|(written, _)| !written.trim().is_empty())
        .collect()
}

fn strip_comments(part: &str) -> String {
    part.lines()
        .filter(|line| !line.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn kind_of(tree: &Syntax) -> String {
    match tree.applies() {
        Some("rule") => {
            let head = tree
                .operand()
                .and_then(|operand| operand.items.first())
                .and_then(Syntax::applies);
            match head {
                Some("syntax") => "macro".into(),
                Some("warning") => "check".into(),
                Some("assert") | Some("retract") => "inductive rule".into(),
                _ => "deductive rule".into(),
            }
        }
        Some(head) => head.to_owned(),
        None if matches!(tree, Syntax::Tuple(_)) => "macro use".into(),
        None => "form".into(),
    }
}

/// The diagnostics of `checked` as text, and the refusal of each form.
fn diagnostics(
    checked: &Checked,
    text: &str,
) -> (
    String,
    BTreeMap<usize, Refusal>,
    BTreeMap<usize, Vec<String>>,
) {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut refused = BTreeMap::new();
    let mut warned: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for diagnostic in checked.diagnostics() {
        let severity = match diagnostic.severity() {
            Severity::Error => "refused",
            Severity::Warning => "warning",
        };
        let (line, column) = match diagnostic.span {
            Some(span) => (Some(span.start.line + 1), Some(span.start.column + 1)),
            None => (None, None),
        };
        let place = match (line, column) {
            (Some(line), Some(column)) => format!("{line}:{column}"),
            _ => "-".into(),
        };
        out.push_str(&format!("{severity} at {place}: {}\n", diagnostic.message));
        if diagnostic.severity() == Severity::Warning {
            let at = match diagnostic.path.first() {
                Some(dialog_notation::Segment::Item(at)) => *at,
                _ => 0,
            };
            warned
                .entry(at)
                .or_default()
                .push(format!("{place}: {}", diagnostic.message));
        }
        if diagnostic.severity() == Severity::Error {
            let at = match diagnostic.path.first() {
                Some(dialog_notation::Segment::Item(at)) => *at,
                _ => 0,
            };
            let excerpt = line.and_then(|line| {
                let text = lines.get(line as usize - 1)?;
                let caret = " ".repeat(column.unwrap_or(1) as usize - 1);
                Some(format!("{text}\n{caret}^"))
            });
            refused.entry(at).or_insert(Refusal {
                message: diagnostic.message.clone(),
                line,
                column,
                excerpt,
            });
        }
    }
    if out.is_empty() {
        out.push_str("no diagnostics\n");
    }
    (out, refused, warned)
}

fn declarations(checked: &Checked) -> String {
    let mut out = String::new();
    for declaration in checked.declarations() {
        match declaration {
            Declaration::Concept(concept) => {
                let what = if concept.command {
                    "command"
                } else {
                    "concept"
                };
                out.push_str(&format!("{what} {}", concept.name));
                if let Some(entity) = &concept.entity {
                    out.push_str(&format!(" (entity {entity})"));
                }
                out.push('\n');
                for (label, term) in &concept.fields {
                    out.push_str(&format!("  field {label}: {term}\n"));
                }
                for line in &concept.lines {
                    out.push_str(&format!(
                        "  attribute {} of {} is {}{}{}{}\n",
                        line.attribute,
                        line.of,
                        line.is,
                        line.kind
                            .as_ref()
                            .map(|kind| format!(" as {kind}"))
                            .unwrap_or_default(),
                        line.cardinality
                            .as_ref()
                            .map(|c| format!(", cardinality {c}"))
                            .unwrap_or_default(),
                        line.layer
                            .as_ref()
                            .map(|l| format!(", in {l}"))
                            .unwrap_or_default(),
                    ));
                }
                for descriptor in &concept.descriptors {
                    out.push_str(&format!(
                        "  descriptor {}\n",
                        serde_json::to_string(descriptor).unwrap_or_default()
                    ));
                }
            }
            Declaration::Query { name, markup, .. } => {
                let what = if *markup {
                    "query (its answer is markup: a view)"
                } else {
                    "query"
                };
                out.push_str(&format!("{what} {name}\n"));
            }
        }
    }
    if out.is_empty() {
        out.push_str("declares nothing\n");
    }
    out
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| format!("<{error}>"))
}

fn compiled(checked: &Checked) -> String {
    let mut out = String::new();
    for (step, at) in checked.steps().iter().zip(checked.step_forms()) {
        let (name, body) = match step {
            Step::Assert(statement) => ("assert", json(statement)),
            Step::Retract(statement) => ("retract", json(statement)),
            Step::Issue(statement) => ("issue (a command, one tick)", json(statement)),
            Step::Deduce(rule) => ("deductive rule", json(rule)),
            Step::Induce(rule) => ("inductive rule", json(rule)),
            Step::Transient(entity) => ("transient concept", entity.to_string()),
            Step::Query(query) => ("query", json(&query.select)),
            Step::Write(write) => (
                "write once, per match of its when",
                json(&write.query.select),
            ),
            Step::View(plan) => ("view", format!("{plan:#?}")),
            Step::Select(selection) => ("select of a query", format!("{selection:#?}")),
        };
        out.push_str(&format!("form {} → {name}\n{body}\n\n", at + 1));
    }
    if out.is_empty() {
        out.push_str("compiles to no step (a definition lives in the analyzer)\n");
    }
    out
}

/// A cell checked, its writes ready to stage and its questions ready to
/// ask. Made by [`prepare`]; staged by [`Prepared::stage`] into the
/// caller's transaction, which the caller commits; ends in
/// [`Prepared::finish`].
///
/// The writes are dialog-notation's own driver
/// ([`dialog_notation::apply::stage`]): each form's `when` reads the
/// branch as the forms before it in the cell left it.
pub struct Prepared {
    answer: Answer,
    forms: Vec<FormOutcome>,
    checked: Option<Checked>,
    kept: Vec<ConceptStatement>,
    report: DocumentReport,
    done: bool,
}

impl Prepared {
    /// Stage the cell's writes into `transaction`, the caller's, and the
    /// forms the cell keeps as data after them, and give it back to
    /// commit. Nothing is committed. `None` when there is nothing to
    /// commit: the cell was refused before it ran, or a write's `when`
    /// failed to read, which the answer says.
    pub async fn stage<T: Staged, E: Reader>(&mut self, transaction: T, env: &E) -> Option<T> {
        let checked = self.checked.as_ref().filter(|_| !self.done)?;
        dialog_notation::supplied::supply_pipeline(env);
        match apply::stage(transaction, checked, env).await {
            Ok((mut transaction, report)) => {
                self.report = report;
                for kept in &self.kept {
                    transaction = transaction.assert(kept.clone());
                }
                Some(transaction)
            }
            Err(error) => {
                self.answer.error = Some(format!("a write: {error}"));
                self.done = true;
                None
            }
        }
    }

    /// Whether what was staged writes anything.
    pub fn writes(&self) -> bool {
        !self.done
            && (!self.kept.is_empty()
                || self
                    .report
                    .forms
                    .iter()
                    .flat_map(|form| &form.did)
                    .any(|did| {
                        matches!(
                            did,
                            Did::Asserted(_)
                                | Did::Retracted(_)
                                | Did::Issued(_)
                                | Did::Installed(..)
                        )
                    }))
    }

    /// Say the commit failed: nothing is asked.
    pub fn failed(mut self, why: String) -> Answer {
        self.answer.error = Some(format!("the commit failed: {why}"));
        self.answer.forms = self.forms;
        self.answer
    }

    /// Ask the cell's questions on `branch`, after its writes were
    /// committed (`wrote`) or left out (a dry run), and say what each
    /// form did.
    pub async fn finish<E: Reader>(self, branch: &Branch, wrote: bool, env: &E) -> Answer {
        dialog_notation::supplied::supply_pipeline(env);
        let Prepared {
            mut answer,
            mut forms,
            checked,
            report,
            done,
            ..
        } = self;
        let Some(checked) = checked.filter(|_| !done) else {
            answer.forms = forms;
            return answer;
        };
        answer.wrote = wrote;
        finish(&mut forms, &checked, &report, wrote, branch, env).await;
        answer.forms = forms;
        answer
    }
}

/// Check `cell` after its context, and say what each stage of the
/// pipeline gave. Nothing is read from a branch yet: [`Prepared::stage`]
/// runs the cell's writes, reading each `when` as it goes.
pub fn prepare(cell: &Cell) -> Prepared {
    prepare_cell(cell, true)
}

/// Check an interactive document as a whole. Its source is kept by the
/// editor, and the branch stores only what the document writes.
pub fn prepare_document(cell: &Cell) -> Prepared {
    prepare_cell(cell, false)
}

fn prepare_cell(cell: &Cell, tour: bool) -> Prepared {
    let mut answer = Answer::default();
    let done = |answer: Answer, forms: Vec<FormOutcome>| Prepared {
        answer,
        forms,
        checked: None,
        kept: Vec::new(),
        report: DocumentReport::default(),
        done: true,
    };
    let mut analyzer = match analyzer_after(&cell.context) {
        Ok(analyzer) => analyzer,
        Err(error) => {
            answer.error = Some(error);
            return done(answer, Vec::new());
        }
    };
    let text = cell.source.as_str();
    let parsed = parse_document(text);
    let mut read = String::new();
    for tree in &parsed.trees {
        read.push_str(&format!("{tree}\n"));
    }
    read.push_str("\n;; each tree as data (Syntax::value), the record the notation keeps it as\n");
    for tree in &parsed.trees {
        match tree.value() {
            Ok(value) => read.push_str(&format!("{}\n", said(&value))),
            Err(error) => read.push_str(&format!(";; {error}\n")),
        }
    }
    answer.stages.push(Stage {
        name: "read".into(),
        note: "the syntax trees the text parses to, names unresolved".into(),
        text: read,
    });

    let checked = check_forms(&mut analyzer, text);
    answer.stages.push(Stage {
        name: "expanded".into(),
        note: "every form expanded to a rule of the kernel (macros and core forms rewritten)"
            .into(),
        text: if checked.analyzed().is_empty() {
            "nothing: the cell expands to no rule\n".into()
        } else {
            checked
                .analyzed()
                .iter()
                .map(|tree| format!("{tree}\n"))
                .collect()
        },
    });
    let (diagnosed, refused, warned) = diagnostics(&checked, text);
    answer.stages.push(Stage {
        name: "analysis".into(),
        note: "what the cell declares, and what the analyzer says about it".into(),
        text: format!("{}\n{diagnosed}", declarations(&checked)),
    });
    answer.stages.push(Stage {
        name: "compiled".into(),
        note: "the steps Dialog-DB runs, its descriptors as JSON".into(),
        text: compiled(&checked),
    });

    let mut forms: Vec<FormOutcome> = forms_of(text)
        .into_iter()
        .map(|(form, kind)| FormOutcome {
            form,
            kind,
            ..FormOutcome::default()
        })
        .collect();
    let outcome = |forms: &mut Vec<FormOutcome>, at: usize| -> Option<usize> {
        (at < forms.len()).then_some(at)
    };
    for (at, refusal) in refused {
        match outcome(&mut forms, at) {
            Some(at) => {
                forms[at].summary = "refused".into();
                forms[at].refused = Some(refusal);
            }
            None => forms.push(FormOutcome {
                kind: "form".into(),
                summary: "refused".into(),
                refused: Some(refusal),
                ..FormOutcome::default()
            }),
        }
    }
    for (at, warnings) in warned {
        if at < forms.len() {
            forms[at].warnings = warnings;
        }
    }
    if !tour && checked.refusals().next().is_some() {
        return done(answer, forms);
    }
    for (declaration, at) in checked
        .declarations()
        .iter()
        .zip(checked.declaration_forms())
    {
        if let Some(at) = outcome(&mut forms, *at) {
            forms[at].summary = match declaration {
                Declaration::Concept(concept) if concept.command => {
                    format!("declares the command {}", concept.name)
                }
                Declaration::Concept(concept) => format!(
                    "declares {} with {}",
                    concept.name,
                    concept
                        .fields
                        .iter()
                        .map(|(label, _)| format!("{label}:"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Declaration::Query { name, markup, .. } if *markup => {
                    format!("defines the view {name}")
                }
                Declaration::Query { name, .. } => format!("defines the query {name}"),
            };
        }
    }

    // Keep each form the cell ran as data: its tree, as the record the
    // notation keeps a tree as, under `syntax.form/is`, of the entity the
    // tree names. This is the tour's evaluator doing it, the way
    // dialog-notation's own tests do (`it_finds_a_kept_form_by_a_pattern_
    // over_its_tree`): the notation's `check` writes no syntax itself.
    let mut kept = Vec::new();
    if tour
        && !forms.iter().any(|form| form.refused.is_some())
        && let Ok(is) = serde_json::from_value::<dialog_query::AttributeDescriptor>(
            serde_json::json!({ "the": "syntax.form/is", "as": "composite:map" }),
        )
    {
        for tree in &parsed.trees {
            let (Ok(this), Ok(value)) = (tree.entity(), tree.value()) else {
                continue;
            };
            if let Ok(attribution) = is.resolve(value) {
                kept.push(ConceptStatement {
                    this,
                    with: vec![attribution],
                });
            }
        }
        answer.kept_forms = kept.len();
    }
    Prepared {
        answer,
        forms,
        checked: Some(checked),
        kept,
        report: DocumentReport::default(),
        done: false,
    }
}

/// Say what each form did, as [`apply::stage`] reported it, then ask
/// the questions.
async fn finish<E: Reader>(
    forms: &mut Vec<FormOutcome>,
    checked: &Checked,
    report: &DocumentReport,
    transact: bool,
    branch: &Branch,
    env: &E,
) {
    let outcome = |forms: &mut Vec<FormOutcome>, at: usize| -> Option<usize> {
        (at < forms.len()).then_some(at)
    };
    let mut written: BTreeMap<usize, Vec<&'static str>> = BTreeMap::new();
    let mut asks: Vec<usize> = Vec::new();
    let mut same: Vec<(usize, usize)> = Vec::new();
    for form in &report.forms {
        for did in &form.did {
            match did {
                Did::Read => {
                    written.entry(form.form).or_default();
                }
                Did::Asserted(_) => written.entry(form.form).or_default().push("assert"),
                Did::Retracted(_) => written.entry(form.form).or_default().push("retract"),
                Did::Issued(_) => written.entry(form.form).or_default().push("issue"),
                Did::Installed(installation, _) => {
                    written
                        .entry(form.form)
                        .or_default()
                        .push(match installation {
                            Installation::Deduction => "deductive rule",
                            Installation::Induction => "inductive rule",
                            Installation::Transient => "transient",
                        })
                }
                Did::Repeats(earlier) => same.push((form.form, *earlier)),
                Did::Asked => asks.push(form.form),
            }
        }
    }
    let same_asks: Vec<(usize, usize)> = same
        .iter()
        .copied()
        .filter(|(_, earlier)| asks.contains(earlier))
        .collect();
    for (at, earlier) in same {
        if same_asks.iter().any(|(form, _)| *form == at) {
            continue;
        }
        if let Some(at) = outcome(forms, at)
            && forms[at].summary.is_empty()
        {
            forms[at].summary = format!("the same as form {}, installed there once", earlier + 1);
        }
    }
    for (at, what) in written {
        if let Some(at) = outcome(forms, at) {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for what in &what {
                *counts.entry(what).or_default() += 1;
            }
            let said = if counts.is_empty() {
                "matched nothing, wrote nothing".to_owned()
            } else {
                counts
                    .iter()
                    .map(|(what, n)| match *what {
                        "assert" if forms[at].kind.contains("rule") => {
                            "kept the comment above it as its description".to_owned()
                        }
                        "assert" => format!("asserted {n} statement(s)"),
                        "retract" => format!("retracted {n} statement(s)"),
                        "issue" => format!("issued {n} command(s), held one tick"),
                        "deductive rule" => "installed a deductive rule on the branch".into(),
                        "inductive rule" => {
                            "installed an inductive rule: it fires when a commit lands".into()
                        }
                        "transient" => "declared a concept the rules issue as a command".into(),
                        other => format!("{other} ×{n}"),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let said = if transact {
                said
            } else {
                format!("would have {said}")
            };
            if forms[at].summary.is_empty() {
                forms[at].summary = said;
            } else {
                forms[at].summary = format!("{}; {said}", forms[at].summary);
            }
        }
    }

    // Ask: after the commit, so a query reads what the cell wrote. Each
    // form's answer is its own, so its answers stay under it and one
    // that fails does not take the others with it. A form that asks the
    // same as an earlier one (`Did::Same`) gets no entry of its own: it
    // shows the earlier form's answer.
    let answered: BTreeMap<usize, Result<apply::Answer, String>> =
        apply::answer(branch, checked, env)
            .await
            .forms
            .into_iter()
            .map(|form| (form.form, form.answered.map_err(|error| error.to_string())))
            .collect();
    let mut asked: Vec<(usize, usize)> = asks.iter().map(|at| (*at, *at)).collect();
    asked.extend(same_asks.iter().copied());
    for (at, from) in asked {
        let (Some(at), Some(answer)) = (outcome(forms, at), answered.get(&from)) else {
            continue;
        };
        match answer {
            Ok(apply::Answer::Rows(rows)) => tabulate(&mut forms[at], rows),
            Ok(apply::Answer::Rendered(rendered)) => {
                let tree = Rendered {
                    roots: rendered
                        .iter()
                        .flat_map(|tree| tree.roots.clone())
                        .collect(),
                };
                forms[at].summary =
                    format!("rendered {} instance(s) of the view", tree.roots.len());
                forms[at].html = Some(super::live::rendered_html(&tree));
                forms[at].printed = tree
                    .roots
                    .iter()
                    .map(|root| {
                        Rendered {
                            roots: vec![root.clone()],
                        }
                        .hiccup()
                    })
                    .collect();
            }
            Err(error) => {
                let kind = if forms[at].kind == "query" {
                    "query"
                } else {
                    "selection"
                };
                forms[at].summary = format!("the {kind} failed: {error}");
            }
        }
    }
    for form in forms.iter_mut() {
        if form.summary.is_empty() {
            form.summary = match form.kind.as_str() {
                "macro" => "defines a macro: a rule over syntax the analyzer runs".into(),
                "check" => "defines a check: a rule over syntax that warns".into(),
                _ => "nothing to run".into(),
            };
        }
    }
}

/// [`prepare`], a commit straight on `branch` (no stack, no lock), and
/// [`Prepared::finish`]: what a test or a tool with a branch of its own
/// does. The worker commits through its stack instead.
pub async fn evaluate_on_branch<E: Writer>(branch: &Branch, cell: &Cell, env: &E) -> Answer {
    let mut prepared = prepare(cell);
    let Some(transaction) = prepared.stage(branch.transaction(), env).await else {
        return prepared.finish(branch, false, env).await;
    };
    if !prepared.writes() {
        return prepared.finish(branch, false, env).await;
    }
    let committed = match transaction.commit().perform(env).await {
        Ok(batch) => batch.publish().perform(env).await.map(|_| ()),
        Err(error) => Err(error),
    };
    if let Err(error) = committed {
        return prepared.failed(error.to_string());
    }
    if let Err(error) = branch.refresh(env).await {
        return prepared.failed(format!("refreshing the branch: {error}"));
    }
    prepared.finish(branch, true, env).await
}

/// Answers as a table when each is an application, a column for its
/// entity and one for each label; the printed answers always.
fn tabulate(form: &mut FormOutcome, answers: &[Syntax]) {
    let mut printed: Vec<String> = answers.iter().map(ToString::to_string).collect();
    printed.sort();
    printed.dedup();
    let mut table: Vec<BTreeMap<String, String>> = Vec::new();
    let mut tabular = true;
    for answer in answers {
        let Some(operand) = answer.operand() else {
            tabular = false;
            break;
        };
        let mut row = BTreeMap::new();
        for (at, item) in operand.items.iter().enumerate() {
            let label = if at == 0 {
                "this".to_owned()
            } else {
                format!("#{at}")
            };
            row.insert(label, cell(item));
        }
        for (label, term) in &operand.terms {
            row.insert(label.clone(), cell(term));
        }
        table.push(row);
    }
    if tabular && !table.is_empty() {
        let mut columns: Vec<String> = Vec::new();
        for row in &table {
            for label in row.keys() {
                if !columns.contains(label) {
                    columns.push(label.clone());
                }
            }
        }
        // The entity first, then the labels as they sort.
        columns.sort_by_key(|label| (label != "this", label.clone()));
        let mut rows: Vec<Vec<String>> = table
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|label| row.get(label).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        rows.sort();
        rows.dedup();
        form.columns = columns;
        form.rows = rows;
    }
    form.summary = format!("{} answer(s)", printed.len());
    form.printed = printed;
}

/// A term of an answer as a table cell: text bare, the rest as printed.
fn cell(term: &Syntax) -> String {
    match term {
        Syntax::Constant(dialog_corpus::Value::String(text)) => text.clone(),
        other => other.to_string(),
    }
}

/// A value as the notation writes it, records and lists taken apart:
/// `{:operator {:name /select :type :reference} :type :application ...}`.
fn said(value: &dialog_corpus::Value) -> String {
    if let Ok(Some(map)) = dialog_notation::value::record(value) {
        let mut entries: Vec<(&str, &dialog_corpus::Value)> = map.iter().collect();
        entries.sort_by_key(|(key, _)| *key);
        let inner: Vec<String> = entries
            .into_iter()
            .map(|(key, value)| format!(":{key} {}", said(value)))
            .collect();
        return format!("{{{}}}", inner.join(" "));
    }
    if let Ok(Some(items)) = dialog_notation::value::list(value) {
        let inner: Vec<String> = items.iter().map(said).collect();
        return format!("[{}]", inner.join(" "));
    }
    Syntax::Constant(value.clone()).to_string()
}

/// A view cell kept current: the driver's watch over the view the cell
/// selects, its first frame the whole view as HTML and every later one
/// the driver's patches. What a "Watch live" cell streams.
pub struct LiveCell {
    driven: dialog_notation::live::Watched,
    first: bool,
}

/// Watch the first view `cell` selects, after its context, on `branch`.
/// The driver lays the view's rules on the branch's session overlay;
/// nothing is committed.
pub fn watch_cell(branch: &Branch, cell: &Cell) -> Result<LiveCell, String> {
    let mut analyzer = analyzer_after(&cell.context)?;
    let checked = check_forms(&mut analyzer, &cell.source);
    if let Some(refused) = checked.refusals().next() {
        return Err(format!("the cell is refused: {}", refused.message));
    }
    let plan = checked
        .steps()
        .iter()
        .find_map(|step| match step {
            Step::View(plan) => Some((**plan).clone()),
            _ => None,
        })
        .ok_or("the cell selects no view")?;
    let given = plan.given.clone();
    let driven = Live::new(branch.clone())
        .watch(plan, given)
        .map_err(|error| error.to_string())?;
    Ok(LiveCell {
        driven,
        first: true,
    })
}

impl LiveCell {
    /// What moved since the last poll, as one frame: the whole view as
    /// marked HTML on the first poll, the driver's patches after, and
    /// nothing when nothing the view reads moved.
    pub async fn poll<E: Reader>(&mut self, env: &E) -> Result<Option<serde_json::Value>, String> {
        // An error is told in the author's terms, a derived attribute by
        // the name it was written with.
        let patches = match self.driven.poll(env).await {
            Ok(patches) => patches,
            Err(error) => return Err(self.driven.told(&error)),
        };
        if self.first {
            self.first = false;
            let html: String = self
                .driven
                .tree()
                .roots
                .iter()
                .map(|root| marked(root, 0))
                .collect();
            return Ok(Some(serde_json::json!({
                "kind": "notation-view",
                "first": true,
                "html": html,
            })));
        }
        if patches.is_empty() {
            return Ok(None);
        }
        let patches: Vec<serde_json::Value> = patches.iter().filter_map(patch_json).collect();
        Ok(Some(serde_json::json!({
            "kind": "notation-view",
            "patches": patches,
        })))
    }
}

/// A key as the page matches it: the same text on an instance's
/// `data-nk` and in an address.
fn key_text(key: &[dialog_corpus::Value]) -> String {
    format!("{key:?}")
}

fn address(at: &dialog_notation::Address) -> serde_json::Value {
    serde_json::Value::Array(
        at.iter()
            .map(|(slot, key)| serde_json::json!([slot, key_text(key)]))
            .collect(),
    )
}

fn patch_json(patch: &dialog_notation::Patch) -> Option<serde_json::Value> {
    use dialog_notation::Patch;
    Some(match patch {
        Patch::Insert {
            parent,
            slot,
            index,
            instance,
        } => serde_json::json!({
            "op": "insert",
            "parent": address(parent),
            "slot": slot,
            "index": index,
            "html": marked(instance, *slot),
        }),
        Patch::Remove { at } => serde_json::json!({ "op": "remove", "at": address(at) }),
        Patch::SetText { at, slot, text } => {
            serde_json::json!({ "op": "text", "at": address(at), "slot": slot, "text": text })
        }
        Patch::SetAttribute { at, name, value } => serde_json::json!({
            "op": "attribute", "at": address(at), "name": name, "value": value,
        }),
        Patch::SetEvent { at, event, .. } => serde_json::json!({
            "op": "attribute",
            "at": address(at),
            "name": format!("data-notation-on-{}", event.kind),
            "value": posted(event).to_string(),
        }),
    })
}

/// An instance as HTML a page can patch: the instance carries the slot
/// of its parent it stands in (`data-ns`) and its key (`data-nk`), and
/// each slot of it opens with a comment naming it (`t<n>` for a text,
/// `s<n>` for instances).
fn marked(shown: &dialog_notation::Shown, slot: usize) -> String {
    use dialog_notation::Content;
    use dialog_notation::view::Slot;
    let Content::Element {
        tag,
        attributes,
        events,
        slots,
    } = &shown.content;
    let mut out = format!(
        "<{tag} data-ns=\"{slot}\" data-nk=\"{}\"",
        escape(&key_text(&shown.key))
    );
    for (name, value) in attributes {
        out.push_str(&format!(" {name}=\"{}\"", escape(value)));
    }
    for event in events {
        out.push_str(&format!(
            " data-notation-on-{}=\"{}\"",
            event.kind,
            escape(&posted(event).to_string())
        ));
    }
    out.push('>');
    for (at, inner) in slots.iter().enumerate() {
        match inner {
            Slot::Text(text) => out.push_str(&format!("<!--t{at}-->{}", escape(text))),
            Slot::Instances(list) => {
                out.push_str(&format!("<!--s{at}-->"));
                for child in list {
                    out.push_str(&marked(child, at));
                }
            }
        }
    }
    out.push_str(&format!("</{tag}>"));
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// An event bound on an instance, as the page posts it: the command and
/// each field, a value of the row written in the notation, or a path
/// into the live event the page reads when it fires.
fn posted(event: &dialog_notation::Posted) -> serde_json::Value {
    use dialog_notation::Given;
    let fields: Vec<serde_json::Value> = event
        .fields
        .iter()
        .map(|(label, given)| match given {
            Given::Value(value) => {
                serde_json::json!([label, { "notation": Syntax::Constant(value.clone()).to_string() }])
            }
            Given::Written(text) => serde_json::json!([label, { "notation": text }]),
            Given::Path(steps) => serde_json::json!([label, { "path": steps }]),
        })
        .collect();
    serde_json::json!({ "command": event.command, "fields": fields })
}
