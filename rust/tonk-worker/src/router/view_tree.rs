//! The views a space renders, read headlessly on a peer.
//!
//! An agent asks for a space's outline (the tree of `<tonk-display>`
//! elements its home mounts) to choose one view to show on its own. It runs
//! [`tonk_render`] against the space's `main` branch, which is the same
//! model, view and entity resolution the browser runs.

use axum::Json;
use axum::extract::{Path, Query, State};
use dialog_query::ConceptQuery;
use dialog_varsig::Did;
use serde::Deserialize;
use serde_json::{Value, json};
use tonk_macros::handler;
use tonk_render::{QueryBackend, RenderError, RenderRoute};
use tonk_schema::prelude::DidExt as _;

use super::repository::CONTENT_BRANCH;
use crate::AppState;
use crate::TonkWorkerError;

/// The model the space home mounts on this peer's replica.
const HOME_MODEL: &str = "tonk/space";

/// Longest route an agent may pass.
const ROUTE_LIMIT: usize = 2048;

/// Read `route` (`entity@model!view`), or the space home when it is empty.
pub(crate) async fn route(
    state: &AppState,
    space: &Did,
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

/// The entity the home mounts on: this peer's replica of the space, or
/// the space itself before a replica is recorded.
async fn home_entity(state: &AppState, space: &Did) -> String {
    let tonk = state.read().await;
    let branch = tonk
        .reactor
        .repository(space.repo_key())
        .branch(CONTENT_BRANCH);
    if let Ok(session) = branch.acquire(&tonk.operator).await
        && let Some(replica) = super::session::origin_entity(&tonk, &session).await
    {
        return replica.to_string();
    }
    space.to_string()
}

/// The `<tonk-display>` tree rendering `route` mounts, as the agent reads
/// it: each node with the route that shows it alone.
///
/// The home mounts on this peer's own replica entity, which another peer
/// (a view's) does not have: the home's route is answered empty, which
/// shows the space's home wherever it is opened.
pub(crate) async fn outline(
    state: &AppState,
    space: &Did,
    route: &RenderRoute,
) -> Result<Value, String> {
    let backend = Space {
        state: state.clone(),
        space: space.clone(),
    };
    let mut outline = tonk_render::outline(&backend, route)
        .await
        .map_err(|error| error.to_string())?;
    if outline.model == HOME_MODEL {
        outline.route = String::new();
    }
    Ok(json!({ "space": space.to_string(), "outline": outline }))
}

/// Queries against one space's `main` branch.
struct Space {
    state: AppState,
    space: Did,
}

#[async_trait::async_trait(?Send)]
impl QueryBackend for Space {
    async fn query(
        &self,
        query: ConceptQuery,
    ) -> Result<Vec<tonk_schema::conclusion::Conclusion>, RenderError> {
        let tonk = self.state.read().await;
        tonk.reactor
            .repository(self.space.repo_key())
            .branch(CONTENT_BRANCH)
            .query(query)
            .perform(&tonk.operator)
            .await
            .map_err(|error| RenderError::Query(error.to_string()))
    }
}

/// The route `GET .../outline` asks for: empty for the space's home.
#[derive(Debug, Deserialize)]
pub struct OutlineQuery {
    #[serde(default)]
    route: String,
}

/// `GET /api/repository/{repo}/outline?route=`: the `<tonk-display>` tree
/// the route mounts (see [`outline`]). A space's kernel answers it: what
/// renders a view headlessly is not `Send`, which a native host asks of a
/// route.
#[handler]
pub async fn get(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Query(query): Query<OutlineQuery>,
) -> Result<Json<Value>, TonkWorkerError> {
    let space: Did = repo
        .parse()
        .map_err(|_| TonkWorkerError::Router(format!("'{repo}' names no space")))?;
    let route = route(&state, &space, &query.route)
        .await
        .map_err(TonkWorkerError::Router)?;
    outline(&state, &space, &route)
        .await
        .map(Json)
        .map_err(TonkWorkerError::Router)
}
