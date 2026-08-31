//! Render an [`EvaluateResponse`] to an HTML string.
//!
//! A direct port of the inspector's result rendering (formerly
//! `tonk-ui/src/components/space.rs`, Leptos `view!`) to plain HTML strings the
//! element injects via `set_inner_html`. The result panel is static once a
//! response lands — no reactivity is needed — so string building is both smaller
//! and lower-risk than imperative `web-sys` node construction, and it keeps the
//! crate free of Leptos and the query engine.
//!
//! The markup (classes, `<wa-*>` web components, notation rows) matches the
//! original so the app stylesheet styles it unchanged.

use serde_json::Value;

use crate::response::{EvaluateResponse, QueryMatchBlock, QueryResult, Revision};

/// Block label of a `concept:` query — its results render as `concept!:`.
const CONCEPT_LABEL: &str = "concept";
/// Block label of a `command:` query — `command!:` (transient concept).
const COMMAND_LABEL: &str = "command";
/// Block label of a `rule:` query — `rule!:`.
const RULE_LABEL: &str = "rule";

/// Escape text for safe interpolation into HTML element content / attributes.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// True if `s` reads as a URI (scheme-prefixed or reverse-dotted attribute).
/// Inlined from `tonk_display::notation_format::looks_like_uri` to keep this
/// crate dependency-light.
fn looks_like_uri(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    if s.contains(':') {
        return true;
    }
    s.contains('.') && s.contains('/')
}

/// Render the failure callout (when the most recent submit errored) plus the
/// result panel from the most recent successful response. Either may be empty.
pub fn render_result(failure: Option<&str>, response: Option<&EvaluateResponse>) -> String {
    let failure_html = match failure {
        Some(message) => format!(
            "<wa-callout variant=\"danger\">\
               <wa-icon slot=\"icon\" name=\"circle-exclamation\"></wa-icon>{}\
             </wa-callout>",
            esc(message)
        ),
        None => String::new(),
    };
    let result_html = match response {
        Some(r) => render_evaluate_matches(
            &r.matches_before,
            &r.matches_after,
            r.revision_before.as_ref(),
            r.revision_after.as_ref(),
        ),
        None => String::new(),
    };
    format!(
        "<div class=\"evaluate-result\">\
           <div class=\"evaluate-failure\">{failure_html}</div>\
           <div class=\"evaluate-content\">{result_html}</div>\
         </div>"
    )
}

/// Render the response's match blocks. When the commit changed the result set, a
/// `<wa-comparison>` slider contrasts pre/post; otherwise the blocks render once.
fn render_evaluate_matches(
    before: &[QueryMatchBlock],
    after: &[QueryMatchBlock],
    revision_before: Option<&Revision>,
    revision_after: Option<&Revision>,
) -> String {
    if after.is_empty() && before.is_empty() {
        let badge = revision_badge(revision_after.or(revision_before));
        return format!("<div class=\"evaluate-revision\">{badge}</div>");
    }
    if before == after {
        let badge = revision_badge(revision_after.or(revision_before));
        return format!(
            "<div class=\"evaluate-results wa-stack wa-gap-2xs\">\
               <div class=\"evaluate-revision\">{badge}</div>{}\
             </div>",
            render_result_tabs(after)
        );
    }
    format!(
        "<wa-comparison position=\"50\" class=\"evaluate-comparison\">\
           <div slot=\"before\" class=\"evaluate-side evaluate-side-before wa-stack wa-gap-2xs\">\
             <div class=\"evaluate-revision\">{}</div>{}\
           </div>\
           <div slot=\"after\" class=\"evaluate-side evaluate-side-after wa-stack wa-gap-2xs\">\
             <div class=\"evaluate-revision\">{}</div>{}\
           </div>\
         </wa-comparison>",
        revision_badge(revision_before),
        render_match_block_notation(before),
        revision_badge(revision_after),
        render_match_block_notation(after),
    )
}

