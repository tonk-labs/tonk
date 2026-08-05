//! Host-level event delegation.
//!
//! A `Delegate` owns per-event-type JS listeners installed on the
//! `<tonk-display>` host element. On fire, it walks up from
//! `event.target` to the closest `[data-on<event>]`-bearing
//! ancestor, looks up that attribute's value (the concept name),
//! resolves the cached descriptor, builds a `TransactRequest`
//! body via [`super::extract::build_transact_body`], and
//! dispatches a `tonk-claim` event on the host element. The host
//! routes the claim to `/transact` against the ambient
//! `(space, branch)` context.
//!
//! Listeners stay attached for the lifetime of the host element.
//! When the host's children re-render incrementally (existing
//! tonk-display behaviour), the delegation listener keeps working
//! because it lives on the host, not on the buttons.
//!
//! Descriptors for every distinct concept-name in the template
//! are resolved up-front at mount time, so the click handler is
//! synchronous — no async hop to fetch a schema on each click.

use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;
use tonk_host::consumer as host_consumer;
use tonk_schema::command_definition::CommandDefinition;
use tonk_schema::projection::{ProjectionDefinition, ProjectionError, project};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event};

use super::dom::{DomInput, apply_actions};
use super::legacy::build_transact_body;

/// Concept name → pre-parsed descriptor. Built once at mount time
/// from the worker's phase-1 results; each click reads from this
/// map directly, no per-click JSON parse.
pub type Descriptors = HashMap<String, Value>;

/// One nominal command plus the projections currently indexed to it.
#[derive(Debug, Clone)]
pub struct CommandBinding {
    /// Current authoritative command definition.
    pub command: CommandDefinition,
    /// Current projections for the command.
    pub projections: Vec<ProjectionDefinition>,
}

/// Event bindings resolved during the display's phase-1 refresh.
#[derive(Debug, Clone, Default)]
pub struct BindingsCatalog {
    /// Binding reference → nominal command and candidate projections.
    pub commands: HashMap<String, CommandBinding>,
    /// Binding reference → explicit nominal projection and its command.
    pub projections: HashMap<String, (ProjectionDefinition, CommandDefinition)>,
    /// Explicitly isolated structural compatibility descriptors.
    pub legacy_descriptors: Descriptors,
}

impl From<Descriptors> for BindingsCatalog {
    fn from(legacy_descriptors: Descriptors) -> Self {
        Self {
            legacy_descriptors,
            ..Self::default()
        }
    }
}

enum ResolvedBinding<'a> {
    Nominal {
        projection: &'a ProjectionDefinition,
        command: &'a CommandDefinition,
    },
    Legacy(&'a Value),
}

#[derive(Debug)]
enum BindingResolveError {
    Unresolved,
    MissingProjection,
    AmbiguousProjection,
}

impl BindingsCatalog {
    fn resolve(&self, reference: &str) -> Result<ResolvedBinding<'_>, BindingResolveError> {
        if let Some((projection, command)) = self.projections.get(reference) {
            return Ok(ResolvedBinding::Nominal {
                projection,
                command,
            });
        }
        if let Some(binding) = self.commands.get(reference) {
            let projection = match binding.projections.as_slice() {
                [projection] => projection,
                [] => return Err(BindingResolveError::MissingProjection),
                projections => {
                    let mut defaults = projections
                        .iter()
                        .filter(|projection| projection.descriptor().default);
                    let Some(default) = defaults.next() else {
                        return Err(BindingResolveError::AmbiguousProjection);
                    };
                    if defaults.next().is_some() {
                        return Err(BindingResolveError::AmbiguousProjection);
                    }
                    default
                }
            };
            return Ok(ResolvedBinding::Nominal {
                projection,
                command: &binding.command,
            });
        }
        self.legacy_descriptors
            .get(reference)
            .map(ResolvedBinding::Legacy)
            .ok_or(BindingResolveError::Unresolved)
    }
}

/// Per-listener pair: the event-type name and the JS-side closure
/// whose lifetime owns its memory.
type ListenerEntry = (String, Closure<dyn FnMut(Event)>);

/// One installed delegation listener on the host, paired with the
/// `Closure` that owns its JS-side memory.
pub struct Delegate {
    /// The host the listeners are attached to. We need it on
    /// drop to remove the listeners.
    host: Element,
    /// One `(event_type, closure)` per registered event type.
    /// Dropped on `Delegate::drop`, which also calls
    /// `removeEventListener`.
    listeners: Vec<ListenerEntry>,
}

