//! URL builders for the worker's HTTP endpoints.
//!
//! The four operations map to four endpoint shapes:
//!
//! | Op       | Path                                                |
//! |----------|-----------------------------------------------------|
//! | query    | `/api/repository/{space}/branch/{branch}/query`     |
//! | subscribe| `/api/repository/{space}/branch/{branch}/query`     |
//! | claim    | `/api/repository/{space}/branch/{branch}/transact`  |
//! | evaluate | `/api/repository/{space}/branch/{branch}/evaluate`  |
//!
//! When neither `space` nor `branch` annotation is present, the
//! builders fall back to the bare endpoint path. This path is
//! only valid inside the iframe bridge where the SW intercepts.
//! In top-level pages, the host should always have annotated
//! context — missing context will produce a 405.

const DEFAULT_SPACE: &str = "home";
const DEFAULT_BRANCH: &str = "main";

/// Build the `/query` URL — used by `tonk-query` and
/// `tonk-subscribe` (same endpoint, distinguished by `accept`
/// header).
pub(crate) fn query_url(space: Option<&str>, branch: Option<&str>) -> String {
    endpoint(space, branch, "query")
}

/// Build the `/transact` URL for `tonk-claim`.
pub(crate) fn transact_url(space: Option<&str>, branch: Option<&str>) -> String {
    endpoint(space, branch, "transact")
}

/// Build the `/evaluate` URL for `tonk-evaluate`.
pub(crate) fn evaluate_url(space: Option<&str>, branch: Option<&str>) -> String {
    endpoint(space, branch, "evaluate")
}

fn endpoint(space: Option<&str>, branch: Option<&str>, route: &str) -> String {
    match (space, branch) {
        (None, None) => format!("/{route}"),
        _ => format!(
            "/api/repository/{}/branch/{}/{route}",
            space.unwrap_or(DEFAULT_SPACE),
            branch.unwrap_or(DEFAULT_BRANCH),
        ),
    }
}
