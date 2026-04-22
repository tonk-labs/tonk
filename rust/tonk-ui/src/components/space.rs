use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};

use crate::api;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkSpaceParams {
    space: Option<String>,
}

const DEFAULT_BRANCH: &str = "main";

/// Main workspace area for displaying a repository.
///
/// Fetches the repository record at `/api/repository/{space}`, then
/// kicks off a pull of `main` from upstream so the local state
/// reflects what's on the access service. Below that, an ad-hoc
/// query form drives `/claim/select` for exploration.
///
/// If `:space` is missing, redirects to `/space/{DEFAULT_REPO}`.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpace() -> impl IntoView {
    let params = use_params::<TonkSpaceParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
    });

    let repository = LocalResource::new(move || {
        let name = space_name.get();
        async move {
            match name {
                None => {
                    BrowserUrl::redirect(&format!("/space/{}", api::DEFAULT_REPO));
                    Ok(None)
                }
                Some(name) => api::repository(&name).await,
            }
        }
    });

    // Auto-pull main when the space changes. Tracks `space_name` so
    // navigating to a different repo reruns the pull for that one.
    let pull_trigger = RwSignal::new(0u32);
    let pull = LocalResource::new(move || {
        let name = space_name.get();
        // Read pull_trigger so manual re-pulls refetch the resource.
        let _ = pull_trigger.get();
        async move {
            let Some(name) = name else {
                return Ok::<_, String>(None);
            };
            api::pull(&name, DEFAULT_BRANCH)
                .await
                .map(Some)
                .map_err(|e| format!("{e}"))
        }
    });
    let repull = move |_| pull_trigger.update(|n| *n = n.wrapping_add(1));

    // Query form: two inputs + a submit signal. The claims resource
    // fires only when the submitted query is non-empty.
    let the_input = RwSignal::new(String::new());
    let of_input = RwSignal::new(String::new());
    let submitted = RwSignal::new(None::<(Option<String>, Option<String>)>);

    let claims = LocalResource::new(move || {
        let name = space_name.get();
        let query = submitted.get();
        async move {
            match (name, query) {
                (Some(name), Some((the, of))) => {
                    api::select_claims(&name, DEFAULT_BRANCH, the.as_deref(), of.as_deref())
                        .await
                        .map(|r| Some(r.claims))
                        .map_err(|e| format!("{e}"))
                }
                _ => Ok(None),
            }
        }
    });

    let submit_query = move |_| {
        let the = the_input.get_untracked().trim().to_string();
        let of = of_input.get_untracked().trim().to_string();
        if the.is_empty() && of.is_empty() {
            return;
        }
        submitted.set(Some((
            (!the.is_empty()).then_some(the),
            (!of.is_empty()).then_some(of),
        )));
    };

    // Reset query state when navigating to a different space.
    Effect::new(move |prev: Option<Option<String>>| {
        let current = space_name.get();
        if let Some(p) = prev
            && p != current
        {
            the_input.set(String::new());
            of_input.set(String::new());
            submitted.set(None);
        }
        current
    });

    view! {
        <section class="space">
            <Suspense fallback=|| view! { <span class="loading">"Loading…"</span> }>
                <ErrorBoundary fallback=|errors| view! {
                    <section class="error">
                        { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                    </section>
                }>
                    { move || repository.get().map(|result| result.map(|repo| match repo {
                        Some(status) => Either::Left(view! {
                            <pre class="repository">
                                { serde_json::to_string_pretty(&status).unwrap_or_default() }
                            </pre>
                        }),
                        None => Either::Right(view! {
                            <section class="not-found">
                                { move || format!(
                                    "Repository '{}' not found",
                                    space_name.get().unwrap_or_default(),
                                ) }
                            </section>
                        }),
                    })) }
                </ErrorBoundary>
            </Suspense>

            <section class="sync">
                <h2>"Upstream"</h2>
                <p class="status">
                    {move || match pull.get() {
                        None => "pulling…".to_string(),
                        Some(Err(e)) => format!("error: {e}"),
                        Some(Ok(None)) => String::new(),
                        Some(Ok(Some(resp))) => {
                            if resp.success {
                                "pulled".to_string()
                            } else {
                                resp.error.unwrap_or_else(|| "pull failed".into())
                            }
                        }
                    }}
                </p>
                <button on:click=repull>"Pull again"</button>
            </section>

            <section class="claims">
                <h2>"Claims"</h2>
                <form on:submit=move |ev| { ev.prevent_default(); submit_query(()); }>
                    <input
                        type="text"
                        placeholder="attribute (e.g. user/name)"
                        prop:value=move || the_input.get()
                        on:input=move |ev| the_input.set(event_target_value(&ev))
                    />
                    <input
                        type="text"
                        placeholder="entity (e.g. did:key:…)"
                        prop:value=move || of_input.get()
                        on:input=move |ev| of_input.set(event_target_value(&ev))
                    />
                    <button type="submit">"Query"</button>
                </form>
                {move || match claims.get() {
                    None => view! { <p class="hint">"Submit to query."</p> }.into_any(),
                    Some(Ok(None)) => view! { <p class="hint">"Submit to query."</p> }.into_any(),
                    Some(Err(e)) => view! { <p class="error">{e}</p> }.into_any(),
                    Some(Ok(Some(list))) if list.is_empty() => view! {
                        <p class="hint">"No claims matched."</p>
                    }.into_any(),
                    Some(Ok(Some(list))) => view! {
                        <ul class="claims-list">
                            {list.into_iter().map(|c| view! {
                                <li>
                                    <code class="the">{c.the}</code>
                                    " of "
                                    <code class="of">{c.of}</code>
                                    " is "
                                    <code class="is">
                                        { serde_json::to_string(&c.is).unwrap_or_default() }
                                    </code>
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any(),
                }}
            </section>
        </section>
    }
}
