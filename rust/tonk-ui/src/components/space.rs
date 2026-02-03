use leptos::{either::Either, prelude::*};
use leptos_router::{hooks::use_params, params::Params};

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkSpaceParams {
    did: Option<String>,
}

/// Main workspace area for displaying Tonks.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpace() -> impl IntoView {
    let params = use_params::<TonkSpaceParams>();
    let did = Signal::derive_local(move || {
        params
            .get()
            .map(|params| params.did)
            .ok()
            .unwrap_or_default()
    });

    view! {
        <section class="space">
        {
            move || match did.get() {
                Some(did) => Either::Left(view! {
                    <span>{ did }</span>
                }),
                None => Either::Right(view! {
                    <span>"No DID!"</span>
                }),
            }
        }
        </section>
    }
}