/// The three swappable result views as a `<wa-tab-group>` — listed notation,
/// grouped tree, and per-block tables. The element wires the `wa-tab-show`
/// preference persistence after injecting this markup.
fn render_result_tabs(blocks: &[QueryMatchBlock]) -> String {
    format!(
        "<wa-tab-group id=\"evaluate-tabs\" class=\"evaluate-tabs\" placement=\"end\">\
           <wa-tab panel=\"listed\"><wa-icon name=\"list\" variant=\"solid\"></wa-icon></wa-tab>\
           <wa-tab panel=\"tree\"><wa-icon name=\"folder-tree\" variant=\"solid\"></wa-icon></wa-tab>\
           <wa-tab panel=\"table\"><wa-icon name=\"table\" variant=\"solid\"></wa-icon></wa-tab>\
           <wa-tab-panel name=\"listed\">{}</wa-tab-panel>\
           <wa-tab-panel name=\"tree\">{}</wa-tab-panel>\
           <wa-tab-panel name=\"table\">{}</wa-tab-panel>\
         </wa-tab-group>",
        render_match_block_notation(blocks),
        render_match_block_list(blocks),
        render_match_block_tables(blocks),
    )
}

// ---- Table view -------------------------------------------------------------

fn render_match_block_tables(blocks: &[QueryMatchBlock]) -> String {
    let inner: String = blocks.iter().map(render_match_block_table).collect();
    format!("<div class=\"query-tables wa-stack wa-gap-l\">{inner}</div>")
}

fn render_match_block_table(block: &QueryMatchBlock) -> String {
    // Column order: every field name in first-seen order across the block's
    // results, `this` excluded (it leads as its own column).
    let mut columns: Vec<&str> = Vec::new();
    for result in &block.results {
        for name in result.fields.keys() {
            if name != "this" && !columns.iter().any(|c| c == name) {
                columns.push(name);
            }
        }
    }
    let head: String = columns
        .iter()
        .map(|name| format!("<th>{}</th>", esc(name)))
        .collect();
    let rows: String = block
        .results
        .iter()
        .map(|result| {
            let cells: String = columns
                .iter()
                .map(|name| {
                    let cell = result
                        .fields
                        .get(*name)
                        .map(|v| format!("<span>{}</span>", render_field_value(v)))
                        .unwrap_or_default();
                    format!("<td>{cell}</td>")
                })
                .collect();
            format!(
                "<tr>\
                   <td class=\"query-table-this\">\
                     <wa-copy-button value=\"{}\"><span>{}</span></wa-copy-button>\
                   </td>{cells}\
                 </tr>",
                esc(&result.this),
                esc(&result.this),
            )
        })
        .collect();
    format!(
        "<div class=\"query-table\"><table>\
           <thead><tr><th class=\"query-table-this\"><span>{}</span></th>{head}</tr></thead>\
           <tbody>{rows}</tbody>\
         </table></div>",
        esc(&block.label),
    )
}

// ---- Listed (notation) view -------------------------------------------------

fn render_match_block_notation(blocks: &[QueryMatchBlock]) -> String {
    let inner: String = blocks
        .iter()
        .flat_map(|block| {
            let label = block.label.as_str();
            block.results.iter().map(move |result| match label {
                CONCEPT_LABEL => render_concept_record(result, CONCEPT_LABEL),
                COMMAND_LABEL => render_concept_record(result, COMMAND_LABEL),
                RULE_LABEL => render_rule_record(result),
                other => render_notation_record(other, result),
            })
        })
        .collect();
    format!("<div class=\"query-notation wa-stack wa-gap-s\">{inner}</div>")
}

fn render_concept_record(result: &QueryResult, head: &str) -> String {
    let show_transient = head == CONCEPT_LABEL;
    let descriptor = concept_descriptor(result, show_transient);
    let mut body = render_notation_field_at(1, "this", &Value::String(result.this.clone()));
    if let Some(map) = descriptor {
        for (k, v) in map {
            body.push_str(&render_notation_field_at(1, &k, &v));
        }
    }
    format!(
        "<div class=\"notation-record\">\
           <div class=\"notation-row\"><span class=\"tonk-cm-effect\">{}!:</span></div>{body}\
         </div>",
        esc(head),
    )
}

