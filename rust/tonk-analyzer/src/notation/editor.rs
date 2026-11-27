//! The language-server view of a notation document.

use std::collections::BTreeMap;

use dialog_notation::{Analyzer, Checked, Declaration, Line, Point, Severity, Span, check_forms};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Diagnostic, DiagnosticSeverity,
    Documentation, Hover, HoverContents, MarkupContent, MarkupKind, Position, Range, TextEdit,
};

const FORMS: &[(&str, &str)] = &[
    ("assert", "Write facts."),
    (
        "command",
        "Declare a command that holds for one evaluation round.",
    ),
    (
        "concept",
        "Declare a named relation and the facts behind its fields.",
    ),
    ("distinct", "Collect the distinct values of a query field."),
    ("is", "Bind a value in a query."),
    ("query", "Name a query and define its output."),
    ("retract", "Remove matching facts."),
    ("rule", "Derive facts or describe the effects of a command."),
    ("select", "Ask a query and return its answer."),
    (
        "syntax/analyze",
        "Analyze a form or a list of parsed forms.",
    ),
    (
        "syntax/compile",
        "Compile analyzed forms into inspectable steps.",
    ),
    (
        "syntax/parse",
        "Parse source text into an ordered list of forms.",
    ),
    ("unless", "Require that a premise has no match."),
    ("view", "Use a template as the answer of a query."),
];

/// The definitions and reference metadata after checking a cell.
#[derive(Clone)]
pub struct Editor {
    analyzer: Analyzer,
    references: References,
}

impl Editor {
    /// Start an editor with the core notation.
    pub fn new() -> Result<Self, dialog_notation::Error> {
        Ok(Self {
            analyzer: Analyzer::new()?,
            references: References::builtin(),
        })
    }

    /// Check the next cell and retain the declarations it contributes.
    pub fn check(&mut self, text: &str) -> Vec<Diagnostic> {
        let checked = check_forms(&mut self.analyzer, text);
        self.references.extend(checked.declarations());
        diagnostics(text, &checked)
    }
}

fn position(text: &str, point: Point) -> Position {
    let line = text.split('\n').nth(point.line as usize);
    let Some(line) = line else {
        let mut lines = text.split('\n');
        let last = lines.next_back().unwrap_or_default();
        return Position::new(lines.count() as u32, last.encode_utf16().count() as u32);
    };
    Position::new(
        point.line,
        line.chars()
            .take(point.column as usize)
            .map(char::len_utf16)
            .sum::<usize>() as u32,
    )
}