impl Delegate {
    /// Install delegation listeners on `host` for every event
    /// type in `event_types`. `descriptors` maps a concept name to
    /// its pre-parsed descriptor (the caller parsed the worker's
    /// phase-1 JSON once at mount time so the click handler avoids
    /// the parse cost on every fire). Claims dispatch as `tonk-claim`
    /// events on `host`; the `<tonk-host>` ancestor routes them to
    /// `/transact` against the ambient `(space, branch)`.
    ///
    /// Returns a `Delegate` value whose `Drop` impl removes the
    /// listeners. Store it on the renderer's state so listeners
    /// outlive the renderer-managed children.
    pub fn install(
        host: Element,
        event_types: impl IntoIterator<Item = String>,
        catalog: impl Into<BindingsCatalog>,
    ) -> Self {
        let catalog = Rc::new(catalog.into());
        let mut listeners: Vec<ListenerEntry> = Vec::new();

        for event_type in event_types {
            let catalog = Rc::clone(&catalog);
            let attr_name = format!("data-on{event_type}");
            let host_for_handler = host.clone();
            let closure = Closure::wrap(Box::new(move |event: Event| {
                handle_event(&event, &attr_name, catalog.as_ref(), &host_for_handler);
            }) as Box<dyn FnMut(Event)>);
            let _ = host
                .add_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
            listeners.push((event_type, closure));
        }

        Self { host, listeners }
    }
}

impl Drop for Delegate {
    fn drop(&mut self) {
        for (event_type, closure) in self.listeners.drain(..) {
            let _ = self
                .host
                .remove_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
            // `closure` is moved out and dropped here; the
            // JS-side wrapper releases its references.
        }
    }
}

/// One event fire. Walk up from `event.target` collecting every
/// ancestor that carries the `data-on<event>` attribute (up to and
/// including the host). Try each in innermost-first order; the
/// first one that resolves to a complete, well-typed transact body
/// wins and posts. Bindings whose concept isn't in the descriptors
/// map or whose `dom.event*` fields fail to project fall through
/// to the next ancestor, so a typo or a missing `data-*` on the
/// inner binding doesn't swallow an outer binding's click. Action
/// side effects (`preventDefault`, `stopPropagation`) only fire for
/// the binding that wins, because `build_transact_body` queues them
/// and applies them only after the body is known-good.
fn handle_event(event: &Event, attr_name: &str, catalog: &BindingsCatalog, host: &Element) {
    let Some(body) = resolve_catalog_binding(event, attr_name, catalog, host) else {
        return;
    };
    let request_js = match serde_wasm_bindgen::to_value(&body) {
        Ok(v) => v,
        Err(e) => {
            log_error(format!("event handler: serialize body: {e}"));
            return;
        }
    };
    let host = host.clone();
    spawn_local(async move {
        if let Err(e) = host_consumer::claim(&host, &request_js).await {
            log_error(format!("event handler: tonk-claim: {}", e.message));
        }
    });
    maybe_dismiss_overlay(event);
}

/// Opt-in overlay dismissal on the winning binding's event. The delegate
/// only runs on events the browser actually dispatched, so on a `submit`
/// this fires solely once native constraint validation passed — closing
/// on a valid submit, never on an attempt a `required` field rejected.
/// `event.target()` is the `<form>` for a `submit` and the activated
/// element otherwise, so `closest` finds the marker either way. Two
/// markers, each a no-op unless present:
/// - `[data-close-dialog]` closes the element's nearest `<wa-dialog>`
///   (Web Awesome's `open` property → its animated close).
/// - `[data-close-radio="<id>"]` checks the radio with that id — used to
///   select the "closed" state of a CSS-radio-group overlay, which both
///   hides it and (since the other states deselect) resets its paging.
///   When the marked element is itself a `<form>`, its fields are also
///   reset so the next open starts blank.
fn maybe_dismiss_overlay(event: &Event) {
    let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
        return;
    };
    if let Some(marked) = target.closest("[data-close-dialog]").ok().flatten()
        && let Some(dialog) = marked.closest("wa-dialog").ok().flatten()
    {
        let _ = js_sys::Reflect::set(
            dialog.as_ref(),
            &wasm_bindgen::JsValue::from_str("open"),
            &wasm_bindgen::JsValue::FALSE,
        );
    }
    if let Some(marked) = target.closest("[data-close-radio]").ok().flatten()
        && let Some(id) = marked.get_attribute("data-close-radio")
        && let Some(doc) = marked.owner_document()
        && let Some(radio) = doc.get_element_by_id(&id)
    {
        let _ = js_sys::Reflect::set(
            radio.as_ref(),
            &wasm_bindgen::JsValue::from_str("checked"),
            &wasm_bindgen::JsValue::TRUE,
        );
        if let Some(form) = marked.dyn_ref::<web_sys::HtmlFormElement>() {
            form.reset();
        }
    }
}

