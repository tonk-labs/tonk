//! Publishing live reactor state for the `/console` page.
//!
//! The reactor knows which queries are subscribed and who is listening, but
//! that lives in process memory where no view can reach it. This module
//! bridges the two: it takes a snapshot
//! ([`subscription_snapshot`](dialog_reactor::Reactor::subscription_snapshot))
//! and publishes it into the profile branch's session overlay as
//! [`ConsoleSubscription`] facts, which the console's `<tonk-display>`
//! subscribes to like any other model.
//!
//! Overlay, never a commit — for three reasons, each on its own sufficient.
//! The facts describe one worker process, so committing them would replicate
//! one device's memory to every other. They change constantly, so committing
//! would write a commit per refresh. And the overlay is folded into standing
//! subscriptions, so publishing is what makes the page update rather than
//! sit still.
//!
//! ## The observer effect
//!
//! Publishing subscription rows onto a branch wakes that branch's
//! subscriptions — including the console's own, whose display is subscribed
//! to those very rows. Left alone that is a loop: publish, poll, re-render,
//! publish. It is broken by *what* gets published rather than by suppressing
//! polls: the rows are a pure function of the reactor's current state, so
//! republishing an unchanged state writes identical cardinality-one values
//! onto identical entities, the query results do not change, and the poll
//! pushes no frame. The loop settles after one round.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_reactor::SubscriptionSnapshot;

/// The branch console facts are published onto — the profile's main branch,
/// which is where the `/console` route itself resolves.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const PROFILE_BRANCH: &str = "main";

/// Entity prefix every console row carries. Both the "replace the previous
/// refresh" sweep and the "is the console open" check key on it, so it is
/// named once rather than spelled at each.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const CONSOLE_ENTITY_PREFIX: &str = "console:";