fn diagnostics(text: &str, checked: &Checked) -> Vec<Diagnostic> {
    checked
        .diagnostics()
        .iter()
        .map(|note| {
            let Span { start, end } = note.span.unwrap_or_default();
            Diagnostic {
                range: Range::new(position(text, start), position(text, end)),
                severity: Some(match note.severity() {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                message: note.message.clone(),
                source: Some("dialog-notation".into()),
                ..Diagnostic::default()
            }
        })
        .collect()
}

#[derive(Clone)]
struct Reference {
    kind: CompletionItemKind,
    detail: String,
    documentation: String,
}

#[derive(Clone, Default)]
struct References {
    names: BTreeMap<String, Reference>,
    fields: BTreeMap<(String, String), Reference>,
}

impl References {
    fn builtin() -> Self {
        let mut refs = References::default();
        for (name, detail) in FORMS {
            refs.names.insert(
                (*name).into(),
                Reference {
                    kind: CompletionItemKind::FUNCTION,
                    detail: "notation form".into(),
                    documentation: (*detail).into(),
                },
            );
        }
        refs
    }

    fn extend(&mut self, declarations: &[Declaration]) {
        for declaration in declarations {
            match declaration {
                Declaration::Concept(concept) => {
                    let kind = if concept.command {
                        "command"
                    } else {
                        "concept"
                    };
                    let fields = concept
                        .fields
                        .iter()
                        .map(|(field, term)| format!("{field}: {term}"))
                        .collect::<Vec<_>>();
                    self.names.insert(
                        concept.name.clone(),
                        Reference {
                            kind: CompletionItemKind::CLASS,
                            detail: format!("{kind} {}", concept.name),
                            documentation: [
                                concept
                                    .description
                                    .as_deref()
                                    .unwrap_or_default()
                                    .to_string(),
                                fields.join(", "),
                            ]
                            .into_iter()
                            .filter(|line| !line.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        },
                    );
                    for (label, _) in &concept.fields {
                        self.fields.insert(
                            (concept.name.clone(), format!("{label}:")),
                            Reference {
                                kind: CompletionItemKind::FIELD,
                                detail: format!("field of {}", concept.name),
                                documentation: concept
                                    .lines
                                    .iter()
                                    .find(|line| line.label.as_ref() == Some(label))
                                    .map_or_else(String::new, describe_line),
                            },
                        );
                    }
                    for line in &concept.lines {
                        self.names.insert(
                            line.attribute.clone(),
                            Reference {
                                kind: CompletionItemKind::PROPERTY,
                                detail: "stored attribute".into(),
                                documentation: describe_line(line),
                            },
                        );
                    }
                }
                Declaration::Query {
                    name, description, ..
                } => {
                    self.names.insert(
                        name.clone(),
                        Reference {
                            kind: CompletionItemKind::FUNCTION,
                            detail: "query".into(),
                            documentation: description.clone().unwrap_or_default(),
                        },
                    );
                }
            }
        }
    }
}

fn describe_line(line: &Line) -> String {
    let mut detail = line.attribute.clone();
    if let Some(kind) = &line.kind {
        detail.push_str(&format!(" as {kind}"));
    }
    if let Some(cardinality) = &line.cardinality {
        detail.push_str(&format!(" · {cardinality}"));
    }
    if let Some(description) = &line.description {
        detail.push_str(&format!("\n\n{description}"));
    }
    detail
}

struct Site<'a> {
    operator: Option<&'a str>,
    prefix: &'a str,
    word: &'a str,
    range: Range,
}

fn symbol(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '-' | '_' | ':' | '?' | '.')
}

/// Recover a token and its enclosing operator even while a form is
/// incomplete. Quoted strings and comments have no reference tokens.
fn site(text: &str, position: Position) -> Option<Site<'_>> {
    let line_start = if position.line == 0 {
        0
    } else {
        text.match_indices('\n').nth(position.line as usize - 1)?.0 + 1
    };
    let line = text[line_start..]
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut units = 0;
    let mut cursor = 0;
    for ch in line.chars() {
        if units >= position.character {
            break;
        }
        units += ch.len_utf16() as u32;
        cursor += ch.len_utf8();
    }
    if units != position.character {
        return None;
    }
    let start = line[..cursor]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| symbol(*ch))
        .last()
        .map_or(cursor, |(at, _)| at);
    let end = cursor
        + line[cursor..]
            .chars()
            .take_while(|ch| symbol(*ch))
            .map(char::len_utf8)
            .sum::<usize>();
    let mut stack: Vec<Option<&str>> = Vec::new();
    let prefix = &text[..line_start + start];
    let mut chars = prefix.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        match ch {
            ';' => {
                if !chars.any(|(_, ch)| ch == '\n') {
                    return None;
                }
            }
            '"' => loop {
                match chars.next()?.1 {
                    '\\' => {
                        chars.next()?;
                    }
                    '"' => break,
                    _ => {}
                }
            },
            '(' | '[' | '{' => stack.push(None),
            ')' | ']' | '}' => {
                stack.pop();
            }
            c if symbol(c) => {
                let mut end = at + c.len_utf8();
                while let Some((next, ch)) = chars.next_if(|(_, ch)| symbol(*ch)) {
                    end = next + ch.len_utf8();
                }
                if let Some(operator @ None) = stack.last_mut() {
                    *operator = Some(&prefix[at..end]);
                }
            }
            _ => {}
        }
    }
    let char_at = |at| line[..at].encode_utf16().count() as u32;
    Some(Site {
        operator: stack.last().copied().flatten(),
        prefix: &line[start..cursor],
        word: &line[start..end],
        range: Range::new(
            Position::new(position.line, char_at(start)),
            Position::new(position.line, char_at(end)),
        ),
    })
}

impl Editor {
    /// Complete a form name or the fields of its enclosing operator.
    pub fn complete(&self, text: &str, at: Position) -> Vec<CompletionItem> {
        let Some(site) = site(text, at) else {
            return Vec::new();
        };
        let refs = &self.references;
        let item = |label: &str, reference: &Reference| CompletionItem {
            label: label.into(),
            kind: Some(reference.kind),
            detail: Some(reference.detail.clone()),
            documentation: Some(Documentation::String(reference.documentation.clone())),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                site.range,
                label.to_string(),
            ))),
            ..CompletionItem::default()
        };
        if let Some(operator) = site.operator {
            refs.fields
                .iter()
                .filter(|((name, label), _)| name == operator && label.starts_with(site.prefix))
                .map(|((_, label), reference)| item(label, reference))
                .collect()
        } else {
            refs.names
                .iter()
                .filter(|(label, _)| label.starts_with(site.prefix))
                .map(|(label, reference)| item(label, reference))
                .collect()
        }
    }

    /// Describe the operation or field under the cursor.
    pub fn hover(&self, text: &str, at: Position) -> Option<Hover> {
        let site = site(text, at)?;
        let refs = &self.references;
        let field = || refs.fields.get(&(site.operator?.into(), site.word.into()));
        let reference = field().or_else(|| {
            (site.operator.is_none())
                .then(|| refs.names.get(site.word))
                .flatten()
        })?;
        Some(Hover {
            range: Some(site.range),
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: format!("{}\n\n{}", reference.detail, reference.documentation),
            }),
        })
    }
}
