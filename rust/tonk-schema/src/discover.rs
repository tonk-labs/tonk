//! Template catalogs the account owner added to the Hub's Discover tab.
//!
//! Declared in `profile.yaml` as `discover/catalog` and
//! `discover/catalog-receipt`; these structs implement those
//! declarations.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::discover::Url;
use crate::domain::discover_receipt::{Detail, Status};
use crate::prelude::EntityExt as _;

/// A template catalog the Discover tab lists beside the deployment's
/// default, on the profile branch.
///
/// The entity is derived from the URL, so adding the same catalog twice
/// writes the same row once, and removing it names exactly one row.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoverCatalog {
    /// Derived from the canonical URL.
    pub this: Entity,
    /// The catalog's canonical URL.
    pub url: Url,
}

impl DiscoverCatalog {
    /// The row for the catalog at `url`, which the caller has already
    /// admitted and canonicalised.
    pub fn new(url: &str) -> Self {
        Self {
            this: Entity::of(&("xyz.tonk.discover-catalog", url)),
            url: Url(url.to_owned()),
        }
    }
}

/// How one catalog command came out, keyed by the command's entity so
/// the page that asked can wait for its own answer. Overlay-only: a
/// refusal is this session's business, not the account's.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CatalogReceipt {
    /// The command entity this answers.
    pub this: Entity,
    /// `added`, `removed`, or `refused`.
    pub status: Status,
    /// The listed URL, or why it was refused.
    pub detail: Detail,
}

impl CatalogReceipt {
    /// The answer to the command `this`.
    pub fn new(this: Entity, status: &str, detail: impl Into<String>) -> Self {
        Self {
            this,
            status: Status(status.to_owned()),
            detail: Detail(detail.into()),
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
    fn it_keys_a_catalog_by_its_url() {
        let a = DiscoverCatalog::new("https://a.example/catalog.json");
        assert_eq!(a, DiscoverCatalog::new("https://a.example/catalog.json"));
        assert_ne!(
            a.this,
            DiscoverCatalog::new("https://b.example/catalog.json").this
        );
    }
}
