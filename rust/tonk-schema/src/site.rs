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
use crate::domain::site::{
    Anchor, Branch, Concept as SiteConcept, Path, Replica, Route as SiteRoute, Space,
};

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
    /// The space (repository name) the tab is on.
    pub space: Space,
    /// The active branch the tab is on (defaults to `"main"`).
    pub branch: Branch,
    /// This tab's active replica entity.
    pub replica: Replica,
    /// The matched route entity (the route-table entry).
    pub route: SiteRoute,
    /// The matched route's concept — the model the shell mounts.
    pub concept: SiteConcept,
}

impl Site {
    /// A site stamp for the given site entity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        this: Entity,
        path: String,
        anchor: String,
        space: String,
        branch: String,
        replica: Entity,
        route: Entity,
        concept: Entity,
    ) -> Self {
        Self {
            this,
            path: Path(path),
            anchor: Anchor(anchor),
            space: Space(space),
            branch: Branch(branch),
            replica: Replica(replica),
            route: SiteRoute(route),
            concept: SiteConcept(concept),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_carries_the_space_name() {
        let site = Site::new(
            "site:test".parse().unwrap(),
            "/space/home".to_owned(),
            String::new(),
            "home".to_owned(),
            "main".to_owned(),
            "replica:r".parse().unwrap(),
            "route:x".parse().unwrap(),
            "concept:y".parse().unwrap(),
        );
        assert_eq!(site.space.0, "home");
    }

    #[dialog_common::test]
    async fn it_carries_the_branch_name() {
        let site = Site::new(
            "site:test".parse().unwrap(),
            "/space/feature@home".to_owned(),
            String::new(),
            "home".to_owned(),
            "feature".to_owned(),
            "replica:r".parse().unwrap(),
            "route:x".parse().unwrap(),
            "concept:y".parse().unwrap(),
        );
        assert_eq!(site.branch.0, "feature");
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

/// A seed-update check IN FLIGHT on this device, keyed on the replica.
///
/// The replica entity already pairs this profile with this subject, and
/// replica records live on the profile meta branch and never replicate —
/// exactly the scope of "am I checking right now". Keying on the space
/// alone would let one device's check overwrite another's.
///
/// Presence IS the state: asserted before the fetch, retracted when the
/// check settles, so a view asks whether the attribute exists rather
/// than comparing against a case value. It holds the check's own
/// transient entity rather than a boolean, so a marker stranded by a
/// crashed worker is identifiable and a second check cannot silently
/// clobber the first's record.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaChecking {
    /// The replica entity being stamped.
    pub this: Entity,
    /// The dispatched check's transient entity.
    pub checking: crate::domain::check::Checking,
}

/// When this device last COMPLETED a seed-update check.
///
/// Separate from [`ReplicaChecking`] so it survives the next check
/// starting: a single status field would have to overwrite the previous
/// result to say "pending", losing the answer a view is showing.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct ReplicaChecked {
    /// The replica entity being stamped.
    pub this: Entity,
    /// The completing check's timestamp.
    pub checked: crate::domain::check::Checked,
}

/// Why this device's last seed-update check FAILED — asserted only on
/// failure, retracted on the next success.
///
/// Text rather than a case: an unreachable source and a malformed
/// document are different problems and the message is the useful part.
/// A space that never recorded a seed is not a failure — it simply has
/// no [`SeedInstalled`] fact, and that absence is the answer.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaCheckFailure {
    /// The replica entity being stamped.
    pub this: Entity,
    /// What went wrong, as reported to the person.
    pub failure: crate::domain::check::Failure,
}

/// A seed that EXISTS and where its bytes came from — global, durable,
/// and silent about whether anything installed it.
///
/// The entity is the content hash of the bytes, so two devices fetching
/// the same library converge on one entity without coordinating.
/// [`SeedInstalled`] adds the install-specific fields on this same
/// entity; keeping them apart means a fetched-but-uninstalled seed can
/// never look half-installed.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeedAvailable {
    /// The seed's entity: the content hash of its bytes.
    pub this: Entity,
    /// Where these bytes were fetched from.
    pub source: crate::domain::seed::Source,
    /// The installed seed this one would supersede — the backlink that
    /// makes an available seed answerable on its own.
    pub replaces: crate::domain::seed::Replaces,
}

/// The seed a space is RUNNING — the install-specific half, asserted on
/// the same entity as [`SeedAvailable`].
///
/// The version names the commit that installed it, and a commit's history
/// is a changelog — so an upgrade inverts that commit's assertions rather
/// than consulting a per-component tag.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeedInstalled {
    /// The seed's entity: the content hash of its bytes.
    pub this: Entity,
    /// The seed it replaced, or `seed:none` on a first install.
    pub prior: crate::domain::seed::Prior,
    /// The version of the commit that installed it.
    pub version: crate::domain::seed::Version,
}
