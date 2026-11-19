//! The views a space renders, read headlessly on a peer.
//!
//! An agent asks for a space's outline (the tree of `<tonk-display>`
//! elements its home mounts) to choose one view to show on its own. It runs
//! [`tonk_render`] against the space's `main` branch, which is the same
//! model, view and entity resolution the browser runs.

use dialog_query::ConceptQuery;
use dialog_varsig::Did;
use serde_json::{Value, json};
use tonk_render::{QueryBackend, RenderError, RenderRoute};
use tonk_schema::prelude::DidExt as _;

use super::repository::CONTENT_BRANCH;
use crate::AppState;

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
        return Ok(json!({ "space": space.to_string(), "outline": outline }));
    }
    let mut answer = json!({ "space": space.to_string(), "outline": outline });
    if let Some(title) = title(&backend, route).await {
        answer["outline"]["title"] = json!(title);
    }
    Ok(answer)
}

/// Longest title a view is named by, in characters.
const TITLE_LIMIT: usize = 120;

/// What a view calls itself, as text: its model's `title` facet, else, for
/// one entity, its `label` facet. A directory renders a facet once per
/// instance, so it has no title of its own. `None` when the model has
/// neither facet, or it renders empty.
async fn title(backend: &Space, route: &RenderRoute) -> Option<String> {
    let entity = route.entity.as_ref()?;
    for facet in ["title", "label"] {
        let faceted = RenderRoute {
            model: route.model.clone(),
            entity: Some(entity.clone()),
            view: Some(facet.into()),
        };
        let Ok(html) = tonk_render::render_page(backend, &faceted).await else {
            continue;
        };
        if let Some(text) = text_of(&html) {
            return Some(text);
        }
    }
    None
}

/// The text a facet's markup says: a `<tab-title text="…">` (the browser
/// tab's title, without its " — Tonk" suffix), else the markup's text.
fn text_of(html: &str) -> Option<String> {
    let text = match html.split_once("<tab-title text=\"") {
        Some((_, rest)) => rest
            .split_once('"')
            .map_or(rest, |(text, _)| text)
            .to_owned(),
        None => {
            let mut text = String::new();
            let mut tag = false;
            for c in html.chars() {
                match c {
                    '<' => tag = true,
                    '>' => {
                        tag = false;
                        text.push(' ');
                    }
                    c if !tag => text.push(c),
                    _ => {}
                }
            }
            text
        }
    };
    let text = text
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = text.strip_suffix(" — Tonk").unwrap_or(&text).trim();
    (!text.is_empty()).then(|| text.chars().take(TITLE_LIMIT).collect())
}

/// Whether `route` names the space home, which each peer mounts on its own
/// replica, so a view shows it by path rather than by this entity.
pub(crate) fn is_home(route: &RenderRoute) -> bool {
    route.model == HOME_MODEL
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

#[cfg(test)]
mod tests {
    use super::text_of;

    #[dialog_common::test]
    fn it_reads_a_tab_title_without_its_suffix() {
        assert_eq!(
            text_of(r#"<tab-title text="Packing list — Tonk"></tab-title>"#).as_deref(),
            Some("Packing list")
        );
    }

    #[dialog_common::test]
    fn it_reads_a_label_as_its_text() {
        assert_eq!(
            text_of("<span class=\"x\">Packing\n  list &amp; more</span>").as_deref(),
            Some("Packing list & more")
        );
        assert_eq!(text_of("<span>  </span>"), None);
    }
}
