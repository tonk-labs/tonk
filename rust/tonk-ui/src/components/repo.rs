//! Per-repo view. Route param is the subject DID of the repo the profile
//! has access to — the same string used as the local repo name, and the
//! key under which `/api/repositories` will list this entry.
//!
//! Current shape is a placeholder: renders the DID and nothing else. Data
//! rendering slots in once the read API stabilizes on the new
//! dialog-artifacts shape.

use leptos::{either::Either, prelude::*};
use leptos_router::{hooks::use_params, params::Params};

/// Route params for `/repo/:did?`.
#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkRepoParams {
    /// Subject DID of the repo being viewed.
    did: Option<String>,
}

/// Main area for a single repo.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkRepo() -> impl IntoView {
    let params = use_params::<TonkRepoParams>();
    let did = Signal::derive_local(move || params.get().ok().and_then(|p| p.did));

    view! {
        <section class="repo">
        {
            move || match did.get() {
                Some(did) => Either::Left(view! { <code class="did">{ did }</code> }),
                None => Either::Right(view! { <p class="empty">"Pick a repo from the sidebar."</p> }),
            }
        }
        </section>
    }
}
