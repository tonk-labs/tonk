use leptos::{
    ev::{Event, SubmitEvent},
    prelude::*,
    task::spawn_local,
    web_sys,
};
use wasm_bindgen::JsCast;

use crate::{
    api::{self, CreateSpaceError},
    components::CreateSpaceOpen,
};

/// Modal dialog for creating a new space.
///
/// Rendered once at the launcher level. Visibility is driven by
/// the shared [`CreateSpaceOpen`] signal — the "New space" button
/// flips it to `true`, the dialog flips it back to `false` after a
/// successful create or on Cancel. On success the dialog just closes;
/// the Hub's card for the new space appears (in its installing state)
/// over the profile subscription, so there's nothing to navigate to.
#[component]
pub fn TonkCreateSpace() -> impl IntoView {
    let open = use_context::<CreateSpaceOpen>().expect("CreateSpaceOpen provided by TonkShell");

    let name = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let submitting = RwSignal::new(false);

    // `wa-dialog` fires `wa-after-hide` when the dialog finishes
    // animating shut. Listening here lets the Esc key / light
    // dismiss (also clicking the X) flip our shared `open` signal
    // back to `false` so the next press of `+` reopens it.
    let on_after_hide = move |_: Event| {
        open.set(false);
        name.set(String::new());
        error.set(None);
        submitting.set(false);
    };

    let submit = move |event: SubmitEvent| {
        event.prevent_default();
        let requested = name.get().trim().to_string();
        if requested.is_empty() {
            error.set(Some("Name can't be empty".to_string()));
            return;
        }

        error.set(None);
        submitting.set(true);
        spawn_local(async move {
            match api::create_space(&requested).await {
                Ok(_) => {
                    // Stay on the Hub: the new space's card appears in
                    // its installing state and settles to clickable when
                    // the background seed completes. Navigating now would
                    // land in a not-yet-seeded space.
                    open.set(false);
                    name.set(String::new());
                    submitting.set(false);
                }
                Err(CreateSpaceError::AlreadyExists) => {
                    submitting.set(false);
                    error.set(Some(format!(
                        "A space named '{}' already exists",
                        requested
                    )));
                }
                Err(CreateSpaceError::Other(e)) => {
                    submitting.set(false);
                    error.set(Some(format!("{e}")));
                }
            }
        });
    };

    let on_cancel = move |_| {
        open.set(false);
    };

    // `wa-input` fires a native `input` event whose `target.value`
    // carries the current text. We mirror it into the `name`
    // signal so the submit handler can read a live value.
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

    view! {
        <wa-dialog
            label="Create space"
            prop:open=move || open.get()
            on:wa-after-hide=on_after_hide
        >
            <form on:submit=submit>
                <div class="wa-stack wa-gap-s">
                    <wa-input
                        name="space-name"
                        label="Space name"
                        placeholder="e.g. pictures"
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
                <wa-button
                    slot="footer"
                    variant="neutral"
                    appearance="plain"
                    type="button"
                    on:click=on_cancel
                >"Cancel"</wa-button>
                <wa-button
                    slot="footer"
                    variant="primary"
                    type="submit"
                    prop:loading=move || submitting.get()
                    prop:disabled=move || submitting.get()
                >"Create"</wa-button>
            </form>
        </wa-dialog>
    }
}
