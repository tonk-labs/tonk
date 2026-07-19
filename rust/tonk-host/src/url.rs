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
//!
//! A `profile` annotation (from `<tonk-repository profile>`) targets
//! the profile-as-repository surface (`/api/profile/branch/{branch}/…`)
//! instead of the named-repo namespace. The profile lives outside
//! `/api/repository/{name}`, so its routes are parallel; the `space`
//! name is irrelevant in profile mode.
//!
//! A branch with no space is not a route: outside profile mode the
//! repository segment is required, and there is no default space to
//! fill it with. `ops::route_from` rejects that pairing before it
//! reaches these builders, so `space = None` here means the bare
//! endpoint.

const DEFAULT_BRANCH: &str = "main";

/// Build the `/query` URL — used by `tonk-query` and
/// `tonk-subscribe` (same endpoint, distinguished by `accept`
/// header).
pub(crate) fn query_url(space: Option<&str>, branch: Option<&str>, profile: bool) -> String {
    endpoint(space, branch, profile, "query")
}

/// Build the `/transact` URL for `tonk-claim`.
pub(crate) fn transact_url(space: Option<&str>, branch: Option<&str>, profile: bool) -> String {
    endpoint(space, branch, profile, "transact")
}

/// Build the `/evaluate` URL for `tonk-evaluate`. When `transact`
/// is `false` the `?transact=false` query is appended so the
/// worker runs queries + planning but drops the commit — the
/// dry-run an editor uses to preview a half-typed buffer.
pub(crate) fn evaluate_url(
    space: Option<&str>,
    branch: Option<&str>,
    profile: bool,
    transact: bool,
) -> String {
    let url = endpoint(space, branch, profile, "evaluate");
    if transact {
        url
    } else {
        format!("{url}?transact=false")
    }
}

fn endpoint(space: Option<&str>, branch: Option<&str>, profile: bool, route: &str) -> String {
    if profile {
        return format!(
            "/api/profile/branch/{}/{route}",
            branch.unwrap_or(DEFAULT_BRANCH),
        );
    }
    match space {
        Some(space) => format!(
            "/api/repository/{space}/branch/{}/{route}",
            branch.unwrap_or(DEFAULT_BRANCH),
        ),
        None => format!("/{route}"),
    }
}
