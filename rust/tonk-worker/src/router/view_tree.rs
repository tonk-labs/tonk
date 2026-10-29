//! The views a space renders, read headlessly on a native peer.
//!
//! An agent asks for a space's outline (the tree of `<tonk-display>`
//! elements its home mounts) to choose one view to show on its own. It runs
//! [`tonk_render`] against the space's `main` branch, which is the same
//! model, view and entity resolution the browser runs.

use dialog_query::ConceptQuery;
use serde_json::{Value, json};
use tonk_render::{QueryBackend, RenderError, RenderRoute};

use crate::AppState;

/// The model the space home mounts on this device's replica.
const HOME_MODEL: &str = "tonk/space";

/// Longest route an agent may pass.
const ROUTE_LIMIT: usize = 2048;

/// Read `route` (`entity@model!view`), or the space home when it is empty.
pub(super) async fn route(
    state: &AppState,
    space: &str,
    route: &str,
) -> Result<RenderRoute, String> {
    let route = route.trim();
    if route.len() > ROUTE_LIMIT {
        return Err("Provide a route from tonk_outline, up to 2 KB.".into());
    }
    if !route.is_empty() {
        return RenderRoute::parse(route).map_err(|error| error.to_string());
    }
    Ok(RenderRoute {
        model: HOME_MODEL.into(),
        entity: Some(home_entity(state, space).await),
        view: None,
    })
}

/// The entity the home mounts on: this device's replica of the space, or
/// the space itself before a replica is recorded.
async fn home_entity(state: &AppState, space: &str) -> String {
    let tonk = state.read().await;
    let branch = tonk.reactor.repository(space).branch("main");
    if let Ok(session) = branch.acquire(&tonk.operator).await
        && let Some(replica) = super::session::origin_entity(&tonk, &session).await
    {
        return replica.to_string();
    }
    space.to_owned()
}

/// The `<tonk-display>` tree rendering `route` mounts.
///
/// The renderer's futures are not `Send` (it also runs in the browser), and
/// hosts run peer calls on spawned tasks. So the walk runs on a blocking
/// thread, driven by this runtime's handle, and only its result crosses back.
pub(super) async fn outline(
    state: &AppState,
    space: &str,
    route: &RenderRoute,
) -> Result<Value, String> {
    let backend = Space {
        state: state.clone(),
        space: space.to_owned(),
    };
    let route = route.clone();
    let runtime = tokio::runtime::Handle::current();
    let outline = tokio::task::spawn_blocking(move || {
        runtime.block_on(async move { tonk_render::outline(&backend, &route).await })
    })
    .await
    .map_err(|_| "The outline could not be completed.".to_owned())?
    .map_err(|error| error.to_string())?;
    Ok(json!({ "space": space, "outline": outline }))
}

/// Queries against one space's `main` branch.
struct Space {
    state: AppState,
    space: String,
}

#[async_trait::async_trait(?Send)]
impl QueryBackend for Space {
    async fn query(
        &self,
        query: ConceptQuery,
    ) -> Result<Vec<tonk_schema::conclusion::Conclusion>, RenderError> {
        let tonk = self.state.read().await;
        tonk.reactor
            .repository(&self.space)
            .branch("main")
            .query(query)
            .perform(&tonk.operator)
            .await
            .map_err(|error| RenderError::Query(error.to_string()))
    }
}