/// Publish the reactor's current subscriptions onto the profile branch's
/// overlay, replacing whatever was published before.
///
/// Called when the console page is loaded, so a visit renders current state
/// rather than whatever was last published — which for a page nobody has
/// opened is nothing at all. Cheap enough to run per navigation: it walks
/// in-memory maps and writes a handful of overlay facts.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn publish_subscriptions(tonk: &crate::worker::TonkState) {
    use std::sync::Arc;

    use dialog_common::log;
    use tonk_schema::domain::console_group;
    use tonk_schema::domain::console_subscription::{
        Branch, ConceptName, Group, Hash, LastUpdate, OpenedAt, Pending, Query, Space, Subscribers,
        Updates,
    };
    use tonk_schema::domain::console_update;
    use tonk_schema::{
        ConsoleGroup, ConsoleSubscription, ConsoleSubscriptionUpdate, ConsoleUpdate,
    };

    let snapshot = tonk.reactor.subscription_snapshot();

    let session = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(error) => {
            log!("console: failed to acquire the profile branch: {error}");
            return;
        }
    };

    // Drop the rows published by the previous refresh before writing this
    // one. Cardinality-one fields supersede in place, so a subscription that
    // still exists is simply overwritten — but one that has since CLOSED
    // would otherwise linger on the page forever, since nothing overwrites a
    // row that is no longer in the snapshot. Scoped to console entities by
    // prefix so this never disturbs the site stamps, sync status, or
    // anything else sharing the overlay.
    session
        .state
        .retain_overlay_entities(|entity| !entity.as_str().starts_with(CONSOLE_ENTITY_PREFIX));

    let mut published = 0usize;
    let mut groups: Vec<(String, String, u64)> = Vec::new();

    for row in &snapshot {
        let Some(entity) = ConsoleSubscription::entity_for(&row.repository, &row.branch, &row.hash)
        else {
            continue;
        };
        let Some(group) = ConsoleGroup::entity_for(&row.repository, &row.branch) else {
            continue;
        };

        session.state.assert_overlay(ConsoleSubscription {
            this: entity.clone(),
            space: Space(row.repository.clone()),
            branch: Branch(row.branch.clone()),
            query: Query(render_query(row)),
            subscribers: Subscribers(row.subscribers as u64),
            pending: Pending(row.pending as u64),
            hash: Hash(row.hash.clone()),
            opened_at: OpenedAt(iso_8601(row.opened_at_ms)),
            group: Group(group.clone()),
            updates: Updates(row.updates),
            concept_name: ConceptName(concept_name(row)),
        });

        // The update log, one row per entry. Keyed by position, so as the
        // window shifts each slot is overwritten in place rather than
        // accumulating a row per update ever delivered.
        for (position, record) in row.update_log.iter().enumerate() {
            let Some(update) = ConsoleUpdate::entity_for(&row.hash, position) else {
                continue;
            };
            // The subscription → update back-link, as a raw claim: it hangs
            // one attribute on the SUBSCRIPTION rather than describing the
            // update. `unique: false` — cardinality MANY, so each entry adds
            // its own fact instead of superseding the last.
            if let Ok(attribute) = "xyz.tonk.console.subscription/update".parse() {
                session
                    .state
                    .assert_overlay(crate::router::claim::RawClaim {
                        the: attribute,
                        of: entity.clone(),
                        is: dialog_artifacts::Value::Entity(update.clone()),
                        unique: false,
                    });
            }
            session.state.assert_overlay(ConsoleUpdate {
                this: update,
                subscription: console_update::Subscription(entity.clone()),
                at: console_update::At(iso_8601(record.at_ms)),
                bytes: console_update::Bytes(record.bytes as u64),
                position: console_update::Position(position as u64),
            });
        }

        // The last-update stamp is its own optional fact: a subscription
        // that has never changed simply has none, and the row still renders.
        if let Some(last_update_ms) = row.last_update_ms {
            session.state.assert_overlay(ConsoleSubscriptionUpdate {
                this: entity.clone(),
                last_update: LastUpdate(iso_8601(last_update_ms)),
            });
        }

        // The group → member link, as a raw claim: it hangs one attribute on
        // the GROUP's entity rather than describing the subscription, so it
        // is not a field of either concept struct.
        //
        // `unique: false` — cardinality MANY. Each member contributes its own
        // fact; a unique claim would have every member supersede the last and
        // leave each group showing exactly one child.
        if let Ok(attribute) = "xyz.tonk.console.group/subscription".parse() {
            session
                .state
                .assert_overlay(crate::router::claim::RawClaim {
                    the: attribute,
                    of: group.clone(),
                    is: dialog_artifacts::Value::Entity(entity),
                    unique: false,
                });
        }

        // Tally per group. The snapshot is sorted by (repository, branch),
        // so members of a group are adjacent and the last entry is the one
        // to bump.
        match groups.last_mut() {
            Some((repository, branch, count))
                if repository == &row.repository && branch == &row.branch =>
            {
                *count += 1;
            }
            _ => groups.push((row.repository.clone(), row.branch.clone(), 1)),
        }

        published += 1;
    }

    for (repository, branch, count) in &groups {
        let Some(entity) = ConsoleGroup::entity_for(repository, branch) else {
            continue;
        };
        session.state.assert_overlay(ConsoleGroup {
            this: entity,
            space: console_group::Space(repository.clone()),
            branch: console_group::Branch(branch.clone()),
            subscriptions: console_group::Subscriptions(*count),
        });
    }

    log!(
        "console: published {published} subscription row(s) in {} group(s)",
        groups.len()
    );

    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Republish console rows, but only if the console is currently open.