fn resolve_catalog_binding(
    event: &Event,
    attr_name: &str,
    catalog: &BindingsCatalog,
    host: &Element,
) -> Option<serde_json::Value> {
    let target_el = event.target()?.dyn_ref::<Element>()?.clone();
    let selector = format!("[{attr_name}]");
    let mut cursor = Some(target_el);
    while let Some(current) = cursor {
        let bound = closest(&current, &selector)?;
        if !host.contains(Some(bound.unchecked_ref())) {
            return None;
        }
        let reference = bound.get_attribute(attr_name)?;
        match catalog.resolve(&reference) {
            Ok(ResolvedBinding::Nominal {
                projection,
                command,
            }) => match project(projection, command.schema(), &DomInput::new(event, &bound)) {
                Ok(result) => {
                    apply_actions(event, &result.actions);
                    let request = tonk_schema::claim::TransactRequest {
                        claims: vec![tonk_schema::claim::SourceClaim::Invoke(result.invocation)],
                    };
                    return match serde_json::to_value(request) {
                        Ok(body) => Some(body),
                        Err(error) => {
                            log_diagnostic(
                                "projection_serialize",
                                &reference,
                                Some(projection),
                                Some(command),
                                None,
                                &error.to_string(),
                            );
                            None
                        }
                    };
                }
                Err(error) => {
                    let (field, input) = projection_error_context(&error);
                    log_diagnostic(
                        "projection_failed",
                        &reference,
                        Some(projection),
                        Some(command),
                        field,
                        &format!("{error}; input={input}"),
                    );
                    return None;
                }
            },
            Ok(ResolvedBinding::Legacy(descriptor)) => {
                return try_legacy_binding(descriptor, &reference, event, &bound);
            }
            Err(error) => {
                log_diagnostic(
                    match error {
                        BindingResolveError::Unresolved => "binding_unresolved",
                        BindingResolveError::MissingProjection => "projection_missing",
                        BindingResolveError::AmbiguousProjection => "projection_ambiguous",
                    },
                    &reference,
                    None,
                    catalog
                        .commands
                        .get(&reference)
                        .map(|binding| &binding.command),
                    None,
                    "event binding could not resolve unambiguously",
                );
            }
        }
        cursor = bound.parent_element();
    }
    None
}

fn projection_error_context(error: &ProjectionError) -> (Option<&str>, &'static str) {
    match error {
        ProjectionError::ReadFailed { field, .. } => (Some(field), "read-failed"),
        ProjectionError::MissingRequired { field, .. } => (Some(field), "missing"),
        ProjectionError::InvalidInvocation { error, .. } => {
            let field = match error {
                tonk_core::command::CommandValidationError::UnknownArgument { field }
                | tonk_core::command::CommandValidationError::MissingRequiredArgument { field }
                | tonk_core::command::CommandValidationError::ReservedArgument { field }
                | tonk_core::command::CommandValidationError::TypeMismatch { field, .. } => field,
            };
            (Some(field), "invalid")
        }
    }
}

fn log_diagnostic(
    code: &str,
    reference: &str,
    projection: Option<&ProjectionDefinition>,
    command: Option<&CommandDefinition>,
    field: Option<&str>,
    message: &str,
) {
    let diagnostic = serde_json::json!({
        "code": code,
        "binding": reference,
        "projection": projection.map(|projection| projection.this().to_string()),
        "command": command.map(|command| command.kind().to_string()),
        "field": field,
        "message": message,
    });
    web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&diagnostic.to_string()));
}

/// Walk ancestors of `event.target` in innermost-first order,
/// trying each one that matches `[data-on<event>]`. Returns the
/// first transact body that builds cleanly. Skips bindings whose
/// concept isn't in `descriptors`, whose descriptor can't build a
/// body, or whose `dom.event*` fields fail to project. Stops once
/// `closest` walks past `host`.
///
/// Action side effects only fire for the binding that wins, because
/// `build_transact_body` queues actions and applies them only after
/// the body is known-good.
#[cfg(test)]
pub(super) fn resolve_actionable_binding(
    event: &Event,
    attr_name: &str,
    descriptors: &Descriptors,
    host: &Element,
) -> Option<serde_json::Value> {
    let target_el = event.target()?.dyn_ref::<Element>()?.clone();
    let selector = format!("[{attr_name}]");

    let mut cursor: Option<Element> = Some(target_el);
    while let Some(current) = cursor {
        let bound = closest(&current, &selector)?;
        // Bindings outside the host belong to a different view or
        // to nothing at all — `Node::contains` includes equality,
        // so the host itself counts as in-scope.
        if !host.contains(Some(bound.unchecked_ref())) {
            return None;
        }
        if let Some(body) = try_binding(attr_name, descriptors, event, &bound) {
            return Some(body);
        }
        // Move up: next iteration's `closest` starts from `bound`'s
        // parent so we skip past the binding we just tried (and
        // don't get the same answer again).
        cursor = bound.parent_element();
    }
    None
}