fn render_rule_record(result: &QueryResult) -> String {
    let definition = rule_definition(result);
    let mut body = render_notation_field_at(1, "this", &Value::String(result.this.clone()));
    if let Some(map) = definition {
        for (k, v) in map {
            body.push_str(&render_notation_field_at(1, &k, &v));
        }
    }
    format!(
        "<div class=\"notation-record\">\
           <div class=\"notation-row\"><span class=\"tonk-cm-effect\">rule!:</span></div>{body}\
         </div>"
    )
}

fn render_notation_record(label: &str, result: &QueryResult) -> String {
    let mut body = render_notation_field_at(1, "this", &Value::String(result.this.clone()));
    // Collection entries arrive folded (`show: {ui: <template>}`), so
    // the Object arm below nests them as the entry form naturally.
    for (name, value) in &result.fields {
        if name != "this" {
            body.push_str(&render_notation_field_at(1, name, value));
        }
    }
    format!(
        "<div class=\"notation-record\">\
           <div class=\"notation-row\"><span class=\"tonk-cm-effect\">{}!:</span></div>{body}\
         </div>",
        esc(label),
    )
}

/// Two spaces of notation indent per nesting level.
fn notation_indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// Render one field at nesting `depth` (1 = directly under the head).
fn render_notation_field_at(depth: usize, name: &str, value: &Value) -> String {
    let indent = notation_indent(depth);
    match value {
        Value::Object(map) => {
            let mut out = format!(
                "<div class=\"notation-row notation-field\">\
                   <span class=\"notation-indent\">{}</span>\
                   <span class=\"tonk-cm-key\">{}</span>\
                   <span class=\"tonk-cm-plain\">:</span>\
                 </div>",
                esc(&indent),
                esc(name),
            );
            for (k, v) in map {
                out.push_str(&render_notation_field_at(depth + 1, k, v));
            }
            out
        }
        Value::Array(items) => {
            let dash_indent = notation_indent(depth + 1);
            let mut out = format!(
                "<div class=\"notation-row notation-field\">\
                   <span class=\"notation-indent\">{}</span>\
                   <span class=\"tonk-cm-key\">{}</span>\
                   <span class=\"tonk-cm-plain\">:</span>\
                 </div>",
                esc(&indent),
                esc(name),
            );
            for item in items {
                match item {
                    Value::Object(map) => {
                        let mut fields = map.iter();
                        if let Some((k, v)) = fields.next() {
                            out.push_str(&render_dash_field(&dash_indent, depth + 2, k, v));
                        }
                        for (k, v) in fields {
                            out.push_str(&render_notation_field_at(depth + 2, k, v));
                        }
                    }
                    other => out.push_str(&format!(
                        "<div class=\"notation-row notation-field\">\
                           <span class=\"notation-indent\">{}</span>\
                           <span class=\"tonk-cm-plain\">- </span>{}\
                         </div>",
                        esc(&dash_indent),
                        render_field_value(other),
                    )),
                }
            }
            out
        }
        Value::String(s) if s.contains('\n') => {
            let line_indent = notation_indent(depth + 1);
            let mut out = format!(
                "<div class=\"notation-row notation-field\">\
                   <span class=\"notation-indent\">{}</span>\
                   <span class=\"tonk-cm-key\">{}</span>\
                   <span class=\"tonk-cm-plain\">:</span>\
                 </div>",
                esc(&indent),
                esc(name),
            );
            for line in s.split('\n') {
                out.push_str(&format!(
                    "<div class=\"notation-row notation-value-line\">\
                       <span class=\"notation-indent\">{}</span>\
                       <span class=\"tonk-cm-string\">{}</span>\
                     </div>",
                    esc(&line_indent),
                    esc(line),
                ));
            }
            out
        }
        _ => format!(
            "<div class=\"notation-row notation-field\">\
               <span class=\"notation-indent\">{}</span>\
               <span class=\"tonk-cm-key\">{}</span>\
               <span class=\"tonk-cm-plain\">: </span>{}\
             </div>",
            esc(&indent),
            esc(name),
            render_field_value(value),
        ),
    }
}

