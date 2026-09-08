//! Concepts backing the `/console` introspection page.
//!
//! These implement the declarations in
//! `tonk-core/assets/library/console.yaml`, which is the schema of record.
//!
//! Every fact here is published into a branch's **session overlay**, never
//! committed: it describes the live state of one worker process (which
//! queries are subscribed, who is listening), which is not something a
//! branch should carry across a restart, let alone replicate to another
//! device. The overlay is exactly the right home — folded into every read,
//! including standing subscriptions, so the console re-renders as the state
//! changes, and gone when the worker is.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::console_group;
use crate::domain::console_subscription::{
    Branch, ConceptName, Group, Hash, LastUpdate, OpenedAt, Pending, Query, Space, Subscribers,
    Updates,
};
use crate::domain::console_update;

/// One live query subscription in this device's reactor.
///
/// The entity is minted per `(repository, branch, query hash)` by
/// [`Self::entity_for`], so a subscription keeps the same row identity for as
/// long as it exists and a re-publish supersedes its fields in place rather
/// than accumulating duplicates.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsoleSubscription {
    /// The row's entity — derived, not chosen. See [`Self::entity_for`].
    pub this: Entity,
    /// The repository the subscription's branch belongs to.
    pub space: Space,
    /// The branch the query runs against.
    pub branch: Branch,
    /// The subscribed query as JSON.
    pub query: Query,
    /// How many subscriber channels share this subscription.
    pub subscribers: Subscribers,
    /// How many of those await their first snapshot.
    pub pending: Pending,
    /// The subscription's query hash, hex-encoded.
    pub hash: Hash,
    /// When the subscription was opened, ISO-8601.
    pub opened_at: OpenedAt,
    /// The `(repository, branch)` group this row nests under.
    pub group: Group,
    /// How many updates this subscription has pushed.
    pub updates: Updates,
    /// The concept being watched, as a readable name — the collapsed row's
    /// label.
    pub concept_name: ConceptName,
}

/// The last-update stamp, as its own fact.
///
/// Separate from [`ConsoleSubscription`] because it is OPTIONAL — a
/// subscription that has never changed has no last-update time — and a
/// concept's `with:` fields are all required: folding it in would make every
/// never-updated row fail to resolve and vanish from the page.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsoleSubscriptionUpdate {
    /// The subscription row this stamps.
    pub this: Entity,
    /// When it last pushed an update, ISO-8601.
    pub last_update: LastUpdate,
}

impl ConsoleSubscription {
    /// The entity naming one subscription row.
    ///
    /// Derived from `(repository, branch, hash)` — the same triple that
    /// identifies the subscription in the reactor — so that republishing an
    /// unchanged subscription lands on the same entity and its
    /// cardinality-one fields supersede in place. A freshly minted entity
    /// per publish would instead grow the console a new row on every
    /// refresh.
    ///
    /// `console:` rather than a hash URI so a row is legible in a log line
    /// and obviously ephemeral; the hash already carries the uniqueness, and
    /// the branch and repository disambiguate the same query watched in two
    /// places.
    pub fn entity_for(repository: &str, branch: &str, hash: &str) -> Option<Entity> {
        format!("console:sub/{repository}/{branch}/{hash}")
            .parse()
            .ok()
    }
}

/// One `(repository, branch)` pair the console groups subscriptions under.
///
/// Published alongside the rows so the page can render a tree — repository
/// and branch named once, their queries nested beneath — instead of a flat
/// list that repeats both on every row.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsoleGroup {
    /// The group's entity, from [`Self::entity_for`].
    pub this: Entity,
    /// The repository this group covers.
    pub space: console_group::Space,
    /// The branch within it.
    pub branch: console_group::Branch,
    /// How many subscriptions are live on it.
    pub subscriptions: console_group::Subscriptions,
}

impl ConsoleGroup {
    /// The entity naming one `(repository, branch)` group.
    ///
    /// Derived from the pair, like [`ConsoleSubscription::entity_for`], so a
    /// group keeps its identity across refreshes and its fields supersede in
    /// place rather than accumulating a new row each time.
    pub fn entity_for(repository: &str, branch: &str) -> Option<Entity> {
        format!("console:group/{repository}/{branch}").parse().ok()
    }
}

/// One delivered update in a subscription's log.
///
/// Published per entry so the console's expanded row can render them as a
/// feed, newest first — the shape a network panel uses for its request list.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsoleUpdate {
    /// The update's own entity, from [`Self::entity_for`].
    pub this: Entity,
    /// The subscription it belongs to.
    pub subscription: console_update::Subscription,
    /// When it was pushed, ISO-8601.
    pub at: console_update::At,
    /// Serialized size of the delta frame, in bytes.
    pub bytes: console_update::Bytes,
    /// Position in the log, newest first.
    pub position: console_update::Position,
}

impl ConsoleUpdate {
    /// The entity naming one logged update.
    ///
    /// Keyed by the subscription and the entry's POSITION rather than its
    /// timestamp: the log is a fixed-size window that shifts as updates
    /// arrive, so position-keyed rows are overwritten in place as entries
    /// move down it. Keying by timestamp would instead mint a new entity per
    /// update and leave every expired one on the overlay forever.
    pub fn entity_for(subscription: &str, position: usize) -> Option<Entity> {
        format!("console:update/{subscription}/{position}")
            .parse()
            .ok()
    }
}