#[cfg(test)]
fn try_binding(
    attr_name: &str,
    descriptors: &Descriptors,
    event: &Event,
    bound: &Element,
) -> Option<serde_json::Value> {
    let concept = bound.get_attribute(attr_name)?;
    let descriptor = descriptors.get(&concept)?;
    try_legacy_binding(descriptor, &concept, event, bound)
}

fn try_legacy_binding(
    descriptor: &Value,
    concept: &str,
    event: &Event,
    bound: &Element,
) -> Option<serde_json::Value> {
    // The wire body omits `this:` unless the descriptor itself
    // populates the slot from an event field. The worker derives
    // an absent `this:` from `(predicate, parameters)` so each
    // event-derived assertion gets a distinct, content-addressed
    // subject entity.
    match build_transact_body(descriptor, concept, event, bound) {
        Ok(built) => {
            // Blank form fields are omitted, not fatal — but a rule
            // premise naming one will silently match nothing, which
            // is indistinguishable from "the event didn't fire"
            // without this breadcrumb.
            if !built.blank_fields.is_empty() {
                log_error(format!(
                    "event handler: {concept}: blank fields omitted: {} — the command still \
                     fired without them; a rule premise naming them will not match",
                    built.blank_fields.join(", "),
                ));
            }
            Some(built.body)
        }
        Err(e) => {
            log_error(format!("event handler: build body for {concept}: {e}"));
            None
        }
    }
}

/// `Element.closest(selector)` — walks up the parent chain until
/// it finds an element matching `selector`, or returns `None`.
fn closest(start: &Element, selector: &str) -> Option<Element> {
    start.closest(selector).ok().flatten()
}

fn log_error(message: String) {
    web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&message));
}

/// Resolve one authored event-binding reference into its nominal command or
/// projection catalog entry, falling back to a legacy concept only when no
/// nominal definition exists.
pub async fn load_binding(
    host: &Element,
    reference: &str,
) -> Result<BindingsCatalog, tonk_host::error::ErrorDetail> {
    let entity = resolve_binding_entity(host, reference).await?;
    let mut catalog = BindingsCatalog::default();

    if let Some(projection) = load_projection(host, &entity).await? {
        let command_kind = projection.descriptor().command.to_string();
        if let Some(command) = load_command(host, &command_kind).await? {
            catalog
                .projections
                .insert(reference.to_owned(), (projection, command));
            return Ok(catalog);
        }
    }

    if let Some(command) = load_command(host, &entity).await? {
        let projections = load_projections_for_command(host, command.kind()).await?;
        catalog.commands.insert(
            reference.to_owned(),
            CommandBinding {
                command,
                projections,
            },
        );
        return Ok(catalog);
    }

    if let Some(descriptor) = load_legacy_descriptor(host, &entity).await? {
        catalog
            .legacy_descriptors
            .insert(reference.to_owned(), descriptor);
    }
    Ok(catalog)
}

async fn resolve_binding_entity(
    host: &Element,
    reference: &str,
) -> Result<String, tonk_host::error::ErrorDetail> {
    if tonk_template::resolve::looks_like_uri(reference) {
        return Ok(reference.to_owned());
    }
    let rows = run_query(host, tonk_template::resolve::name_query(reference)).await?;
    Ok(rows
        .first()
        .and_then(|row| ipld_text(row.fields.get("entity")))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("id:{reference}")))
}

async fn load_command(
    host: &Element,
    kind: &str,
) -> Result<Option<CommandDefinition>, tonk_host::error::ErrorDetail> {
    let rows = run_query(
        host,
        one_attribute_query(kind, "schema", "dialog.command/schema", "Entity"),
    )
    .await?;
    let Some(schema_entity) = rows
        .first()
        .and_then(|row| ipld_text(row.fields.get("schema")))
    else {
        return Ok(None);
    };
    let rows = run_query(
        host,
        one_attribute_query(schema_entity, "source", "dialog.command/source", "Text"),
    )
    .await?;
    let Some(source) = rows
        .first()
        .and_then(|row| ipld_text(row.fields.get("source")))
    else {
        return Ok(None);
    };
    let schema = serde_ipld_dagjson::from_slice(source.as_bytes()).map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Descriptor,
            format!("command {kind} schema: {error}"),
        )
    })?;
    let kind = kind.parse().map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Descriptor,
            format!("command kind {kind}: {error}"),
        )
    })?;
    Ok(Some(CommandDefinition::asserting(kind, schema)))
}

