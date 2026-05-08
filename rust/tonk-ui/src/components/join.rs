use leptos::{
    ev::{Event, SubmitEvent},
    prelude::*,
    task::spawn_local,
    web_sys,
};
use leptos_router::{NavigateOptions, hooks::use_navigate};
use tonk_invite::{Invite, InviteAudience};
use tonk_worker::JoinResponse;
use url::Url;
use wasm_bindgen::JsCast;

use crate::{
    api::{self, JoinError},
    components::{LastJoinOutcome, ProfileResource, Status},
    did,
};

/// Lifecycle of a `/join` page load.
///
/// The component computes this once after parsing the invite +
/// reading the profile, then either fast-paths through (already-
/// have-it) or hands the user a form to confirm a fresh name.
#[derive(Clone, Debug)]
enum JoinView {
    /// Invite parse + profile load haven't both completed yet.
    Loading,
    /// Invite URL was malformed or the seed-audience check failed.
    /// String is the underlying error.
    InvalidInvite(String),
    /// Scoped invite whose chain audience is some DID other than
    /// the active profile's. The recipient can't redeem it.
    AudienceMismatch {
        /// DID of the intended recipient (chain's tail audience).
        audience: String,
        /// Subject DID of the space the invite is for. Surfaced
        /// so we can still render the sigil — the user knows
        /// which space they were being pointed at, even though
        /// they can't redeem it.
        subject: String,
    },
    /// The invited subject is already mounted locally — auto-
    /// submitting `/api/profile/join` to refresh the chain, then
    /// navigating. Renders a "joining…" affordance with the sigil
    /// of the destination space.
    AlreadyMember {
        /// Local name of the existing replica we'll land in.
        name: String,
        /// Subject DID, for the sigil.
        subject: String,
    },
    /// Fresh subject — no replica yet. Render the form with the
    /// sigil so the recipient can confirm what they're joining.
    NewMember {
        /// Subject DID, for the sigil + a hint of what they're joining.
        subject: String,
    },
}

/// Suggested name extracted from a `?name=...` query param on the
/// current URL. Returns `None` when the param is missing or empty.
fn suggested_name_from_url(href: &str) -> Option<String> {
    Url::parse(href)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "name")
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Post-claim navigation suffix extracted from a `?then=...`
/// query param on the current URL.
///
/// The value is treated as a path *under the recipient's
/// `/space/<name>/` root* — it must not start with `/` (no
/// absolute paths) and must not contain a URL scheme (no
/// `http://...`). The caller composes the final destination
/// as `/space/<recipient-name>/<suffix>` once the actual local
/// name is known. This keeps the share URL valid regardless of
/// whether the recipient renamed the space on the join form or
/// already had the subject mounted under a different name.
fn then_suffix_from_url(href: &str) -> Option<String> {
    Url::parse(href)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "then")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty() && !value.starts_with('/') && !value.contains("://"))
}

/// Build the post-claim destination from the recipient's local
/// space name and an optional `then=` suffix. Falls back to the
/// space root (`/space/<name>`) when no suffix is present.
fn compose_destination(space_name: &str, then_suffix: Option<&str>) -> String {
    match then_suffix {
        Some(suffix) => format!("/space/{}/{}", space_name, suffix),
        None => format!("/space/{}", space_name),
    }
}

/// Sigil hex string for a DID, suitable for `<tonk-sigil value=...>`.
/// Matches the helper used in [`super::space`] so a space's sigil
/// is consistent across the join page and the space view.
fn did_sigil_value(did: &str) -> Option<String> {
    did::did_key_prefix(did).map(|bytes| {
        let n = u32::from_be_bytes(bytes);
        format!("0x{n:08x}")
    })
}

