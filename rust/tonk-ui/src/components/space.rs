use leptos::{either::Either, prelude::*, web_sys};
use tonk_worker::{EvaluateResponse, Revision};
use wasm_bindgen::JsCast;

/// State machine for the per-branch transaction editor.
///
/// State of the editor's submit cycle. The worker's `/evaluate`
/// route handles any mix of queries and mutations — the editor
/// no longer has to pre-classify the document.
///
/// Successful results don't live here; they live in
/// `last_response` and stay across re-runs so the result panel
/// keeps rendering during a new in-flight request. This state
/// only carries the *transient* lifecycle (idle / running /
/// failed).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TransactState {
    Idle,
    Running,
    Failed(String),
}

/// Pre-flight check: parser-only. Catches malformed buffers
/// before they hit the worker. Structural errors the parser is
/// permissive about (e.g. `AssertionWithoutFields`) come back
/// as LSP diagnostics in the editor — the LSP runs the
/// analyzer with a no-op resolver on every change and surfaces
/// the errors as squigglies. The worker's analyzer is the final
/// authority for "this can run" so we don't duplicate that work
/// here.
pub(crate) enum DocDispatch {
    /// Parser accepted the buffer and there's at least one
    /// expression. `has_mutation` is true if any expression is
    /// an assertion (`head!:`) — the play button only surfaces
    /// when there's something to commit; pure-query documents
    /// auto-evaluate on every fresh diagnostics frame and don't
    /// need the affordance.
    Submit { has_mutation: bool },
    /// Empty / whitespace-only document.
    Empty,
    /// Parser raised diagnostics.
    ParseError(String),
}

pub(crate) fn classify_for_dispatch(body: &str) -> DocDispatch {
    let parsed = tonk_notation::parse(body);
    if !parsed.diagnostics.is_empty() {
        let messages = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return DocDispatch::ParseError(messages);
    }
    let Some(syntax) = parsed.syntax else {
        return DocDispatch::Empty;
    };
    if syntax.expressions.is_empty() {
        return DocDispatch::Empty;
    }
    let has_mutation = syntax
        .expressions
        .iter()
        .any(|e| matches!(e, tonk_notation::Expression::Claim(_)));
    DocDispatch::Submit { has_mutation }
}

