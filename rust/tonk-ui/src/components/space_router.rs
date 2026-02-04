//! Space router component - displays the currently selected space.

use leptos::{logging::log, prelude::*};
use leptos_router::hooks::use_params_map;

use crate::api;
use crate::components::{TonkSpace, TonkToolbar};

/// The current space's multikey, provided via context.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentSpace(pub String);

impl CurrentSpace {
    /// Get the multikey (z6Mk...).
    pub fn multikey(&self) -> &str {
        &self.0
    }

    /// Get the full DID (did:key:z6Mk...).
    pub fn did(&self) -> String {
        if self.0.starts_with("did:key:") {
            self.0.clone()
        } else {
            format!("did:key:{}", self.0)
        }
    }
}

/// Space initialization status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpaceStatus {
    /// Space is being initialized.
    Loading,
    /// Space is ready.
    Ready,
    /// Space initialization failed.
    Error(String),
}

/// Routes to a specific space based on the URL path.
///
/// Extracts the multikey from `/{multikey}/` and provides it as context
/// to child components. Also handles space-specific initialization like
/// setting up the upstream remote.
#[component]
pub fn SpaceRouter() -> impl IntoView {
    let params = use_params_map();

    // Get multikey from URL params
    let multikey = Memo::new(move |_| params.read().get("multikey").unwrap_or_default());

    // Initialize the space: check status, setup remote if needed
    let space_init = LocalResource::new(move || {
        let mk = multikey.get();
        async move {
            if mk.is_empty() {
                return Err(crate::error::TonkUiError::ApiError(
                    "No space selected".to_string(),
                ));
            }

            log!("Initializing space: {}", mk);
            let status = api::status(&mk).await?;

            // If no upstream configured, add the remote
            if !status.has_upstream {
                log!("No upstream configured for {}, adding remote...", mk);
                api::authorize(&mk).await?;
                log!("Remote added successfully for {}", mk);
            }

            Ok::<_, crate::error::TonkUiError>(())
        }
    });

    // Derive space status from init resource
    let space_status = Signal::derive_local(move || match space_init.get() {
        Some(Ok(())) => SpaceStatus::Ready,
        Some(Err(e)) => SpaceStatus::Error(format!("{:?}", e)),
        None => SpaceStatus::Loading,
    });

    // Provide space context to children
    let current_space = Signal::derive(move || CurrentSpace(multikey.get()));
    provide_context(current_space);
    provide_context(space_status);

    view! {
        <section class="launcher">
            <TonkToolbar />
            <TonkSpace />
        </section>
    }
}
