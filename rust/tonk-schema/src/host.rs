//! [`HostContext`] — a tab's per-request routing context.
//!
//! The service worker stamps this onto the per-tab **host-id entity** (the
//! `X-Tonk-Session` header value, parsed to an [`Entity`]) in the
//! Level-0-resolved branch's overlay, exactly the `state:here` pattern the sync
//! chip uses but keyed per tab instead of a singleton. Multiple tabs coexist as
//! distinct entities in the same overlay; a view scoped to a tab's host-id
//! entity reads only that tab's context.

// The `#[derive(Concept)]` macro generates helper types without doc comments.
// Suppress the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::host::{Hash, Path};
use crate::domain::route::Path as RouteMatchedPath;
use crate::domain::router_active::Model as ActiveModel;
use crate::domain::router_route::{Model as RouteModel, Path as RoutePathAttr};

/// A tab's request context: the location (`path`, `hash`) the host stamped,
/// keyed on the tab's host-id entity.
///
/// Both attributes are cardinality one, so re-stamping on navigation supersedes
/// the prior values rather than accumulating — the entity always reflects the
/// tab's latest location.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostContext {
    /// The per-tab host-id entity (`host:<uuid>`), the `X-Tonk-Session` value.
    pub this: Entity,
    /// The document path the request came from.
    pub path: Path,
    /// The document fragment (may be empty when the location has none).
    pub hash: Hash,
}

impl HostContext {
    /// A context stamp for the given host-id entity.
    pub fn new(this: Entity, path: String, hash: String) -> Self {
        Self {
            this,
            path: Path(path),
            hash: Hash(hash),
        }
    }
}

/// A durable route — one row of the table the SW page router reads. A `route`
/// command materializes these (via the library rules); the SW queries them on a
/// branch and feeds `path` → `model` to `matchit`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouterRoute {
    /// The route's entity (the command's derived `this`).
    pub this: Entity,
    /// The axum/matchit path pattern.
    pub path: RoutePathAttr,
    /// The page model rendered when this path matches.
    pub model: RouteModel,
}

/// The per-tab matched route, keyed on the host-id entity. The SW asserts this
/// in the overlay after matching; the shell mounts
/// `<tonk-display model=router/active entity={host-id}>` and the active view
/// delegates to `model`. Cardinality one, so a navigation re-stamp supersedes.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouterActive {
    /// The per-tab host-id entity.
    pub this: Entity,
    /// The matched page model.
    pub model: ActiveModel,
}

impl RouterActive {
    /// A matched-route stamp for the given host-id entity.
    pub fn new(this: Entity, model: Entity) -> Self {
        Self {
            this,
            model: ActiveModel(model),
        }
    }
}

/// The matched `path` on the host-id entity — the shared field every route page
/// model (`route/default`, `route/board`, …) carries, so its instance resolves
/// for the entity-bound delegation. Stamped by the SW alongside [`RouterActive`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteMatch {
    /// The per-tab host-id entity.
    pub this: Entity,
    /// The matched path.
    pub path: RouteMatchedPath,
}

impl RouteMatch {
    /// A matched-path stamp for the given host-id entity.
    pub fn new(this: Entity, path: String) -> Self {
        Self {
            this,
            path: RouteMatchedPath(path),
        }
    }
}