async fn load_projection(
    host: &Element,
    entity: &str,
) -> Result<Option<ProjectionDefinition>, tonk_host::error::ErrorDetail> {
    let rows = run_query(
        host,
        one_attribute_query(entity, "source", "dialog.projection/source", "Text"),
    )
    .await?;
    let Some(source) = rows
        .first()
        .and_then(|row| ipld_text(row.fields.get("source")))
    else {
        return Ok(None);
    };
    let descriptor = serde_ipld_dagjson::from_slice(source.as_bytes()).map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Descriptor,
            format!("projection {entity}: {error}"),
        )
    })?;
    let entity = entity.parse().map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Descriptor,
            format!("projection entity {entity}: {error}"),
        )
    })?;
    Ok(Some(ProjectionDefinition::asserting(entity, descriptor)))
}

async fn load_projections_for_command(
    host: &Element,
    command: &dialog_artifacts::Entity,
) -> Result<Vec<ProjectionDefinition>, tonk_host::error::ErrorDetail> {
    let query: tonk_schema::query::Query = serde_json::from_value(serde_json::json!({
        "terms": {
            "this": { "?": { "name": "this" } },
            "command": command.to_string(),
        },
        "predicate": {
            "with": {
                "command": {
                    "the": "dialog.projection/command",
                    "as": "Entity",
                    "cardinality": "one"
                }
            }
        }
    }))
    .expect("projection index query is well formed");
    let rows = run_query(host, query).await?;
    let mut projections = Vec::new();
    for row in rows {
        if let Some(projection) = load_projection(host, &row.this).await? {
            projections.push(projection);
        }
    }
    Ok(projections)
}

async fn load_legacy_descriptor(
    host: &Element,
    reference: &str,
) -> Result<Option<Value>, tonk_host::error::ErrorDetail> {
    let parsed = tonk_template::resolve::parse_source(reference);
    let result = host_consumer::query(
        host,
        &serde_wasm_bindgen::to_value(&tonk_template::resolve::phase1_query(&parsed)).map_err(
            |error| {
                tonk_host::error::ErrorDetail::new(
                    tonk_host::error::ErrorKind::Parse,
                    format!("legacy descriptor query: {error}"),
                )
            },
        )?,
    )
    .await?;
    let rows: Vec<tonk_schema::conclusion::Conclusion> = serde_wasm_bindgen::from_value(result)
        .map_err(|error| {
            tonk_host::error::ErrorDetail::new(
                tonk_host::error::ErrorKind::Parse,
                format!("legacy descriptor result: {error}"),
            )
        })?;
    rows.first()
        .and_then(|row| ipld_text(row.fields.get("source")))
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            tonk_host::error::ErrorDetail::new(
                tonk_host::error::ErrorKind::Descriptor,
                format!("legacy descriptor {reference}: {error}"),
            )
        })
}

fn one_attribute_query(
    this: &str,
    field: &str,
    attribute: &str,
    value_type: &str,
) -> tonk_schema::query::Query {
    let mut terms = serde_json::Map::new();
    terms.insert("this".into(), serde_json::json!(this));
    terms.insert(field.into(), serde_json::json!({ "?": { "name": field } }));
    let mut with = serde_json::Map::new();
    with.insert(
        field.into(),
        serde_json::json!({
            "the": attribute,
            "as": value_type,
            "cardinality": "one"
        }),
    );
    serde_json::from_value(serde_json::json!({
        "terms": terms,
        "predicate": { "with": with }
    }))
    .expect("single-attribute query is well formed")
}

async fn run_query(
    host: &Element,
    query: tonk_schema::query::Query,
) -> Result<Vec<tonk_schema::conclusion::Conclusion>, tonk_host::error::ErrorDetail> {
    let body = serde_wasm_bindgen::to_value(&query).map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Parse,
            format!("binding query: {error}"),
        )
    })?;
    let result = host_consumer::query(host, &body).await?;
    serde_wasm_bindgen::from_value(result).map_err(|error| {
        tonk_host::error::ErrorDetail::new(
            tonk_host::error::ErrorKind::Parse,
            format!("binding query result: {error}"),
        )
    })
}