/// `/join` view. Parses the invite client-side, decides whether the
/// recipient already has the invited subject, and either:
/// - fast-paths through `/api/profile/join` and navigates (the
///   recipient already has the space — they get the chain refresh
///   silently), or
/// - renders a form so they can confirm a local name (fresh
///   subject — they're joining for the first time).
///
/// The full URL (including any `#fragment`, which carries the
/// ephemeral seed for audience-open invites) is read from
/// `window.location.href` and forwarded verbatim to the worker —
/// browsers do not transmit fragments with `fetch`, so the worker
/// would otherwise never see the seed.
///
/// All worker calls are gated on [`Status::Ready`] so deep-link
/// loads don't race the service worker's startup.
#[component]
pub fn TonkJoin() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>();
    let profile_resource =
        use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");
    let last_join_outcome =
        use_context::<LastJoinOutcome>().expect("LastJoinOutcome provided by TonkShell");
    let navigate = use_navigate();

    // Capture the full URL once at mount. The fragment is part of
    // the invite and must not be lost as we re-render.
    let invite_url = window().location().href().unwrap_or_else(|_| String::new());

    let suggested = suggested_name_from_url(&invite_url).unwrap_or_default();
    // Optional post-claim navigation suffix. Lets the inviter
    // (e.g. `slide share concept`) drop the recipient on a
    // specific page within the joined space — e.g. a concept
    // view — instead of the default space root. The value is a
    // path suffix under `/space/<name>/`; the actual local
    // name (resolved post-claim) gets prefixed at navigation
    // time. Absent or malformed values fall through.
    let then_suffix = then_suffix_from_url(&invite_url);
    let name = RwSignal::new(suggested);
    let error = RwSignal::new(Option::<String>::None);
    let submitting = RwSignal::new(false);

    // Parse + validate the invite asynchronously (Invite::parse_url
    // does the seed-audience check for open invites). Holds:
    //   - Ok(Invite) on success
    //   - Err(message) on parse / validation failure
    //   - None while the future is pending
    let parsed_invite: RwSignal<Option<Result<Invite, String>>> = RwSignal::new(None);
    {
        let url = invite_url.clone();
        spawn_local(async move {
            match Invite::parse_url(&url).await {
                Ok(invite) => parsed_invite.set(Some(Ok(invite))),
                Err(e) => parsed_invite.set(Some(Err(format!("{e}")))),
            }
        });
    }

    // Derive the lifecycle view from (parsed invite, profile).
    let join_view = Signal::derive_local(move || {
        let parsed = match parsed_invite.get() {
            None => return JoinView::Loading,
            Some(Err(e)) => return JoinView::InvalidInvite(e),
            Some(Ok(invite)) => invite,
        };

        // Scoped audience check: if the invite is bound to a
        // specific DID and that DID isn't the active profile's,
        // the recipient can't redeem it. Open invites have an
        // ephemeral audience and any redeemer can claim — skip
        // the check there.
        let chain_audience = parsed.chain.audience().to_string();
        if matches!(parsed.audience, InviteAudience::Scoped) {
            // Wait for the profile so we know our own DID before
            // committing to a mismatch verdict.
            let profile_info = match profile_resource.get() {
                Some(Ok(Some(info))) => info,
                Some(Ok(None)) | None => return JoinView::Loading,
                Some(Err(e)) => return JoinView::InvalidInvite(format!("{e}")),
            };
            if chain_audience != profile_info.profile.subject.to_string() {
                return JoinView::AudienceMismatch {
                    audience: chain_audience,
                    subject: parsed.subject().to_string(),
                };
            }
        }

        let subject = parsed.subject().to_string();

        // Profile may still be loading. If we're past the scoped
        // check above we already have it; if this is an open
        // invite, fetch it here.
        let profile_info = match profile_resource.get() {
            Some(Ok(Some(info))) => info,
            Some(Ok(None)) | None => return JoinView::Loading,
            Some(Err(e)) => return JoinView::InvalidInvite(format!("{e}")),
        };

        // Reverse-lookup the subject in the profile's space map.
        // If found, the recipient already has this space mounted
        // locally — they should land back in it (with a refreshed
        // delegation chain) without re-naming anything.
        let existing = profile_info
            .space
            .iter()
            .find(|(_, did)| did.to_string() == subject)
            .map(|(name, _)| name.clone());

        match existing {
            Some(name) => JoinView::AlreadyMember { name, subject },
            None => JoinView::NewMember { subject },
        }
    });

    // Fast-path the `AlreadyMember` case: as soon as the view
    // resolves to it, fire `/api/profile/join` (chain save +
    // `Renewed` outcome) and navigate to the existing space.
    // Tracked separately so we don't double-fire if `join_view`
    // re-derives.
    let auto_joined = RwSignal::new(false);
    {
        let navigate = navigate.clone();
        let invite_url = invite_url.clone();
        let then_suffix = then_suffix.clone();
        Effect::new(move |_| {
            if auto_joined.get() {
                return;
            }
            let JoinView::AlreadyMember { name, .. } = join_view.get() else {
                return;
            };
            let ready = status.map(|s| s.get() == Status::Ready).unwrap_or(false);
            if !ready {
                return;
            }
            auto_joined.set(true);

            let url = invite_url.clone();
            let target = name.clone();
            let navigate = navigate.clone();
            let then_suffix = then_suffix.clone();
            spawn_local(async move {
                match api::join(&url, &target).await {
                    Ok(response) => {
                        let outcome = match &response {
                            JoinResponse::Joined { .. } => "joined",
                            JoinResponse::Renewed { .. } => "renewed",
                        };
                        last_join_outcome.set(Some(outcome));
                        profile_resource.refetch();
                        let destination = compose_destination(&target, then_suffix.as_deref());
                        navigate(&destination, NavigateOptions::default());
                    }
                    Err(e) => {
                        // The chain save failed (network, 5xx, or
                        // even a name collision somehow). Surface
                        // it inline so the user has visibility
                        // even though the auto-path swallowed the
                        // explicit form.
                        let message = match e {
                            JoinError::NameTaken => format!(
                                "Name '{}' is taken — please refresh and try again.",
                                target,
                            ),
                            JoinError::Other(err) => format!("{err}"),
                        };
                        error.set(Some(message));
                        auto_joined.set(false);
                    }
                }
            });
        });
    }

    let on_input = move |event: Event| {
        let value = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
            .and_then(|el| {
                js_sys::Reflect::get(&el, &wasm_bindgen::JsValue::from_str("value"))
                    .ok()
                    .and_then(|v| v.as_string())
            })
            .unwrap_or_default();
        name.set(value);
    };

    let on_cancel = {
        let navigate = navigate.clone();
        move |_| {
            navigate("/", NavigateOptions::default());
        }
    };

    let submit = {
        let navigate = navigate.clone();
        let invite_url = invite_url.clone();
        let then_suffix = then_suffix.clone();
        move |event: SubmitEvent| {
            event.prevent_default();

            let ready = status.map(|s| s.get() == Status::Ready).unwrap_or(false);
            if !ready {
                return;
            }
            // Only valid in the `NewMember` branch — the form
            // doesn't render in any other state, but a stray
            // Enter on the input could still fire here.
            if !matches!(join_view.get(), JoinView::NewMember { .. }) {
                return;
            }

            let requested = name.get().trim().to_string();
            if requested.is_empty() {
                error.set(Some("Name can't be empty".to_string()));
                return;
            }

            error.set(None);
            submitting.set(true);
            let navigate = navigate.clone();
            let url = invite_url.clone();
            let then_suffix = then_suffix.clone();
            spawn_local(async move {
                match api::join(&url, &requested).await {
                    Ok(response) => {
                        submitting.set(false);
                        profile_resource.refetch();
                        let (target_name, outcome) = match &response {
                            JoinResponse::Joined { repository } => {
                                (repository.name.clone(), "joined")
                            }
                            JoinResponse::Renewed { repository } => {
                                (repository.name.clone(), "renewed")
                            }
                        };
                        last_join_outcome.set(Some(outcome));
                        let destination = compose_destination(&target_name, then_suffix.as_deref());
                        navigate(&destination, NavigateOptions::default());
                    }
                    Err(JoinError::NameTaken) => {
                        submitting.set(false);
                        error.set(Some(format!(
                            "A space named '{}' already exists. Pick a different name.",
                            requested
                        )));
                    }
                    Err(JoinError::Other(e)) => {
                        submitting.set(false);
                        error.set(Some(format!("{e}")));
                    }
                }
            });
        }
    };

    let ready_signal = move || status.map(|s| s.get() == Status::Ready).unwrap_or(false);

    view! {
        <main class="join-view">
            <wa-card class="join-card">
                { move || render_body(
                    join_view.get(),
                    name,
                    error,
                    submitting,
                    ready_signal(),
                    on_input,
                    submit.clone(),
                    on_cancel.clone(),
                ) }
            </wa-card>
        </main>
    }
}

