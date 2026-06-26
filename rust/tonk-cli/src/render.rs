//! `tonk render <route>` — headless server-side rendering of a
//! `<tonk-display>` view to an HTML string.
//!
//! The concept -> view -> entity -> HTML orchestration lives in
//! [`tonk_render::page`]; this module is just the tonk-side wiring:
//! it implements [`tonk_render::QueryBackend`] over a [`TonkSite`]'s
//! reactor (the on-disk `.tonk/` `main` branch) and re-exports the
//! shared [`RenderRoute`] so callers (the binary, the integration
//! tests) keep a single render entry point.

use async_trait::async_trait;
use dialog_query::ConceptQuery;
use tonk_render::{QueryBackend, RenderError};

pub use tonk_render::RenderRoute;

use crate::site::{BRANCH_NAME, REPO_NAME, TonkSite};

/// Run a concept query against the site's `main` branch through the
/// reactor's one-shot query.
#[async_trait(?Send)]
impl QueryBackend for TonkSite {
    async fn query(
        &self,
        query: ConceptQuery,
    ) -> Result<Vec<tonk_schema::conclusion::Conclusion>, RenderError> {
        self.reactor
            .repository(REPO_NAME)
            .branch(BRANCH_NAME)
            .query(query)
            .perform(&self.operator)
            .await
            .map_err(|e| RenderError::Query(e.to_string()))
    }
}

/// Render `route` against `site` and return the HTML string.
pub async fn render(site: &TonkSite, route: &RenderRoute) -> Result<String, RenderError> {
    tonk_render::render_page(site, route).await
}