/// The first field of a block-sequence object item — shares the `- ` row.
fn render_dash_field(dash_indent: &str, child_depth: usize, name: &str, value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut out = format!(
                "<div class=\"notation-row notation-field\">\
                   <span class=\"notation-indent\">{}</span>\
                   <span class=\"tonk-cm-plain\">- </span>\
                   <span class=\"tonk-cm-key\">{}</span>\
                   <span class=\"tonk-cm-plain\">:</span>\
                 </div>",
                esc(dash_indent),
                esc(name),
            );
            for (k, v) in map {
                out.push_str(&render_notation_field_at(child_depth + 1, k, v));
            }
            out
        }
        Value::Array(items) => {
            format!(
                "<div class=\"notation-row notation-field\">\
                   <span class=\"notation-indent\">{}</span>\
                   <span class=\"tonk-cm-plain\">- </span>\
                   <span class=\"tonk-cm-key\">{}</span>\
                   <span class=\"tonk-cm-plain\">:</span>\
                 </div>{}",
                esc(dash_indent),
                esc(name),
                render_notation_field_at(child_depth, "", &Value::Array(items.clone())),
            )
        }
        _ => format!(
            "<div class=\"notation-row notation-field\">\
               <span class=\"notation-indent\">{}</span>\
               <span class=\"tonk-cm-plain\">- </span>\
               <span class=\"tonk-cm-key\">{}</span>\
               <span class=\"tonk-cm-plain\">: </span>{}\
             </div>",
            esc(dash_indent),
            esc(name),
            render_field_value(value),
        ),
    }
}

// ---- Grouped tree view ------------------------------------------------------

fn render_match_block_list(blocks: &[QueryMatchBlock]) -> String {
    let inner: String = blocks
        .iter()
        .map(|block| {
            let label = block.label.as_str();
            let results: String = block
                .results
                .iter()
                .map(|result| match label {
                    CONCEPT_LABEL => render_concept_tree_item(result, CONCEPT_LABEL),
                    COMMAND_LABEL => render_concept_tree_item(result, COMMAND_LABEL),
                    RULE_LABEL => render_rule_tree_item(result),
                    _ => render_result_tree_item(result),
                })
                .collect();
            format!(
                "<wa-tree-item expanded>\
                   <span class=\"tonk-cm-effect\">{}</span><span class=\"tonk-cm-plain\">:</span>{results}\
                 </wa-tree-item>",
                esc(&block.label),
            )
        })
        .collect();
    format!("<wa-tree class=\"query-tree\">{inner}</wa-tree>")
}

fn render_result_tree_item(result: &QueryResult) -> String {
    let fields: String = result
        .fields
        .iter()
        .map(|(name, value)| {
            format!(
                "<wa-tree-item expanded>\
                   <span class=\"tonk-cm-key\">{}</span><span class=\"tonk-cm-plain\">:</span>\
                   <wa-tree-item>{}</wa-tree-item>\
                 </wa-tree-item>",
                esc(name),
                render_field_value(value),
            )
        })
        .collect();
    format!(
        "<wa-tree-item expanded>\
           <span class=\"tonk-cm-entity\">{}</span><span class=\"tonk-cm-plain\">:</span>{fields}\
         </wa-tree-item>",
        esc(&result.this),
    )
}

fn render_concept_tree_item(result: &QueryResult, head: &str) -> String {
    let show_transient = head == CONCEPT_LABEL;
    let descriptor = concept_descriptor(result, show_transient);
    let mut body = render_notation_tree_item("this", &Value::String(result.this.clone()));
    if let Some(map) = descriptor {
        for (k, v) in map {
            body.push_str(&render_notation_tree_item(&k, &v));
        }
    }
    format!(
        "<wa-tree-item expanded>\
           <span class=\"tonk-cm-effect\">{}</span><span class=\"tonk-cm-plain\">!:</span>{body}\
         </wa-tree-item>",
        esc(head),
    )
}

