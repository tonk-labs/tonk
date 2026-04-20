//! Per-repo view. Route param is the *local name* (storage key) of the
//! repo — distinct from the repo's own DID and from the subject DID it
//! tracks (see `project_repo_three_identifiers.md`). `/api/repositories`
//! will list entries keyed by this same local name.
//!
//! Current shape is a placeholder: renders the local name. The subject
//! DID (the identity the user actually thinks they're viewing) will be
//! fetched from `/api/repository/<name>/status` once that endpoint
//! surfaces it.

use leptos::{either::Either, prelude::*};
use leptos_router::{hooks::use_params, params::Params};

/// Route params for `/repo/:name?`.
#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkRepoParams {
    /// Local repo name (storage key).
    name: Option<String>,
}

/// Main area for a single repo.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkRepo() -> impl IntoView {
    let params = use_params::<TonkRepoParams>();
    let name = Signal::derive_local(move || params.get().ok().and_then(|p| p.name));

    view! {
        <section class="repo">
        {
            move || match name.get() {
                Some(name) => Either::Left(view! { <code class="local-name">{ name }</code> }),
                None => Either::Right(view! { <p class="empty">"Pick a repo from the sidebar."</p> }),
            }
        }
        </section>
    }
}