fn ipld_text(value: Option<&ipld_core::ipld::Ipld>) -> Option<&str> {
    match value? {
        ipld_core::ipld::Ipld::String(value) => Some(value),
        _ => None,
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Object, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Event, EventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    fn command(kind: &str) -> CommandDefinition {
        CommandDefinition::asserting(kind.parse().unwrap(), Default::default())
    }

    fn text_command(kind: &str, field: &str) -> CommandDefinition {
        let mut required = serde_json::Map::new();
        required.insert(
            field.into(),
            serde_json::json!({
                "the": format!("xyz.tonk.test/{field}"),
                "as": "String",
                "cardinality": "one"
            }),
        );
        let schema = serde_json::from_value(serde_json::json!({
            "required": required,
            "optional": {}
        }))
        .unwrap();
        CommandDefinition::asserting(kind.parse().unwrap(), schema)
    }

    fn projection(this: &str, kind: &str, default: bool) -> ProjectionDefinition {
        ProjectionDefinition::asserting(
            this.parse().unwrap(),
            tonk_schema::projection::ProjectionDescriptor {
                command: kind.parse().unwrap(),
                default,
                arguments: Default::default(),
                actions: Vec::new(),
            },
        )
    }

    #[dialog_common::test]
    fn catalog_resolves_explicit_projection_then_command_default() {
        let kind = "id:test/save";
        let explicit = projection("id:test/save-form", kind, false);
        let default = projection("id:test/save-default", kind, true);
        let alternate = projection("id:test/save-alternate", kind, false);
        let mut catalog = BindingsCatalog::default();
        catalog
            .projections
            .insert("save-form".into(), (explicit.clone(), command(kind)));
        catalog.commands.insert(
            "save".into(),
            CommandBinding {
                command: command(kind),
                projections: vec![alternate, default.clone()],
            },
        );

        assert!(matches!(
            catalog.resolve("save-form"),
            Ok(ResolvedBinding::Nominal { projection, .. }) if projection.this() == explicit.this()
        ));
        assert!(matches!(
            catalog.resolve("save"),
            Ok(ResolvedBinding::Nominal { projection, .. }) if projection.this() == default.this()
        ));
    }

    #[dialog_common::test]
    fn catalog_uses_the_only_projection_and_rejects_ambiguity() {
        let kind = "id:test/save";
        let only = projection("id:test/only", kind, false);
        let mut catalog = BindingsCatalog::default();
        catalog.commands.insert(
            "only".into(),
            CommandBinding {
                command: command(kind),
                projections: vec![only.clone()],
            },
        );
        catalog.commands.insert(
            "ambiguous".into(),
            CommandBinding {
                command: command(kind),
                projections: vec![
                    projection("id:test/a", kind, false),
                    projection("id:test/b", kind, false),
                ],
            },
        );

        assert!(matches!(
            catalog.resolve("only"),
            Ok(ResolvedBinding::Nominal { projection, .. }) if projection.this() == only.this()
        ));
        assert!(matches!(
            catalog.resolve("ambiguous"),
            Err(BindingResolveError::AmbiguousProjection)
        ));
    }

    #[dialog_common::test]
    fn catalog_falls_back_only_to_an_explicit_legacy_descriptor() {
        let mut catalog = BindingsCatalog::default();
        catalog
            .legacy_descriptors
            .insert("legacy".into(), serde_json::json!({ "with": {} }));
        catalog.commands.insert(
            "nominal-without-projection".into(),
            CommandBinding {
                command: command("id:test/nominal"),
                projections: Vec::new(),
            },
        );
        catalog.legacy_descriptors.insert(
            "nominal-without-projection".into(),
            serde_json::json!({ "with": {} }),
        );

        assert!(matches!(
            catalog.resolve("legacy"),
            Ok(ResolvedBinding::Legacy(_))
        ));
        assert!(matches!(
            catalog.resolve("nominal-without-projection"),
            Err(BindingResolveError::MissingProjection)
        ));
        assert!(matches!(
            catalog.resolve("unknown"),
            Err(BindingResolveError::Unresolved)
        ));
    }

    /// Build a click event whose `event.target` is `target_el`.
    /// Done by defining an own `target` property on the event JS
    /// object — `Reflect::get` and the web-sys getter both check
    /// own props before falling through to the readonly getter
    /// inherited from `Event.prototype`.
    fn click_event_targeting(target_el: &Element) -> Event {
        let event = Event::new_with_event_init_dict("click", &EventInit::new()).expect("Event");
        let event_js: &JsValue = event.as_ref();
        let descriptor = Object::new();
        Reflect::set(&descriptor, &JsValue::from_str("value"), target_el.as_ref()).unwrap();
        Reflect::set(
            &descriptor,
            &JsValue::from_str("configurable"),
            &JsValue::TRUE,
        )
        .unwrap();
        Reflect::set(&descriptor, &JsValue::from_str("writable"), &JsValue::TRUE).unwrap();
        Reflect::set(
            &descriptor,
            &JsValue::from_str("enumerable"),
            &JsValue::TRUE,
        )
        .unwrap();
        let _ = Object::define_property(
            event_js.unchecked_ref::<Object>(),
            &JsValue::from_str("target"),
            &descriptor,
        );
        event
    }

    /// Mount a fresh host div under `<body>` and append `markup`
    /// inside. The host is the element listeners would normally
    /// be installed on; here we just need it as the boundary the
    /// ancestor walk respects.
    fn mount(markup: &str) -> Element {
        let document = window().expect("window").document().expect("document");
        let host = document.create_element("div").expect("create host");
        host.set_inner_html(markup);
        document.body().expect("body").append_child(&host).unwrap();
        host
    }

    #[dialog_common::test]
    fn nominal_submit_prevents_default_and_posts_invoke_with_blank_text() {
        let host = mount(r#"<form data-onsubmit="add-form"><input name="title" value=""></form>"#);
        let form = host.query_selector("form").unwrap().unwrap();
        let kind = "id:todo/add";
        let command = text_command(kind, "title");
        let projection = ProjectionDefinition::asserting(
            "id:todo/add-form".parse().unwrap(),
            tonk_schema::projection::ProjectionDescriptor {
                command: kind.parse().unwrap(),
                default: true,
                arguments: [(
                    "title".into(),
                    tonk_schema::projection::ProjectionSource::Control(
                        tonk_schema::projection::ControlSource {
                            name: "title".into(),
                            property: tonk_schema::projection::ControlProperty::Value,
                        },
                    ),
                )]
                .into_iter()
                .collect(),
                actions: vec![tonk_schema::projection::EventAction::PreventDefault],
            },
        );
        let mut catalog = BindingsCatalog::default();
        catalog
            .projections
            .insert("add-form".into(), (projection, command));
        let init = EventInit::new();
        init.set_cancelable(true);
        let event = Event::new_with_event_init_dict("submit", &init).unwrap();
        let event_js: &JsValue = event.as_ref();
        Reflect::set(event_js, &JsValue::from_str("target"), form.as_ref()).unwrap();

        let body = resolve_catalog_binding(&event, "data-onsubmit", &catalog, &host)
            .expect("nominal projection succeeds");
        assert!(event.default_prevented());
        assert_eq!(body["claims"][0]["op"], serde_json::json!("invoke"));
        assert_eq!(body["claims"][0]["command"], serde_json::json!(kind));
        assert_eq!(
            body["claims"][0]["arguments"]["title"],
            serde_json::json!("")
        );
        assert!(body["claims"][0]["arguments"].get("this").is_none());
    }

    /// Build a [`Descriptors`] map from `(concept_name, json_text)`
    /// pairs, parsing each descriptor once just like `element.rs`
    /// does at mount.
    fn descriptors(pairs: &[(&str, &str)]) -> Descriptors {
        let mut out: Descriptors = HashMap::new();
        for (name, json_text) in pairs {
            let value: Value = serde_json::from_str(json_text).expect("descriptor parses");
            out.insert((*name).to_owned(), value);
        }
        out
    }

    #[dialog_common::test]
    fn it_falls_through_when_inner_concept_is_unknown() {
        // Outer binding is the only one in `descriptors`. Inner has
        // a `data-onclick` referencing a concept we never resolved
        // at mount time; the click should still reach outer.
        let host = mount(
            r#"<div data-onclick="outer" data-counter="did:key:zOuter">
                 <span data-onclick="unknown">click</span>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        let descriptors = descriptors(&[(
            "outer",
            r#"{ "with": {
                "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
            } }"#,
        )]);
        let body = resolve_actionable_binding(&event, "data-onclick", &descriptors, &host)
            .expect("outer should resolve");
        assert_eq!(
            body["claims"][0]["application"]["parameters"]["counter"],
            serde_json::json!("did:key:zOuter"),
        );
    }

    #[dialog_common::test]
    fn it_falls_through_when_inner_field_does_not_resolve() {
        // Inner concept *is* known, but its descriptor requires a
        // `data-todo` attribute the inner element doesn't carry.
        // Outer is well-formed; outer should win.
        let host = mount(
            r#"<div data-onclick="outer" data-counter="did:key:zOuter">
                 <span data-onclick="inner">click</span>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        let descriptors = descriptors(&[
            (
                "inner",
                r#"{ "with": {
                    "todo": { "the": "dom.event.current-target.dataset/todo", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
            (
                "outer",
                r#"{ "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
        ]);
        let body = resolve_actionable_binding(&event, "data-onclick", &descriptors, &host)
            .expect("outer should resolve after inner unresolved-field");
        let params = &body["claims"][0]["application"]["parameters"];
        assert!(params.get("todo").is_none(), "inner field must not leak");
        assert_eq!(params["counter"], serde_json::json!("did:key:zOuter"));
    }

    #[dialog_common::test]
    fn it_picks_innermost_when_both_resolve() {
        // Inner and outer both have well-formed descriptors with
        // their required data-* present. Inner should win.
        let host = mount(
            r#"<div data-onclick="outer" data-counter="did:key:zOuter">
                 <span data-onclick="inner" data-todo="did:key:zTodo">click</span>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        let descriptors = descriptors(&[
            (
                "inner",
                r#"{ "with": {
                    "todo": { "the": "dom.event.current-target.dataset/todo", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
            (
                "outer",
                r#"{ "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
        ]);
        let body = resolve_actionable_binding(&event, "data-onclick", &descriptors, &host)
            .expect("inner should resolve");
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(params["todo"], serde_json::json!("did:key:zTodo"));
        assert!(
            params.get("counter").is_none(),
            "outer must not be consulted when inner wins",
        );
    }

    #[dialog_common::test]
    fn it_returns_none_when_no_ancestor_resolves() {
        let host = mount(
            r#"<div data-onclick="outer">
                 <span data-onclick="inner">click</span>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        // Inner needs a data-todo it doesn't have; outer needs a
        // data-counter it doesn't have. Neither can resolve.
        let descriptors = descriptors(&[
            (
                "inner",
                r#"{ "with": {
                    "todo": { "the": "dom.event.current-target.dataset/todo", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
            (
                "outer",
                r#"{ "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
        ]);
        assert!(
            resolve_actionable_binding(&event, "data-onclick", &descriptors, &host).is_none(),
            "no binding should resolve, fallthrough exhausts ancestors",
        );
    }

    #[dialog_common::test]
    fn it_falls_through_three_levels_with_each_binding_carrying_real_data() {
        // The shape that prompted this test:
        //   <div data-onclick=outer data-subject=...>
        //     <div data-onclick=inner data-subject=...>
        //       <span>click me</span>
        //     </div>
        //   </div>
        //
        // Both handler elements carry a real `data-subject`. Inner's
        // descriptor still fails because it requires a different
        // field (`ready`) which the inner element doesn't carry.
        // Outer's descriptor reads `subject` off its own element and
        // resolves cleanly.
        //
        // Exercises two things the simpler tests didn't:
        //  1. The click target is a non-handler descendant (`<span>`),
        //     so `closest` has to walk past it before finding inner.
        //  2. Inner *has* data on it (just not the field the
        //     descriptor wants), so the failure is a missing required
        //     field rather than a wholly-absent dataset.
        let host = mount(
            r#"<div data-onclick="report-outer" data-subject="did:key:zOuter">
                 <div data-onclick="report-inner" data-subject="did:key:zHeader">
                   <span>click me</span>
                 </div>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        let descriptors = descriptors(&[
            (
                "report-inner",
                r#"{ "with": {
                    "subject": { "the": "dom.event.current-target.dataset/subject", "as": "Entity", "cardinality": "one" },
                    "ready":   { "the": "dom.event.current-target.dataset/ready",   "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
            (
                "report-outer",
                r#"{ "with": {
                    "subject": { "the": "dom.event.current-target.dataset/subject", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
        ]);
        let body = resolve_actionable_binding(&event, "data-onclick", &descriptors, &host)
            .expect("outer should resolve after inner falls through on missing `ready`");
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(
            params["subject"],
            serde_json::json!("did:key:zOuter"),
            "outer should read its own data-subject, not inner's",
        );
        assert!(
            params.get("ready").is_none(),
            "inner's unresolved field must not leak into the posted body",
        );
    }

    #[dialog_common::test]
    fn it_does_not_apply_actions_for_a_failed_binding() {
        // Inner has both an Action (`stop-propagation`) and a Read
        // field that won't resolve. The action must NOT fire,
        // because the body fails to build and we fall through. We
        // verify by checking that the event isn't marked as
        // "propagation stopped" — `cancelBubble` reflects
        // stopPropagation() having been called.
        let host = mount(
            r#"<div data-onclick="outer" data-counter="did:key:zOuter">
                 <span data-onclick="inner">click</span>
               </div>"#,
        );
        let target = host.query_selector("span").unwrap().expect("span");
        let event = click_event_targeting(&target);
        let descriptors = descriptors(&[
            (
                "inner",
                r#"{ "with": {
                    "todo": { "the": "dom.event.current-target.dataset/todo", "as": "Entity", "cardinality": "one" },
                    "stop": { "the": "dom.event.do/stop-propagation" }
                } }"#,
            ),
            (
                "outer",
                r#"{ "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                } }"#,
            ),
        ]);
        let _ = resolve_actionable_binding(&event, "data-onclick", &descriptors, &host)
            .expect("outer should resolve");
        // `Event::cancel_bubble` returns true iff stopPropagation
        // has been called on this event. Inner's action queued but
        // we bailed before applying it.
        assert!(
            !event.cancel_bubble(),
            "stopPropagation must not fire when the binding falls through",
        );
    }
}