/// Read the `value` property off a `<tonk-code>` element from one
/// of its `change` events. The element exposes the buffer through
/// the standard custom-element property (see
/// `rust/tonk-code/src-js/index.ts:794`); we walk through the
/// event target and pull it reflectively, mirroring
/// [`read_wa_input_value`] but for the editor's contract.
pub(crate) fn read_tonk_code_value(event: &leptos::ev::Event) -> String {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
        .and_then(|el| {
            js_sys::Reflect::get(&el, &wasm_bindgen::JsValue::from_str("value"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_default()
}

/// Clear externally-pushed diagnostics on the editor whose
/// `source` matches by dispatching an empty
/// `tonk-push-diagnostics` event. Called after a successful
/// re-submit so a stale squiggle from a previous failure doesn't
/// linger.
pub(crate) fn clear_pushed_diagnostics(source: &str) {
    dispatch_push_diagnostics(source, &js_sys::Array::new());
}

/// Dispatch a `tonk-push-diagnostics` CustomEvent on the
/// `<tonk-diagnostics-provider>` for `source`. The event detail
/// is `{ source, diagnostics }`. Provider routes by `source` so
/// multiple editors under one provider don't collide.
fn dispatch_push_diagnostics(source: &str, diagnostics: &js_sys::Array) {
    let document = match window().document() {
        Some(d) => d,
        None => return,
    };
    let provider = match document
        .query_selector("tonk-diagnostics-provider")
        .ok()
        .flatten()
    {
        Some(el) => el,
        None => return,
    };
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &detail,
        &wasm_bindgen::JsValue::from_str("source"),
        &wasm_bindgen::JsValue::from_str(source),
    );
    let _ = js_sys::Reflect::set(
        &detail,
        &wasm_bindgen::JsValue::from_str("diagnostics"),
        diagnostics,
    );
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    let event = match web_sys::CustomEvent::new_with_event_init_dict("tonk-push-diagnostics", &init)
    {
        Ok(e) => e,
        Err(_) => return,
    };
    let _ = provider.dispatch_event(&event);
}

/// Render the status surface below the editor.
///
/// `Idle` and `Running` render nothing (the button itself shows
/// the loading spinner via `prop:loading`). The `Done…` variants
/// show kind-specific success callouts; `Failed` shows the
/// worker's error text in a danger callout.
/// Render the area below the editor: the failure callout (when
/// the most recent submit errored) plus the result panel from
/// the most recent successful response. Both regions are always
/// mounted; their inner content swaps. Keeping the wrapper divs
/// in the tree across the Idle → Running → Done cycle prevents
/// the form from shrinking and re-expanding mid-request, which
/// otherwise reads as a "flash" as the page reflows.
pub(crate) fn render_transact_state(
    state: TransactState,
    response: Option<Box<EvaluateResponse>>,
) -> impl IntoView {
    let failure = match state {
        TransactState::Failed(message) => Either::Left(view! {
            <wa-callout variant="danger">
                <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                { message }
            </wa-callout>
        }),
        TransactState::Idle | TransactState::Running => Either::Right(()),
    };
    let result = match response {
        Some(response) => {
            let response = *response;
            Either::Left(render_evaluate_matches(
                response.matches_before,
                response.matches_after,
                response.revision_before,
                response.revision_after,
            ))
        }
        None => Either::Right(()),
    };
    view! {
        <div class="evaluate-result">
            <div class="evaluate-failure">{ failure }</div>
            <div class="evaluate-content">{ result }</div>
        </div>
    }
}

/// Render the evaluate response's match blocks.
///
/// When the commit changed the result set, render a
/// `<wa-comparison>` slider with the pre-commit state on the
/// left (dimmed) and the post-commit state on the right, with
/// each side's branch revision badged in its header. Otherwise
/// just render the blocks once with the after-revision badge.
fn render_evaluate_matches(
    before: Vec<tonk_worker::QueryMatchBlock>,
    after: Vec<tonk_worker::QueryMatchBlock>,
    revision_before: Option<Revision>,
    revision_after: Option<Revision>,
) -> impl IntoView {
    use leptos::either::EitherOf3;
    if after.is_empty() && before.is_empty() {
        return EitherOf3::A(view! {
            <div class="evaluate-revision">{ revision_badge(revision_after.or(revision_before)) }</div>
        });
    }
    if before == after {
        let badge = revision_badge(revision_after.or(revision_before.clone()));
        return EitherOf3::B(view! {
            <div class="evaluate-results wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ badge }</div>
                { render_result_tabs(after) }
            </div>
        });
    }
    // Commit changed the result set — a `<wa-comparison>` slider
    // contrasts pre/post state. Each side stays single-view (the
    // listed notation); a tab group inside a comparison half
    // would be too cramped.
    EitherOf3::C(view! {
        <wa-comparison position="50" class="evaluate-comparison">
            <div slot="before" class="evaluate-side evaluate-side-before wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ revision_badge(revision_before) }</div>
                { render_match_block_notation(before) }
            </div>
            <div slot="after" class="evaluate-side evaluate-side-after wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ revision_badge(revision_after) }</div>
                { render_match_block_notation(after) }
            </div>
        </wa-comparison>
    })
}

/// `localStorage` key holding the user's preferred result view
/// (`listed` / `tree` / `table`). Persisting it makes the choice
/// stick across results and across reloads.
const RESULT_VIEW_KEY: &str = "tonk:result-view";

/// Read the persisted result-view preference, falling back to
/// `listed` when nothing is stored (or the stored value isn't a
/// known panel name).
fn result_view_pref() -> String {
    let stored = window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(RESULT_VIEW_KEY).ok().flatten());
    match stored.as_deref() {
        Some(view @ ("listed" | "tree" | "table")) => view.to_owned(),
        _ => "listed".to_owned(),
    }
}

/// Persist the chosen result view so the next result — and the
/// next session — opens on the same tab.
fn store_result_view_pref(view: &str) {
    if let Ok(Some(storage)) = window().local_storage() {
        let _ = storage.set_item(RESULT_VIEW_KEY, view);
    }
}

/// Render the result in three swappable views — listed notation,
/// grouped tree, and a per-block table — as panels of a
/// `<wa-tab-group>` with the tabs down the inline-end side. The
/// active tab is the user's persisted preference; switching tabs
/// writes the new choice back, so every later result opens on
/// the same view.
fn render_result_tabs(blocks: Vec<tonk_worker::QueryMatchBlock>) -> impl IntoView {
    use wasm_bindgen::closure::Closure;

    let tree_blocks = blocks.clone();
    let table_blocks = blocks.clone();
    let active = result_view_pref();

    // The `<wa-tab-group>` is reached by id after mount (a typed
    // `NodeRef` for a custom element is awkward in Leptos). The
    // `wa-tab-show` event carries the newly-shown panel name in
    // `event.detail.name`; persisting it makes the preference
    // follow the user's last pick. The listener outlives this
    // function, so its closure is intentionally leaked.
    let group_id = "evaluate-tabs";
    Effect::new(move |_| {
        let Some(el) = window()
            .document()
            .and_then(|d| d.get_element_by_id(group_id))
        else {
            return;
        };
        let cb = Closure::<dyn FnMut(web_sys::CustomEvent)>::new(|ev: web_sys::CustomEvent| {
            let name = js_sys::Reflect::get(&ev.detail(), &wasm_bindgen::JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string());
            if let Some(name) = name {
                store_result_view_pref(&name);
            }
        });
        let _ = el.add_event_listener_with_callback("wa-tab-show", cb.as_ref().unchecked_ref());
        cb.forget();
    });

    view! {
        <wa-tab-group
            id=group_id
            class="evaluate-tabs"
            placement="end"
            prop:active=active
        >
            <wa-tab panel="listed">
                <wa-icon name="list" variant="solid"></wa-icon>
            </wa-tab>
            <wa-tab panel="tree">
                <wa-icon name="folder-tree" variant="solid"></wa-icon>
            </wa-tab>
            <wa-tab panel="table">
                <wa-icon name="table" variant="solid"></wa-icon>
            </wa-tab>
            <wa-tab-panel name="listed">
                { render_match_block_notation(blocks) }
            </wa-tab-panel>
            <wa-tab-panel name="tree">
                { render_match_block_list(tree_blocks) }
            </wa-tab-panel>
            <wa-tab-panel name="table">
                { render_match_block_tables(table_blocks) }
            </wa-tab-panel>
        </wa-tab-group>
    }
}

/// Table rendering — one `<table>` per query block. The header
/// row is the projected field names; each result is a row, with
/// the entity URI in a leading `this` column. The `this` column
/// is monospaced and hard-clipped to its last few characters
/// (the unique suffix), the full URI on the cell `title`.
fn render_match_block_tables(blocks: Vec<tonk_worker::QueryMatchBlock>) -> impl IntoView {
    view! {
        <div class="query-tables wa-stack wa-gap-l">
            { blocks.into_iter().map(render_match_block_table).collect_view() }
        </div>
    }
}

/// One query block as a table. Columns are the union of field
/// names across the block's results, in first-seen order, with
/// `this` always leading.
fn render_match_block_table(block: tonk_worker::QueryMatchBlock) -> impl IntoView {
    // Column order: every field name in first-seen order across
    // the block's results. Results in a block share a projection,
    // but a union keeps the table correct if they ever diverge.
    let mut columns: Vec<String> = Vec::new();
    for result in &block.results {
        for name in result.fields.keys() {
            if name != "this" && !columns.contains(name) {
                columns.push(name.clone());
            }
        }
    }
    let header_columns = columns.clone();
    view! {
        <div class="query-table">
            <table>
                <thead>
                    <tr>
                        // First column is headed by the concept name
                        // (the query's head) rather than the literal
                        // `this`; its cells carry the entity URI. The
                        // name sits in a span so the inverse-color
                        // cover hugs the text, not the whole cell.
                        <th class="query-table-this">
                            <span>{ block.label }</span>
                        </th>
                        { header_columns.into_iter()
                            .map(|name| view! { <th>{ name }</th> })
                            .collect_view() }
                    </tr>
                </thead>
                <tbody>
                    { block.results.into_iter().map(move |result| {
                        let entity = result.this.clone();
                        let entity_label = entity.clone();
                        let columns = columns.clone();
                        view! {
                            <tr>
                                // The entity URI is hard-clipped to its
                                // trailing characters; the full value
                                // sits on `<wa-copy-button>` so a click
                                // copies it. The truncated span is the
                                // button's custom trigger (default slot).
                                <td class="query-table-this">
                                    <wa-copy-button value=entity>
                                        <span>{ entity_label }</span>
                                    </wa-copy-button>
                                </td>
                                { columns.into_iter().map(move |name| {
                                    let cell = result.fields.get(&name).cloned();
                                    view! {
                                        <td>
                                            { cell.map(|v| view! {
                                                <span>{ render_field_value(v) }</span>
                                            }) }
                                        </td>
                                    }
                                }).collect_view() }
                            </tr>
                        }
                    }).collect_view() }
                </tbody>
            </table>
        </div>
    }
}

/// Listed (inspector) rendering — flatten every result across all
/// blocks into a stack of notation-shaped records. Each result
/// renders as a `<label>!:` head row followed by one row per
/// field, every row its own element so lines stay independently
/// styleable and selectable. Values reuse the shared `tonk-cm-*`
/// classifier; long single-line values ellipsize with a
/// click-to-expand; multi-line values render one element per
/// line. Highlighting and typography match the editor.
fn render_match_block_notation(blocks: Vec<tonk_worker::QueryMatchBlock>) -> impl IntoView {
    view! {
        <div class="query-notation wa-stack wa-gap-s">
            { blocks.into_iter().flat_map(|block| {
                let label = block.label;
                let is_concept = label == CONCEPT_LABEL;
                let is_command = label == COMMAND_LABEL;
                let is_rule = label == RULE_LABEL;
                block.results.into_iter().map(move |result| {
                    if is_concept {
                        render_concept_record(result, CONCEPT_LABEL).into_any()
                    } else if is_command {
                        render_concept_record(result, COMMAND_LABEL).into_any()
                    } else if is_rule {
                        render_rule_record(result).into_any()
                    } else {
                        render_notation_record(label.clone(), result).into_any()
                    }
                }).collect::<Vec<_>>()
            }).collect_view() }
        </div>
    }
}

/// Block label of a `concept:` query. Results in a block with this
/// label are concept definitions and render as `concept!:`
/// assertions (the `source` descriptor expanded as notation)
/// rather than the generic field-by-field record.
const CONCEPT_LABEL: &str = "concept";

/// Block label of a `command:` query. A command *is* a transient
/// concept, so results render as `command!:` definitions — same
/// descriptor expansion as a concept, but with the `command!:`
/// head and without the redundant `transient:` row (the keyword
/// already implies it).
const COMMAND_LABEL: &str = "command";

/// Block label of a `rule:` query. Results in a block with this
/// label are inductive-rule definitions and render as `rule!:`
/// assertions (the `definition` descriptor expanded as notation)
/// rather than the generic field-by-field record.
const RULE_LABEL: &str = "rule";

/// Render an attribute `Type` discriminant the way it is *typed*
/// in notation.
///
/// A descriptor stores `as` as dialog's PascalCase serde
/// discriminant (`Text`, `UnsignedInteger`, …), but the analyzer
/// accepts — and the guide teaches — the kebab-case surface form
/// (`text`, `unsigned-integer`, …). The concept view shows what a
/// user would type, so it translates back. An unrecognized value
/// is passed through unchanged.
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

/// Rewrite every `as` value in a descriptor tree to its
/// notation surface form (see [`type_name_to_notation`]). Walks
/// objects and arrays so the `as` inside each `with` attribute is
/// caught regardless of nesting depth.
fn notation_normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "as"
                    && let serde_json::Value::String(s) = child
                {
                    *s = type_name_to_notation(s).to_owned();
                } else {
                    notation_normalize(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                notation_normalize(item);
            }
        }
        _ => {}
    }
}