///
/// Called from the subscription lifecycle — the moments the console's
/// contents actually change — so the page updates itself instead of showing
/// whatever was true when it loaded.
///
/// "Open" is decided by the branch, not by tracking page visits: if console
/// rows are on the overlay, a console page put them there. That makes the
/// check one map scan, and makes closing the page stop the work on its own
/// (the rows go with the client's overlay facts, or the worker restart).
///
/// The self-limiting part matters, since this sits on a hot path. With no
/// console open there is nothing to republish and it returns immediately.
/// With one open, republishing an unchanged reactor writes identical
/// cardinality-one values, so the poll finds no change and pushes no frame —
/// the loop of "publish wakes the console's own subscription, which
/// publishes" settles after one round rather than spinning.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn refresh_if_open(tonk: &crate::worker::TonkState) {
    let Ok(session) = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        return;
    };

    // Read the overlay's entities through `retain_overlay_entities`, keeping
    // every one. The overlay exposes no read-only iterator (it lives in the
    // pinned dialog crate), but its retain closure is `FnMut` and visits each
    // entity — so returning `true` throughout makes this a pure observation.
    let mut console_is_open = false;
    session.state.retain_overlay_entities(|entity| {
        if entity.as_str().starts_with(CONSOLE_ENTITY_PREFIX) {
            console_is_open = true;
        }
        true
    });

    if console_is_open {
        publish_subscriptions(tonk).await;
    }
}

/// The readable concept name a subscription watches — the collapsed row's
/// label, e.g. `tonk:site`.
///
/// Derived from the query rather than passed down: a `<tonk-display
/// model=…>` resolves its model to a query before subscribing, and only the
/// query crosses the bridge. The attribute namespace its fields share is
/// what identifies the concept on the wire.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn concept_name(row: &SubscriptionSnapshot) -> String {
    match serde_json::to_value(&row.query) {
        Ok(json) => query_head(&json),
        Err(_) => "concept".to_owned(),
    }
}

/// Render a snapshot's query as dialog-yaml notation.
///
/// Notation rather than JSON because that is the language the rest of the
/// system speaks: the same shape the inspector shows, the editor edits, and
/// a library is written in. The console hands this text to
/// `<tonk-notation>`, which tokenizes and highlights it with the same
/// palette as the editor — so a reader recognises a query here the way they
/// would anywhere else.
///
/// The output mirrors what someone would *write* to ask the same question:
///
/// ```text
/// tonk:site!:
///   this: site:f4a6…
///   path: ?path
/// ```
///
/// The head is the concept (or formula) being selected; each field is either
/// a bound value or a `?variable` standing for what the query returns.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn render_query(row: &SubscriptionSnapshot) -> String {
    match serde_json::to_value(&row.query) {
        Ok(json) => render_query_json(&json),
        Err(error) => format!("# unserializable query: {error}"),
    }
}

/// Render an already-serialized wire query as notation. Split from
/// [`render_query`] so the shape can be tested without fabricating a
/// reactor subscription.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn render_query_json(json: &serde_json::Value) -> String {
    // No `!` on the head: the trailing bang is notation's "this is a
    // mutation" marker (it is what makes `concept!:` an assertion rather than
    // a query). A subscription only ever READS, so writing `tonk:site!:` here
    // would show the reader a claim that asserts facts — the opposite of what
    // the subscription does.
    let head = query_head(json);
    let mut out = format!("{head}:\n");

    // `terms` carries the bindings: a concrete value pins the field, a
    // `{"?": {"name": …}}` marks it as an output the subscription selects.
    if let Some(terms) = json.get("terms").and_then(|t| t.as_object()) {
        for (name, term) in terms {
            out.push_str(&format!("  {name}: {}\n", render_term(term)));
        }
    }
    out
}

/// The notation head for a query: the concept it selects, or the formula it
/// names.
///
/// A concept query's predicate is a descriptor whose fields are attributes;
/// the concept's own name is not carried on the wire, so the head is derived
/// from the attribute namespace its fields share (`xyz.tonk.site/path` →
/// `tonk:site`). A formula query names its procedure outright.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn query_head(json: &serde_json::Value) -> String {
    let predicate = match json.get("predicate") {
        Some(predicate) => predicate,
        None => return "query".to_owned(),
    };

    // A formula predicate serializes as a bare string.
    if let Some(name) = predicate.as_str() {
        return name.to_owned();
    }

    predicate
        .get("with")
        .and_then(|with| with.as_object())
        .and_then(|fields| {
            fields.values().find_map(|field| {
                let the = field.get("the")?.as_str()?;
                // `xyz.tonk.site/path` → `tonk:site`: drop the vendor
                // prefix and the attribute name, and spell what is left as
                // a URI. A namespace that is not `xyz.`-prefixed (or is
                // only two segments deep) falls back to showing itself
                // rather than nothing — an unfamiliar attribute is still
                // more informative than the word "concept".
                let namespace = the.split('/').next()?;
                let rest = namespace.strip_prefix("xyz.").unwrap_or(namespace);
                Some(match rest.split_once('.') {
                    Some((head, tail)) => format!("{head}:{tail}"),
                    None => rest.to_owned(),
                })
            })
        })
        .unwrap_or_else(|| "concept".to_owned())
}

