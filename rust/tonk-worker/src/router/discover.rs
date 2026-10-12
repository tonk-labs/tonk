//! The Discover tab's catalogs: `discover/add-catalog` and
//! `discover/remove-catalog`.
//!
//! The Hub lists templates from the deployment's default catalog (served
//! at `/.well-known/tonk/discover`) plus every `discover/catalog` row on
//! the profile branch. Those rows are written only here. A catalog
//! decides what code is installed into new spaces, so the URL is held to
//! the rule template sources are fetched under — `https:`, or `http:` on
//! a loopback host — before anything is recorded, and both commands are
//! in the profile vocabulary alone: a space branch cannot add one.
//!
//! Each command answers on a [`CatalogReceipt`] keyed by its own entity,
//! on the profile overlay, so the control that asked can show a refusal.

use dialog_artifacts::Entity;
use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_schema::{CatalogReceipt, DiscoverCatalog};
use tonk_worker_api::{CatalogRefusal, admit_catalog_url};

use crate::worker::TonkState;

/// Receipt status: the catalog is listed.
pub(crate) const ADDED: &str = "added";
/// Receipt status: the catalog is no longer listed.
pub(crate) const REMOVED: &str = "removed";
/// Receipt status: nothing was written, and the detail says why.
pub(crate) const REFUSED: &str = "refused";

/// Why a catalog command did nothing.
#[derive(Debug)]
pub(crate) enum CatalogError {
    /// The URL may not name a catalog.
    Refused(CatalogRefusal),
    /// The command came from a space's branch, not the profile.
    Containment,
    /// There is no such `discover/catalog` row to remove.
    NotFound,
    /// The branch could not be read or written.
    Internal(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::Containment => write!(f, "catalogs are added from the Hub, not from a space"),
            Self::NotFound => write!(f, "that catalog is not in your list"),
            Self::Internal(detail) => write!(f, "couldn't update your catalogs: {detail}"),
        }
    }
}

/// Every catalog the account owner added, from the profile branch.
pub(crate) async fn catalogs(tonk: &TonkState) -> Result<Vec<DiscoverCatalog>, CatalogError> {
    let branch = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| CatalogError::Internal(error.to_string()))?;
    branch
        .handle()
        .query()
        .select(Query::<DiscoverCatalog> {
            this: Term::var("this"),
            url: Term::var("url"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| CatalogError::Internal(format!("{error:?}")))
}

/// Admit `raw` and record it as a `discover/catalog` row on the profile
/// branch. A refused URL writes nothing.
pub(crate) async fn add_catalog(
    tonk: &TonkState,
    raw: &str,
) -> Result<DiscoverCatalog, CatalogError> {
    let url = admit_catalog_url(raw).map_err(CatalogError::Refused)?;
    let row = DiscoverCatalog::new(url.as_str());
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction()
        .assert(row.clone())
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| CatalogError::Internal(error.to_string()))?;
    Ok(row)
}

/// Retract the `discover/catalog` row `catalog`. Only a row that exists
/// is retracted, so the command cannot be aimed at anything else.
pub(crate) async fn remove_catalog(
    tonk: &TonkState,
    catalog: &Entity,
) -> Result<DiscoverCatalog, CatalogError> {
    let row = catalogs(tonk)
        .await?
        .into_iter()
        .find(|row| &row.this == catalog)
        .ok_or(CatalogError::NotFound)?;
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction()
        .retract(row.clone())
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| CatalogError::Internal(error.to_string()))?;
    Ok(row)
}

/// Answer the command `this` on the profile overlay.
async fn answer(tonk: &TonkState, this: Entity, outcome: Result<(&str, String), CatalogError>) {
    let (status, detail) = match outcome {
        Ok((status, detail)) => (status, detail),
        Err(error) => {
            log!("discover catalog: refused: {error}");
            (REFUSED, error.to_string())
        }
    };
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .overlay()
        .assert(CatalogReceipt::new(this, status, detail))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("discover catalog: the receipt was not published: {error}");
    }
}

/// Run `discover/add-catalog`, from the Discover tab's catalogs control.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AddCatalog> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::AddCatalog) {
        let tonk = self.state().read().await;
        let outcome = if self.origin().repo.is_empty() {
            add_catalog(&tonk, &command.url.0)
                .await
                .map(|row| (ADDED, row.url.0))
        } else {
            Err(CatalogError::Containment)
        };
        answer(&tonk, command.this, outcome).await;
    }
}

/// Run `discover/remove-catalog`, from a row of the catalogs control.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::RemoveCatalog>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::RemoveCatalog) {
        let tonk = self.state().read().await;
        let outcome = if self.origin().repo.is_empty() {
            remove_catalog(&tonk, &command.catalog.0)
                .await
                .map(|row| (REMOVED, row.url.0))
        } else {
            Err(CatalogError::Containment)
        };
        answer(&tonk, command.this, outcome).await;
    }
}