fn render_rule_tree_item(result: &QueryResult) -> String {
    let definition = rule_definition(result);
    let mut body = render_notation_tree_item("this", &Value::String(result.this.clone()));
    if let Some(map) = definition {
        for (k, v) in map {
            body.push_str(&render_notation_tree_item(&k, &v));
        }
    }
    format!(
        "<wa-tree-item expanded>\
           <span class=\"tonk-cm-effect\">rule!</span><span class=\"tonk-cm-plain\">:</span>{body}\
         </wa-tree-item>"
    )
}

fn render_notation_tree_item(name: &str, value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let children: String = map
                .iter()
                .map(|(k, v)| render_notation_tree_item(k, v))
                .collect();
            format!(
                "<wa-tree-item expanded>\
                   <span class=\"tonk-cm-key\">{}</span><span class=\"tonk-cm-plain\">:</span>{children}\
                 </wa-tree-item>",
                esc(name),
            )
        }
        Value::Array(items) => {
            let children: String = items
                .iter()
                .map(|item| match item {
                    Value::Object(map) => {
                        let fields: String = map
                            .iter()
                            .map(|(k, v)| render_notation_tree_item(k, v))
                            .collect();
                        format!(
                            "<wa-tree-item expanded>\
                               <span class=\"tonk-cm-plain\">-</span>{fields}\
                             </wa-tree-item>"
                        )
                    }
                    other => format!(
                        "<wa-tree-item>\
                           <span class=\"tonk-cm-plain\">- </span>{}\
                         </wa-tree-item>",
                        render_field_value(other),
                    ),
                })
                .collect();
            format!(
                "<wa-tree-item expanded>\
                   <span class=\"tonk-cm-key\">{}</span><span class=\"tonk-cm-plain\">:</span>{children}\
                 </wa-tree-item>",
                esc(name),
            )
        }
        _ => format!(
            "<wa-tree-item expanded>\
               <span class=\"tonk-cm-key\">{}</span><span class=\"tonk-cm-plain\">:</span>\
               <wa-tree-item>{}</wa-tree-item>\
             </wa-tree-item>",
            esc(name),
            render_field_value(value),
        ),
    }
}

// ---- Field values + descriptor expansion ------------------------------------

/// `+41` / `-7`: the wire spelling of a SignedInteger value.
fn is_signed_literal(s: &str) -> bool {
    match s.strip_prefix(['+', '-']) {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Render a single field value as a highlighted `<span>`, applying the
/// `tonk-cm-*` class matching its shape.
fn render_field_value(value: &Value) -> String {
    let (class, text) = match value {
        Value::Null => ("tonk-cm-variable", "_".to_owned()),
        Value::Bool(b) => ("tonk-cm-number", b.to_string()),
        Value::Number(n) => ("tonk-cm-number", n.to_string()),
        Value::String(s) => {
            if is_signed_literal(s) {
                // The wire spelling of a SignedInteger (`+41`, `-7`).
                ("tonk-cm-number", s.clone())
            } else if looks_like_uri(s) {
                ("tonk-cm-entity", s.clone())
            } else {
                ("tonk-cm-string", s.clone())
            }
        }
        other => (
            "tonk-cm-plain",
            serde_json::to_string(other).unwrap_or_else(|_| "<?>".to_owned()),
        ),
    };
    format!("<span class=\"{class}\">{}</span>", esc(&text))
}

/// The full `#<base58>` form of a revision's `tree`, from the wire value.
///
/// The wire `tree` is a `TreeReference` — a Blake3 hash serialized as a byte
/// SEQUENCE (the typed value's `#<base58>` `Display` never reaches the wire). So
/// base58-encode the byte array to reconstruct the display form. A string value
/// (a hypothetical future shape) is used as-is; anything else yields `None`.
fn tree_display(tree: &Value) -> Option<String> {
    match tree {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(bytes) => {
            let raw: Vec<u8> = bytes
                .iter()
                .filter_map(|b| b.as_u64().map(|n| n as u8))
                .collect();
            if raw.is_empty() {
                return None;
            }
            Some(format!("#{}", bs58::encode(raw).into_string()))
        }
        _ => None,
    }
}

/// Github-style short form of a tree reference (`#<base58>` → first 8 chars).
fn abbreviate_tree(tree: &str) -> String {
    const SHORT_LEN: usize = 8;
    let body = tree.strip_prefix('#').unwrap_or(tree);
    body.chars().take(SHORT_LEN).collect()
}

/// A revision as a `<wa-badge>` (truncated tree hash, full hash on `title`).
fn revision_badge(revision: Option<&Revision>) -> String {
    match revision.and_then(|rev| tree_display(&rev.tree)) {
        Some(full) => format!(
            "<wa-badge variant=\"neutral\" appearance=\"filled\" title=\"{}\">\
               <wa-icon name=\"code-commit\" slot=\"start\"></wa-icon>{}\
             </wa-badge>",
            esc(&full),
            esc(&abbreviate_tree(&full)),
        ),
        None => {
            "<wa-badge variant=\"neutral\" appearance=\"filled\">no commits</wa-badge>".to_owned()
        }
    }
}

/// An attribute `Type` discriminant the way it is typed in notation.
fn type_name_to_notation(stored: &str) -> &str {
    match stored {
        "Text" => "text",
        "UnsignedInteger" => "unsigned-integer",
        "SignedInteger" => "signed-integer",
        "Float" => "float",
        "Boolean" => "boolean",
        "Entity" => "entity",
        "Bytes" => "bytes",
        other => other,
    }
}

/// Rewrite every `as` value in a descriptor tree to its notation surface form.
fn notation_normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "as"
                    && let Value::String(s) = child
                {
                    *s = type_name_to_notation(s).to_owned();
                } else {
                    notation_normalize(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                notation_normalize(item);
            }
        }
        _ => {}
    }
}

