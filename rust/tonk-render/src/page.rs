//! Headless `model -> view -> entity -> HTML` page orchestration.
//!
//! The host-agnostic half of server-side `<tonk-display>` rendering.
//! It runs the same model/view/entity resolution the browser component
//! runs, feeds the result through the shared [`tonk_template`] planner
//! and this crate's headless renderer, and recursively expands nested
//! `<tonk-display>` elements. Where the pure renderer ([`crate::render`])
//! takes a template plus data, this layer figures out *which* view and
//! *what* data, by querying.
//!
//! The only thing it does *not* own is where the data comes from:
//! every query goes through a [`QueryBackend`], a small trait that
//! turns a [`dialog_query::ConceptQuery`] into a `Vec<Conclusion>`.
//! A host implements it over whatever branch handle it has:
//!
//! - `slide` over its on-disk `.tonk/` reactor (fixed repo/branch).
//! - the worker over a URL-named repository/branch.
//!
//! Both then share this whole orchestrator, so `slide render` and a
//! worker SSR route produce identical markup by construction.

mod orchestrate;
mod route;

pub use orchestrate::render;
pub use route::RenderRoute;

use async_trait::async_trait;
use dialog_query::ConceptQuery;
use tonk_schema::conclusion::Conclusion;

/// A source of query results: the one host-specific seam in the
/// otherwise pure render pipeline.
///
/// Implementors run `query` against whatever branch they front (an
/// on-disk reactor, a URL-named worker branch, …) and return the flat
/// [`Conclusion`] rows. Everything else in the pipeline is pure logic
/// over `tonk-schema` / `tonk-template` / `tonk-render` types.
///
/// `query` takes a [`ConceptQuery`] (not the broader
/// `tonk_schema::query::Query`): the render pipeline only ever builds
/// concept queries, so lowering to a `ConceptQuery` happens inside this
/// module, before the backend is ever asked. That keeps the trait
/// minimal and free of the `into_concept_query` fallibility.
#[async_trait(?Send)]
pub trait QueryBackend {
    /// Run a concept query and return its conclusions.
    async fn query(&self, query: ConceptQuery) -> Result<Vec<Conclusion>, RenderError>;
}

/// Errors surfaced by the page-render orchestrator.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The backend's query failed. Carries the host's own error text.
    #[error("query failed: {0}")]
    Query(String),

    /// A render query was a formula, not a concept query. The render
    /// pipeline only builds concept queries, so this is an internal
    /// invariant violation rather than a user error.
    #[error("render queries must be concept queries, not formulas")]
    NotConceptQuery,

    /// No concept matched a model/view name or URI.
    #[error("no concept matched `{0}`")]
    NoConcept(String),

    /// A concept descriptor was missing or malformed.
    #[error("{0}")]
    Descriptor(String),

    /// No view resolved for a model (no model-specific view and no
    /// `_:_` default).
    #[error("no view found for model `{0}` (no model-specific view and no `_:_` default)")]
    NoView(String),

    /// A bookmark name did not resolve to an entity.
    #[error("no entity named `{0}`")]
    UnknownName(String),

    /// A built query could not be constructed (predicate/term error).
    #[error("query construction failed: {0}")]
    QueryConstruction(String),

    /// Nested-render recursion exceeded the depth backstop.
    #[error("render recursion exceeded {0} levels (cycle in nested views?)")]
    RecursionDepth(usize),

    /// A view nests itself with the same parameters (a genuine cycle).
    #[error("render cycle: `{0}` nests itself with the same parameters")]
    RenderCycle(String),
}

/// A `serde_json` error while building a query from a descriptor maps
/// to a query-construction failure: the descriptor JSON was malformed.
impl From<serde_json::Error> for RenderError {
    fn from(error: serde_json::Error) -> Self {
        RenderError::QueryConstruction(error.to_string())
    }
}