/// Render the card body for the current [`JoinView`] state.
///
/// Pulled into a standalone function so each branch is locally
/// readable; the closures it takes are the same ones the
/// component sets up, threaded through.
#[allow(clippy::too_many_arguments)]
fn render_body<I, S, C>(
    state: JoinView,
    name: RwSignal<String>,
    error: RwSignal<Option<String>>,
    submitting: RwSignal<bool>,
    ready: bool,
    on_input: I,
    submit: S,
    on_cancel: C,
) -> impl IntoView
where
    I: Fn(Event) + 'static + Clone,
    S: Fn(SubmitEvent) + 'static + Clone,
    C: Fn(leptos::ev::MouseEvent) + 'static + Clone,
{
    use leptos::either::EitherOf5;
    match state {
        JoinView::Loading => EitherOf5::A(view! {
            <div slot="header">
                <h1>"Joining…"</h1>
            </div>
            <wa-spinner></wa-spinner>
        }),
        JoinView::InvalidInvite(message) => EitherOf5::B(view! {
            <div slot="header">
                <h1>"Invite link is invalid"</h1>
            </div>
            <wa-callout variant="danger">
                <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                { message }
            </wa-callout>
            <wa-button
                slot="footer"
                variant="neutral"
                appearance="plain"
                on:click=on_cancel
            >"Back"</wa-button>
        }),
        JoinView::AudienceMismatch { audience, subject } => {
            let sigil_value = did_sigil_value(&subject);
            EitherOf5::C(view! {
                <div slot="header">
                    <h1>"This invite is for someone else"</h1>
                </div>
                <div class="join-target wa-stack wa-gap-s wa-align-items-center">
                    <tonk-sigil value=sigil_value></tonk-sigil>
                    <p>
                        "This invite was issued to "
                        <code>{ audience }</code>
                        ", which is not your current profile."
                    </p>
                    <p>
                        "Ask the inviter to issue a new invite for your DID, "
                        "or switch to the identity the invite was issued to."
                    </p>
                </div>
            })
        }
        JoinView::AlreadyMember { name: _, subject } => {
            let sigil_value = did_sigil_value(&subject);
            EitherOf5::D(view! {
                <div slot="header">
                    <h1>"Joining…"</h1>
                </div>
                <div class="join-target wa-stack wa-gap-s wa-align-items-center">
                    <tonk-sigil value=sigil_value></tonk-sigil>
                    <p>"You're already a member of this space — taking you there now."</p>
                    { move || error.get().map(|message| view! {
                        <wa-callout variant="danger">
                            <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                            { message }
                        </wa-callout>
                    }) }
                </div>
            })
        }
        JoinView::NewMember { subject } => {
            let sigil_value = did_sigil_value(&subject);
            let subject_for_title = subject.clone();
            EitherOf5::E(view! {
                <div slot="header">
                    <h1>"Join space"</h1>
                    <p class="join-subtitle">
                        "Pick a local name for this space. The inviter's "
                        "suggestion is filled in below; you can rename "
                        "it before joining."
                    </p>
                </div>
                <form on:submit=submit>
                    <div class="wa-stack wa-gap-s">
                        <div class="join-target wa-cluster wa-gap-s wa-align-items-center">
                            <tonk-sigil value=sigil_value></tonk-sigil>
                            <code class="join-subject" title=subject_for_title>{ subject }</code>
                        </div>
                        <wa-input
                            name="space-name"
                            label="Local name"
                            placeholder="e.g. team-foo"
                            autocomplete="off"
                            autofocus
                            required
                            prop:value=move || name.get()
                            on:input=on_input
                        ></wa-input>
                        { move || error.get().map(|message| view! {
                            <wa-callout variant="danger">
                                <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                                { message }
                            </wa-callout>
                        }) }
                    </div>
                    <div slot="footer" class="join-actions">
                        <wa-button
                            type="button"
                            variant="neutral"
                            appearance="plain"
                            on:click=on_cancel
                        >"Cancel"</wa-button>
                        <wa-button
                            type="submit"
                            variant="primary"
                            prop:loading=move || submitting.get()
                            prop:disabled=move || submitting.get() || !ready
                        >"Join"</wa-button>
                    </div>
                </form>
            })
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{compose_destination, suggested_name_from_url, then_suffix_from_url};

    #[test]
    fn it_extracts_a_then_suffix_from_a_well_formed_url() {
        let url = "https://ui.example.test/join?access=abc&then=branch/main/concept/task#seed";
        assert_eq!(
            then_suffix_from_url(url).as_deref(),
            Some("branch/main/concept/task"),
        );
    }

    #[test]
    fn it_drops_a_then_value_that_is_absolute_or_off_site() {
        // Absolute paths, full URLs, and missing values are
        // silently dropped — the suffix model assumes the value
        // is a path *under* `/space/<name>/`, so anything else
        // would compose into a broken navigation target.
        for href in [
            "https://ui.example.test/join?access=abc&then=/space/shared/foo",
            "https://ui.example.test/join?access=abc&then=https://evil.test/x",
            "https://ui.example.test/join?access=abc",
        ] {
            assert!(
                then_suffix_from_url(href).is_none(),
                "expected None for {href}",
            );
        }
    }

    #[test]
    fn it_keeps_then_and_name_independent() {
        // Both parameters can coexist and are read independently.
        let url =
            "https://ui.example.test/join?name=tasks&access=abc&then=branch/main/concept/task";
        assert_eq!(suggested_name_from_url(url).as_deref(), Some("tasks"));
        assert_eq!(
            then_suffix_from_url(url).as_deref(),
            Some("branch/main/concept/task"),
        );
    }

    #[test]
    fn it_composes_a_destination_from_the_recipient_local_name() {
        // The crucial property: the actual space name (whatever
        // the recipient ended up with) is what shows up in the
        // composed URL — *not* the inviter's suggestion. This
        // is what keeps the share working when the recipient
        // already had the subject mounted under another name.
        assert_eq!(
            compose_destination("home", Some("branch/main/concept/task")),
            "/space/home/branch/main/concept/task",
        );
        assert_eq!(compose_destination("home", None), "/space/home");
    }
}
