//! [`Site`] — a tab's location and the route it renders.
//!
//! The service worker stamps `Site` onto the per-tab **site entity** (the
//! `X-Tonk-Site` header value, a `site:<uuid>` parsed to an [`Entity`]) in the
//! Level-0-resolved branch's overlay, exactly the `state:here` pattern the sync
//! chip uses but keyed per tab instead of a singleton. Multiple tabs coexist as
//! distinct site entities; a view scoped to a tab's site reads only its context.
//!
//! Route models (e.g. `tonk:space/route`) pick the `site/*` fields they need and
//! resolve on the same site entity; the shell mounts the matched route model
//! ([`Site::concept`]) on the site entity, and that model's view renders.

// The `#[derive(Concept)]` macro generates helper types without doc comments.
// Suppress the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::route::{Concept as RoutePathConcept, Path as RouteTablePath};
use crate::domain::site::{Anchor, Concept as SiteConcept, Path, Replica, Route as SiteRoute};

/// A tab's location and matched route, keyed on the per-tab site entity. The SW
/// stamps it; the shell reads it. All fields cardinality one, so a navigation
/// re-stamp supersedes — the site always reflects the tab's latest location.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    /// The per-tab site entity (`site:<uuid>`), the `X-Tonk-Site` value.
    pub this: Entity,
    /// The matched document path.
    pub path: Path,
    /// The document fragment (may be empty).
    pub anchor: Anchor,
    /// This tab's active replica entity.
    pub replica: Replica,
    /// The matched route entity (the route-table entry).
    pub route: SiteRoute,
    /// The matched route's concept — the model the shell mounts.
    pub concept: SiteConcept,
}

impl Site {
    /// A site stamp for the given site entity.
    pub fn new(
        this: Entity,
        path: String,
        anchor: String,
        replica: Entity,
        route: Entity,
        concept: Entity,
    ) -> Self {
        Self {
            this,
            path: Path(path),
            anchor: Anchor(anchor),
            replica: Replica(replica),
            route: SiteRoute(route),
            concept: SiteConcept(concept),
        }
    }
}

/// A durable route — one row of the table the SW reads to build its matchit
/// router: a path pattern → the route model to mount. `route!` instances in the
/// library populate it; the SW queries them on a branch and feeds
/// `path` → `concept` to `matchit`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Route {
    /// The route's entity.
    pub this: Entity,
    /// The axum/matchit path pattern.
    pub path: RouteTablePath,
    /// The route model mounted when this path matches.
    pub concept: RoutePathConcept,
}