/// Rewrite every variable term in a rule descriptor tree to its `?name` form.
fn rule_normalize_terms(value: &mut Value) {
    if let Value::Object(map) = value {
        if map.len() == 1
            && let Some(inner) = map.get("?")
        {
            let name = inner.get("name").and_then(Value::as_str).map(str::to_owned);
            *value = match name {
                Some(name) => Value::String(format!("?{name}")),
                None => Value::String("?".to_owned()),
            };
            return;
        }
        for child in map.values_mut() {
            rule_normalize_terms(child);
        }
    } else if let Value::Array(items) = value {
        for item in items {
            rule_normalize_terms(item);
        }
    }
}

/// Extract a concept result's descriptor as an object map (parsed from the
/// stringified `source` field, with `as` discriminants normalized).
fn concept_descriptor(
    result: &QueryResult,
    show_transient: bool,
) -> Option<serde_json::Map<String, Value>> {
    let value = result.fields.get("source")?.clone();
    let map = match value {
        Value::Object(map) => map,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut value = Value::Object(map);
    notation_normalize(&mut value);
    let Value::Object(mut map) = value else {
        unreachable!("value was constructed as an object")
    };
    if show_transient && matches!(result.fields.get("transient"), Some(Value::Bool(true))) {
        map.insert("transient".to_owned(), Value::Bool(true));
    }
    Some(map)
}

/// Expand a `rule:` result's `definition` field into the `rule!:` field layout.
fn rule_definition(result: &QueryResult) -> Option<serde_json::Map<String, Value>> {
    let value = result.fields.get("definition")?.clone();
    let outer = match value {
        Value::Object(map) => map,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut rule = match outer.get("rule") {
        Some(Value::Object(map)) => map.clone(),
        _ => return None,
    };
    // The embedded descriptor carries the head in its polarity's own
    // field now (`retract!` for retract rules), so the swap below is
    // a no-op on current data; it remains for rows written by older
    // releases, whose descriptors always used `assert!`.
    let retract = matches!(
        outer.get("polarity"),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("retract")
    );
    if retract && let Some(head) = rule.remove("assert!") {
        rule.insert("retract!".to_owned(), head);
    }
    let mut value = Value::Object(rule);
    rule_normalize_terms(&mut value);
    notation_normalize(&mut value);
    match value {
        Value::Object(map) => Some(map),
        _ => unreachable!("value was constructed as an object"),
    }
}