/// Render one term: a `?variable` for a selected output, otherwise the bound
/// value as notation writes it.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn render_term(term: &serde_json::Value) -> String {
    if let Some(variable) = term.get("?") {
        let name = variable.get("name").and_then(|n| n.as_str()).unwrap_or("_");
        return format!("?{name}");
    }
    match term {
        serde_json::Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

/// Format epoch milliseconds as an ISO-8601 string.
///
/// Via the platform's own `Date`, which is present wherever this runs (the
/// service worker) and already implements the exact format
/// `<wa-relative-time date=…>` parses. Adding a date-formatting crate to
/// reimplement `toISOString` would be a dependency in exchange for nothing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn iso_8601(epoch_ms: u64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(epoch_ms as f64))
        .to_iso_string()
        .into()
}

/// The notation rendering — pure, so it runs on every target.
#[cfg(test)]
mod tests {
    use super::{query_head, render_query_json, render_term};

    /// A wire query as it actually arrives: the site display's standing
    /// subscription, with one bound term and one selected variable.
    fn site_query() -> serde_json::Value {
        serde_json::json!({
            "terms": {
                "this": "site:abc",
                "path": {"?": {"name": "path"}}
            },
            "predicate": {
                "with": {
                    "path": {
                        "the": "xyz.tonk.site/path",
                        "description": "",
                        "cardinality": "one",
                        "as": "Text"
                    }
                }
            }
        })
    }

    /// A subscription READS. The `!` suffix is notation's mutation marker,
    /// so rendering `tonk:site!:` would show a claim that asserts facts —
    /// the opposite of what a subscription does.
    #[dialog_common::test]
    async fn it_renders_a_query_without_the_mutation_marker() {
        let rendered = render_query_json(&site_query());
        assert!(
            !rendered.contains("!:"),
            "a query is a read, not an assertion: {rendered}"
        );
        assert!(
            rendered.starts_with("tonk:site:"),
            "expected a `tonk:site:` head, got: {rendered}"
        );
    }

    /// Bound terms render as values, selected ones as `?variable` — the
    /// distinction a reader needs to tell what the query asks for from what
    /// it pins.
    #[dialog_common::test]
    async fn it_distinguishes_bound_terms_from_selected_variables() {
        let rendered = render_query_json(&site_query());
        assert!(
            rendered.contains("this: site:abc"),
            "a bound term renders as its value: {rendered}"
        );
        assert!(
            rendered.contains("path: ?path"),
            "a selected term renders as a variable: {rendered}"
        );
    }

    /// The head comes from the attribute namespace the predicate's fields
    /// share, because the concept's own name is not carried on the wire.
    #[dialog_common::test]
    async fn it_derives_the_head_from_the_attribute_namespace() {
        assert_eq!(query_head(&site_query()), "tonk:site");
    }

    /// A formula query names its procedure outright, rather than describing
    /// a concept — so the head is that name.
    #[dialog_common::test]
    async fn it_uses_the_procedure_name_as_the_head_of_a_formula_query() {
        let formula = serde_json::json!({
            "terms": {},
            "predicate": "tree/node"
        });
        assert_eq!(query_head(&formula), "tree/node");
    }

    /// An unnamed variable still renders as one. The term is a selection
    /// whatever it is called, and dropping the `?` would read as a bound
    /// value.
    #[dialog_common::test]
    async fn it_renders_an_unnamed_variable_as_a_wildcard() {
        assert_eq!(render_term(&serde_json::json!({"?": {}})), "?_");
    }
}