/// Extract a concept result's descriptor as an object map.
///
/// The `source` attribute of `db:concept` is typed `Text`, so the
/// descriptor arrives as a *stringified* JSON object, not a
/// structured value — it has to be parsed before its keys can be
/// expanded. The `as` discriminants are rewritten to their
/// notation surface form so the rendered concept reads as the
/// user would type it.
///
/// When `result.fields.get("transient")` is `Bool(true)`, a
/// `transient: true` entry is inserted into the map so the
/// rendered notation surfaces the marker. Durable concepts
/// (absent or `Bool(false)`) get no row — the convention is that
/// `transient: true` is affirmative, absence means durable.
///
/// Returns `None` when there's no `source` field or it doesn't
/// parse as a JSON object.
fn concept_descriptor(
    result: &tonk_worker::QueryResult,
    show_transient: bool,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let value = result.fields.get("source")?.clone();
    let map = match value {
        // Already structured (a future schema might store it so).
        serde_json::Value::Object(map) => map,
        // Stringified JSON — the current `Text`-typed shape.
        serde_json::Value::String(s) => match serde_json::from_str(&s) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut value = serde_json::Value::Object(map);
    notation_normalize(&mut value);
    let mut map = match value {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("value was constructed as an object"),
    };
    // A `command!:` head already implies transience, so only the
    // `concept!:` rendering surfaces the explicit `transient:` row.
    if show_transient
        && matches!(
            result.fields.get("transient"),
            Some(serde_json::Value::Bool(true))
        )
    {
        map.insert("transient".to_owned(), serde_json::Value::Bool(true));
    }
    Some(map)
}

/// One concept result as a `concept!:` assertion: the head, a
/// `this:` row for the concept entity, then the `source`
/// descriptor's own keys (`description`, `with`, …) expanded as
/// nested notation. The `name`/`concept` projection fields are
/// vestigial here — the descriptor in `source` is the definition.
fn render_concept_record(result: tonk_worker::QueryResult, head: &'static str) -> impl IntoView {
    // `command!:` implies transience; only `concept!:` shows the row.
    let show_transient = head == CONCEPT_LABEL;
    let descriptor = concept_descriptor(&result, show_transient);
    let entity = result.this;
    view! {
        <div class="notation-record">
            <div class="notation-row">
                <span class="tonk-cm-effect">{ format!("{head}!:") }</span>
            </div>
            { render_notation_field_at(
                1,
                "this".to_owned(),
                serde_json::Value::String(entity),
            ) }
            { descriptor.map(|map| map
                .into_iter()
                .map(|(k, v)| render_notation_field_at(1, k, v))
                .collect_view()) }
        </div>
    }
}

/// Rewrite every term in a rule descriptor tree to its notation
/// surface form. A serialized [`Term`](dialog_query::Term)
/// variable is `{ "?": { "name": "foo" } }` (named) or `{ "?":
/// {} }` (anonymous); notation writes those as `?foo` and `?`.
/// Walks objects and arrays so a `where` binding at any depth is
/// caught — the rule-side parallel of [`notation_normalize`].
fn rule_normalize_terms(value: &mut serde_json::Value) {
    use serde_json::Value;
    if let Value::Object(map) = value {
        // A single-key `{"?": …}` object is a variable term.
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

/// Expand a `rule:` result's `definition` field into the field
/// layout a `rule!:` head is typed with.
///
/// The `definition` attribute is typed `Text`, so the rule arrives
/// as a *stringified* JSON [`RuleDefinition`](tonk_schema::rule_query::RuleDefinition)
/// — `{ "rule": <InductiveRuleDescriptor>, "polarity": … }`. The
/// inner descriptor already serializes to the `rule!:` shape
/// (`assert!` / `when` / `unless`); this lifts those keys to the
/// top, renames the head to `retract!` when the polarity is
/// `Retract`, rewrites variable terms to `?name` form, and
/// normalizes `as` discriminants. Returns `None` when there is no
/// `definition` field or it doesn't parse.
fn rule_definition(
    result: &tonk_worker::QueryResult,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;
    let value = result.fields.get("definition")?.clone();
    let outer = match value {
        Value::Object(map) => map,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    // The inner `rule` object is the InductiveRuleDescriptor.
    let mut rule = match outer.get("rule") {
        Some(Value::Object(map)) => map.clone(),
        _ => return None,
    };
    // Retract-polarity rules type their head as `retract!`.
    let retract = matches!(outer.get("polarity"), Some(Value::String(s)) if s == "Retract");
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

/// One rule result as a `rule!:` assertion: the head, a `this:`
/// row for the effect entity, then the `definition` descriptor's
/// own keys (`assert!` / `when` / `unless`, …) expanded as nested
/// notation. Mirrors [`render_concept_record`].
fn render_rule_record(result: tonk_worker::QueryResult) -> impl IntoView {
    let definition = rule_definition(&result);
    let entity = result.this;
    view! {
        <div class="notation-record">
            <div class="notation-row">
                <span class="tonk-cm-effect">"rule!:"</span>
            </div>
            { render_notation_field_at(
                1,
                "this".to_owned(),
                serde_json::Value::String(entity),
            ) }
            { definition.map(|map| map
                .into_iter()
                .map(|(k, v)| render_notation_field_at(1, k, v))
                .collect_view()) }
        </div>
    }
}

/// One result as a notation-shaped record: a `head!:` row, the
/// `this:` entity row, then a row per projected field.
fn render_notation_record(label: String, result: tonk_worker::QueryResult) -> impl IntoView {
    let head = format!("{label}!:");
    let entity = result.this;
    view! {
        <div class="notation-record">
            <div class="notation-row">
                <span class="tonk-cm-effect">{ head }</span>
            </div>
            { render_notation_field("this".to_owned(), serde_json::Value::String(entity)) }
            { result.fields.into_iter()
                .filter(|(name, _)| name != "this")
                .map(|(name, value)| render_notation_field(name, value))
                .collect_view() }
        </div>
    }
}

/// One field of a notation record at the top level (one indent
/// under the head). Thin wrapper over [`render_notation_field_at`].
fn render_notation_field(name: String, value: serde_json::Value) -> AnyView {
    render_notation_field_at(1, name, value)
}

/// Two spaces of notation indent per nesting level, as a literal
/// string. Indentation is real text — not CSS padding — so a
/// selection copied out of the result keeps its structure when
/// pasted elsewhere.
fn notation_indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// Render one field at nesting `depth` (1 = directly under the
/// head). Each row opens with a literal-space indent span so the
/// rendered text is copy-paste faithful — `depth` levels of two
/// spaces, the same as the notation a user would type.
///
/// - A nested object recurses: a bare `key:` row followed by its
///   children one level deeper, so a `with:` block reads as
///   indented notation rather than JSON.
/// - A multi-line string drops its lines onto their own rows,
///   indented one level past the key.
/// - Every other value sits inline on the `key: value` row.
fn render_notation_field_at(depth: usize, name: String, value: serde_json::Value) -> AnyView {
    let indent = notation_indent(depth);
    if let serde_json::Value::Object(map) = value {
        return view! {
            <div class="notation-row notation-field">
                <span class="notation-indent">{ indent }</span>
                <span class="tonk-cm-key">{ name }</span>
                <span class="tonk-cm-plain">":"</span>
            </div>
            { map.into_iter()
                .map(|(k, v)| render_notation_field_at(depth + 1, k, v))
                .collect_view() }
        }
        .into_any();
    }
    // An array renders as a YAML block sequence: the key row, then
    // one `- ` marker row per item followed by the item's fields
    // indented under it. This is what makes a rule's `when:`
    // premise list read as notation instead of a JSON blob.
    if let serde_json::Value::Array(items) = value {
        let dash_indent = notation_indent(depth + 1);
        return view! {
            <div class="notation-row notation-field">
                <span class="notation-indent">{ indent }</span>
                <span class="tonk-cm-key">{ name }</span>
                <span class="tonk-cm-plain">":"</span>
            </div>
            { items.into_iter().map(move |item| {
                let dash_indent = dash_indent.clone();
                match item {
                    // An object item: the first field shares the
                    // `- ` row; the remaining fields align under it
                    // (the dash's indent plus the two-char dash
                    // width), so a premise reads `- assert:` with
                    // `where:` lined up beneath `assert`.
                    serde_json::Value::Object(map) => {
                        let mut fields = map.into_iter();
                        let first = fields.next();
                        let rest: Vec<_> = fields.collect();
                        view! {
                            { first.map(|(k, v)| render_dash_field(
                                dash_indent.clone(), depth + 2, k, v,
                            )) }
                            { rest.into_iter()
                                .map(|(k, v)| render_notation_field_at(depth + 2, k, v))
                                .collect_view() }
                        }
                        .into_any()
                    }
                    // A scalar item sits inline after the dash.
                    other => view! {
                        <div class="notation-row notation-field">
                            <span class="notation-indent">{ dash_indent.clone() }</span>
                            <span class="tonk-cm-plain">"- "</span>
                            { render_field_value(other) }
                        </div>
                    }
                    .into_any(),
                }
            }).collect_view() }
        }
        .into_any();
    }
    // A multi-line string is the only scalar that spills past one
    // row — its lines sit one level deeper than the key.
    if let serde_json::Value::String(s) = &value
        && s.contains('\n')
    {
        let line_indent = notation_indent(depth + 1);
        let lines: Vec<String> = s.split('\n').map(str::to_owned).collect();
        return view! {
            <div class="notation-row notation-field">
                <span class="notation-indent">{ indent }</span>
                <span class="tonk-cm-key">{ name }</span>
                <span class="tonk-cm-plain">":"</span>
            </div>
            { lines.into_iter().map(move |line| {
                let line_indent = line_indent.clone();
                view! {
                    <div class="notation-row notation-value-line">
                        <span class="notation-indent">{ line_indent }</span>
                        <span class="tonk-cm-string">{ line }</span>
                    </div>
                }
            }).collect_view() }
        }
        .into_any();
    }
    view! {
        <div class="notation-row notation-field">
            <span class="notation-indent">{ indent }</span>
            <span class="tonk-cm-key">{ name }</span>
            <span class="tonk-cm-plain">": "</span>
            { render_field_value(value) }
        </div>
    }
    .into_any()
}

/// Render the first field of a YAML block-sequence object item —
/// the one that shares the `- ` marker's row. `dash_indent` is the
/// indent before the dash; `child_depth` is where this field's
/// nested values (and its object/array children) recurse, which is
/// also where the item's *sibling* fields align. Mirrors
/// [`render_notation_field_at`] but the leading run is
/// `dash_indent` + `"- "` instead of a plain indent.
fn render_dash_field(
    dash_indent: String,
    child_depth: usize,
    name: String,
    value: serde_json::Value,
) -> AnyView {
    if let serde_json::Value::Object(map) = value {
        return view! {
            <div class="notation-row notation-field">
                <span class="notation-indent">{ dash_indent }</span>
                <span class="tonk-cm-plain">"- "</span>
                <span class="tonk-cm-key">{ name }</span>
                <span class="tonk-cm-plain">":"</span>
            </div>
            { map.into_iter()
                .map(|(k, v)| render_notation_field_at(child_depth + 1, k, v))
                .collect_view() }
        }
        .into_any();
    }
    if let serde_json::Value::Array(items) = value {
        // A nested array under a dash-row key is rare in rule
        // notation, but render it correctly: the key on the dash
        // row, the sequence one level deeper.
        return view! {
            <div class="notation-row notation-field">
                <span class="notation-indent">{ dash_indent }</span>
                <span class="tonk-cm-plain">"- "</span>
                <span class="tonk-cm-key">{ name }</span>
                <span class="tonk-cm-plain">":"</span>
            </div>
            { render_notation_field_at(
                child_depth,
                String::new(),
                serde_json::Value::Array(items),
            ) }
        }
        .into_any();
    }
    view! {
        <div class="notation-row notation-field">
            <span class="notation-indent">{ dash_indent }</span>
            <span class="tonk-cm-plain">"- "</span>
            <span class="tonk-cm-key">{ name }</span>
            <span class="tonk-cm-plain">": "</span>
            { render_field_value(value) }
        </div>
    }
    .into_any()
}

/// Grouped rendering — a `<wa-tree>` nesting concept → entity →
/// field → value. Concept, entity, and field rows all expand; the
/// value is the only leaf. Directory rows carry a trailing `:` so
/// the tree reads like the YAML notation. Highlighting reuses the
/// same `tonk-cm-*` palette the notation renderer uses.
fn render_match_block_list(blocks: Vec<tonk_worker::QueryMatchBlock>) -> impl IntoView {
    view! {
        <wa-tree class="query-tree">
            { blocks.into_iter().map(|block| {
                let is_concept = block.label == CONCEPT_LABEL;
                let is_command = block.label == COMMAND_LABEL;
                let is_rule = block.label == RULE_LABEL;
                view! {
                    <wa-tree-item expanded>
                        <span class="tonk-cm-effect">{ block.label }</span><span class="tonk-cm-plain">":"</span>
                        { block.results.into_iter().map(move |result| {
                            if is_concept {
                                render_concept_tree_item(result, CONCEPT_LABEL).into_any()
                            } else if is_command {
                                render_concept_tree_item(result, COMMAND_LABEL).into_any()
                            } else if is_rule {
                                render_rule_tree_item(result).into_any()
                            } else {
                                render_result_tree_item(result).into_any()
                            }
                        }).collect_view() }
                    </wa-tree-item>
                }
            }).collect_view() }
        </wa-tree>
    }
}

/// One generic query result as a tree item: the entity URI as an
/// expandable directory, each projected field a child whose value
/// is the leaf.
fn render_result_tree_item(result: tonk_worker::QueryResult) -> impl IntoView {
    view! {
        <wa-tree-item expanded>
            <span class="tonk-cm-entity">{ result.this }</span><span class="tonk-cm-plain">":"</span>
            { result.fields.into_iter().map(|(name, value)| view! {
                <wa-tree-item expanded>
                    <span class="tonk-cm-key">{ name }</span><span class="tonk-cm-plain">":"</span>
                    <wa-tree-item>
                        { render_field_value(value) }
                    </wa-tree-item>
                </wa-tree-item>
            }).collect_view() }
        </wa-tree-item>
    }
}

/// One concept result as a `concept!:` tree item: a `this:` child
/// for the entity, then the `source` descriptor's keys expanded as
/// nested tree items so `with:` reads as a notation block.
fn render_concept_tree_item(result: tonk_worker::QueryResult, head: &'static str) -> impl IntoView {
    let show_transient = head == CONCEPT_LABEL;
    let descriptor = concept_descriptor(&result, show_transient);
    let entity = result.this;
    view! {
        <wa-tree-item expanded>
            <span class="tonk-cm-effect">{ head }</span><span class="tonk-cm-plain">"!:"</span>
            { render_notation_tree_item(
                "this".to_owned(),
                serde_json::Value::String(entity),
            ) }
            { descriptor.map(|map| map
                .into_iter()
                .map(|(k, v)| render_notation_tree_item(k, v))
                .collect_view()) }
        </wa-tree-item>
    }
}

/// One rule result as a `rule!:` tree item: a `this:` child for
/// the effect entity, then the `definition` descriptor's keys
/// expanded as nested tree items. Mirrors
/// [`render_concept_tree_item`].
fn render_rule_tree_item(result: tonk_worker::QueryResult) -> impl IntoView {
    let definition = rule_definition(&result);
    let entity = result.this;
    view! {
        <wa-tree-item expanded>
            <span class="tonk-cm-effect">"rule!"</span><span class="tonk-cm-plain">":"</span>
            { render_notation_tree_item(
                "this".to_owned(),
                serde_json::Value::String(entity),
            ) }
            { definition.map(|map| map
                .into_iter()
                .map(|(k, v)| render_notation_tree_item(k, v))
                .collect_view()) }
        </wa-tree-item>
    }
}

/// Render `name: value` as a tree item. A nested object becomes an
/// expandable `key:` directory whose children recurse; every other
/// value is a `key:` directory with the value as its single leaf.
fn render_notation_tree_item(name: String, value: serde_json::Value) -> AnyView {
    if let serde_json::Value::Object(map) = value {
        return view! {
            <wa-tree-item expanded>
                <span class="tonk-cm-key">{ name }</span><span class="tonk-cm-plain">":"</span>
                { map.into_iter()
                    .map(|(k, v)| render_notation_tree_item(k, v))
                    .collect_view() }
            </wa-tree-item>
        }
        .into_any();
    }
    // An array: the key as a directory, one `-` child per item.
    // An object item nests its fields; a scalar is the leaf.
    if let serde_json::Value::Array(items) = value {
        return view! {
            <wa-tree-item expanded>
                <span class="tonk-cm-key">{ name }</span><span class="tonk-cm-plain">":"</span>
                { items.into_iter().map(|item| match item {
                    serde_json::Value::Object(map) => view! {
                        <wa-tree-item expanded>
                            <span class="tonk-cm-plain">"-"</span>
                            { map.into_iter()
                                .map(|(k, v)| render_notation_tree_item(k, v))
                                .collect_view() }
                        </wa-tree-item>
                    }
                    .into_any(),
                    other => view! {
                        <wa-tree-item>
                            <span class="tonk-cm-plain">"- "</span>
                            { render_field_value(other) }
                        </wa-tree-item>
                    }
                    .into_any(),
                }).collect_view() }
            </wa-tree-item>
        }
        .into_any();
    }
    view! {
        <wa-tree-item expanded>
            <span class="tonk-cm-key">{ name }</span><span class="tonk-cm-plain">":"</span>
            <wa-tree-item>
                { render_field_value(value) }
            </wa-tree-item>
        </wa-tree-item>
    }
    .into_any()
}

/// Render a single field value as a highlighted `<span>`, applying
/// the `tonk-cm-*` decoration class that matches the value's
/// shape. Mirrors the notation formatter's value rules: URIs bare
/// and entity-tinted, strings quoted, numbers/bools/null plain.
///
/// The span is inline and wraps on overflow: rows are plain text
/// so a selection copied out of the result keeps its structure.
/// Multi-line strings are handled by the caller (each line gets
/// its own row), so by the time a string reaches here it is
/// single-line.
fn render_field_value(value: serde_json::Value) -> impl IntoView {
    use serde_json::Value;
    let (class, text) = match value {
        Value::Null => ("tonk-cm-variable", "_".to_owned()),
        Value::Bool(b) => ("tonk-cm-number", b.to_string()),
        Value::Number(n) => ("tonk-cm-number", n.to_string()),
        Value::String(s) => {
            if tonk_display::notation_format::looks_like_uri(&s) {
                ("tonk-cm-entity", s)
            } else {
                // Show the string verbatim (the string tint marks
                // it as text) rather than `\"`-escaping quotes.
                ("tonk-cm-string", s)
            }
        }
        // Arrays and objects have no notation form — show compact
        // JSON, undecorated.
        other => (
            "tonk-cm-plain",
            serde_json::to_string(&other).unwrap_or_else(|_| "<?>".to_owned()),
        ),
    };
    view! { <span class=class>{ text }</span> }
}

/// Github-style short form of a tree reference. `TreeReference`'s
/// `Display` produces `#<base58>`; this drops the `#` marker and
/// truncates the base58 body to 8 chars. Callers should expose
/// the full value via a `title` attribute for hover disclosure.
fn abbreviate_tree(tree: &str) -> String {
    const SHORT_LEN: usize = 8;
    let body = tree.strip_prefix('#').unwrap_or(tree);
    body.chars().take(SHORT_LEN).collect()
}

/// Render a single revision as the same `<wa-badge>` shape used
/// in the branch-row header (truncated tree hash with the full
/// hash exposed via `title`). `None` produces a "no commits"
/// fallback identical to the branch row's empty state.
fn revision_badge(revision: Option<Revision>) -> impl IntoView {
    match revision {
        Some(rev) => {
            let full = rev.tree.to_string();
            let short = abbreviate_tree(&full);
            Either::Left(view! {
                <wa-badge variant="neutral" appearance="filled" title=full>
                    <wa-icon name="code-commit" slot="start"></wa-icon>
                    { short }
                </wa-badge>
            })
        }
        None => Either::Right(view! {
            <wa-badge variant="neutral" appearance="filled">
                "no commits"
            </wa-badge>
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};
    use tonk_worker::QueryResult;

    use super::{concept_descriptor, rule_definition};

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a `rule:` result row whose `definition` field carries
    /// the JSON-stringified `RuleDefinition` an `AnonymousRuleQuery`
    /// emits — `{ "rule": <InductiveRuleDescriptor>, "polarity": … }`.
    fn rule_row(polarity: &str) -> QueryResult {
        let definition = json!({
            "rule": {
                "assert!": {
                    "with": {
                        "name": { "the": "person/name", "as": "Text" }
                    }
                },
                "when": [
                    {
                        "assert": {
                            "with": {
                                "name": { "the": "person-entered/name", "as": "Text" },
                                "age":  { "the": "person-entered/age",  "as": "UnsignedInteger" }
                            }
                        },
                        "where": {
                            "this": { "?": { "name": "this" } },
                            "name": { "?": { "name": "name" } },
                            "age":  { "?": { "name": "age" } }
                        }
                    }
                ]
            },
            "polarity": polarity
        });
        let mut fields = BTreeMap::new();
        fields.insert(
            "definition".to_owned(),
            Value::String(definition.to_string()),
        );
        QueryResult {
            this: "effect:E9vvYmyd".to_owned(),
            fields,
        }
    }

    #[dialog_common::test]
    fn it_projects_a_rule_row_into_rule_notation_fields() {
        let map = rule_definition(&rule_row("Assert")).expect("definition projects");

        // Assert polarity keeps the head as `assert!`.
        assert!(map.contains_key("assert!"), "head should be `assert!`");
        assert!(!map.contains_key("retract!"));

        // The `when` array surfaces at the top level.
        let when = map
            .get("when")
            .and_then(Value::as_array)
            .expect("when array");
        assert_eq!(when.len(), 1);

        // A premise's `where` bindings render variable terms as
        // `?name` strings, not nested `{"?": …}` objects.
        let where_map = when[0]
            .get("where")
            .and_then(Value::as_object)
            .expect("premise where map");
        assert_eq!(where_map.get("this"), Some(&json!("?this")));
        assert_eq!(where_map.get("name"), Some(&json!("?name")));
        assert_eq!(where_map.get("age"), Some(&json!("?age")));

        // `as` discriminants are normalized to the surface form.
        let head_with = map
            .get("assert!")
            .and_then(|h| h.get("with"))
            .and_then(Value::as_object)
            .expect("head with map");
        assert_eq!(
            head_with.get("name").and_then(|n| n.get("as")),
            Some(&json!("text")),
        );
    }

    #[dialog_common::test]
    fn it_renames_the_head_to_retract_for_retract_polarity() {
        let map = rule_definition(&rule_row("Retract")).expect("definition projects");
        assert!(
            map.contains_key("retract!"),
            "retract polarity should rename head to `retract!`",
        );
        assert!(!map.contains_key("assert!"));
    }

    #[dialog_common::test]
    fn it_returns_none_without_a_definition_field() {
        let row = QueryResult {
            this: "effect:none".to_owned(),
            fields: BTreeMap::new(),
        };
        assert!(rule_definition(&row).is_none());
    }

    /// Build a `concept:` result row with the given transient
    /// marker value. `source` carries the same canonical JSON
    /// `AnonymousConceptQuery` emits.
    fn concept_row(transient: Option<bool>) -> QueryResult {
        let source = json!({
            "with": {
                "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
            }
        });
        let mut fields = BTreeMap::new();
        fields.insert("source".to_owned(), Value::String(source.to_string()));
        if let Some(t) = transient {
            fields.insert("transient".to_owned(), Value::Bool(t));
        }
        QueryResult {
            this: "concept:abc".to_owned(),
            fields,
        }
    }

    /// `Bool(true)` on the row surfaces as `transient: true` in
    /// the rendered descriptor map.
    #[dialog_common::test]
    fn it_renders_transient_true_on_transient_concept_row() {
        let map = concept_descriptor(&concept_row(Some(true)), true).expect("descriptor projects");
        assert_eq!(map.get("transient"), Some(&Value::Bool(true)));
    }

    /// `Bool(false)` is the durable case — no `transient:` row
    /// appears (the notation convention is that absence means
    /// durable; affirmative is the only marker).
    #[dialog_common::test]
    fn it_omits_transient_on_durable_concept_row() {
        let map = concept_descriptor(&concept_row(Some(false)), true).expect("descriptor projects");
        assert!(
            !map.contains_key("transient"),
            "durable concepts must not surface a transient row",
        );
    }

    /// Missing `transient` binding (caller didn't ask for it)
    /// behaves the same as `Bool(false)`: no row.
    #[dialog_common::test]
    fn it_omits_transient_when_binding_absent() {
        let map = concept_descriptor(&concept_row(None), true).expect("descriptor projects");
        assert!(!map.contains_key("transient"));
    }

    /// A `command!:` rendering (`show_transient = false`) never
    /// surfaces the `transient:` row — the keyword already implies
    /// it — even when the row carries `Bool(true)`.
    #[dialog_common::test]
    fn it_omits_transient_row_on_command_render() {
        let map = concept_descriptor(&concept_row(Some(true)), false).expect("descriptor projects");
        assert!(
            !map.contains_key("transient"),
            "command rendering must not surface a redundant transient row",
        );
    }
}
