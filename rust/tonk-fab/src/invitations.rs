//! Targeted invitation minting, listing, and revocation controls.

use custom_elements::CustomElement;
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlElement, HtmlInputElement, window};

use tonk_worker_api::{
    CreateInviteRequest, CreateInviteResponse, InvitationKind, InvitationSummary,
    RevokeInvitationAcknowledgement,
};

use crate::logic::{create_invitation_endpoint, invitations_endpoint, revoke_invitation_endpoint};
use crate::share::{PendingClipboard, open_deferred_clipboard_write};

#[derive(Default)]
pub(crate) struct TonkInvitations {
    submit: Option<Closure<dyn FnMut(Event)>>,
    click: Option<Closure<dyn FnMut(Event)>>,
}

impl CustomElement for TonkInvitations {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(
            r#"<form class="fab__target-invite">
  <label>Invite identity
    <input name="recipientRoot" type="text" placeholder="did:key:…" autocomplete="off">
  </label>
  <button type="submit">Invite identity</button>
</form>
<p class="fab__invitation-status" role="status" aria-live="polite"></p>
<ul class="fab__invitation-list"></ul>"#,
        );
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let submit_host = this.clone();
        let submit = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            event.prevent_default();
            // Spend transient user activation now. The clipboard holds this
            // promise-backed write open until the HTTP mint returns its URL.
            let clipboard = open_deferred_clipboard_write().ok();
            mint_targeted(submit_host.clone(), clipboard);
        });
        if let Ok(Some(form)) = this.query_selector(".fab__target-invite") {
            let _ =
                form.add_event_listener_with_callback("submit", submit.as_ref().unchecked_ref());
        }
        self.submit = Some(submit);

        let click_host = this.clone();
        let click = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(cid) = target.get_attribute("data-revoke-invitation") else {
                return;
            };
            event.prevent_default();
            revoke(click_host.clone(), cid);
        });
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        self.click = Some(click);
        refresh(this.clone());
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        refresh(this.clone());
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        self.submit = None;
    }
}

fn origin() -> Option<String> {
    tonk_host::bridge::context_origin()
}

fn status(host: &HtmlElement, message: &str) {
    if let Ok(Some(element)) = host.query_selector(".fab__invitation-status") {
        element.set_text_content(Some(message));
    }
}

fn refresh(host: HtmlElement) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(path) = invitations_endpoint(&space) else {
        return;
    };
    let Some(origin) = origin() else {
        return;
    };
    spawn_local(async move {
        let result = reqwest::Client::new()
            .get(format!("{origin}{path}"))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => {
                match response.json::<Vec<InvitationSummary>>().await {
                    Ok(rows) => render(&host, &rows),
                    Err(_) => status(&host, "Invitation list is unavailable."),
                }
            }
            _ => status(&host, "Invitation list is unavailable."),
        }
    });
}

fn mint_targeted(host: HtmlElement, clipboard: Option<PendingClipboard>) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(path) = create_invitation_endpoint(&space) else {
        return;
    };
    let Some(origin) = origin() else {
        return;
    };
    let recipient: Option<HtmlInputElement> = host
        .query_selector("[name=\"recipientRoot\"]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into().ok());
    let Some(recipient) = recipient else {
        return;
    };
    let Ok(recipient_root) = recipient.value().trim().parse() else {
        status(&host, "Enter a valid root DID.");
        return;
    };
    status(&host, "Creating invitation…");
    spawn_local(async move {
        let mut clipboard = clipboard;
        let request = CreateInviteRequest {
            base_url: format!("{origin}/join").parse().ok(),
            recipient_root: Some(recipient_root),
        };
        let result = reqwest::Client::new()
            .post(format!("{origin}{path}"))
            .json(&request)
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => {
                match response.json::<CreateInviteResponse>().await {
                    Ok(minted) => {
                        let copied = clipboard.take().map(|pending| {
                            pending.resolve(minted.url().as_str());
                        });
                        recipient.set_value("");
                        status(
                            &host,
                            if copied.is_some() {
                                "Targeted invitation copied."
                            } else {
                                "Targeted invitation created, but clipboard access is unavailable."
                            },
                        );
                        refresh(host);
                    }
                    Err(_) => {
                        if let Some(pending) = clipboard.take() {
                            pending.reject("invalid invitation response");
                        }
                        status(&host, "The invitation response was invalid.");
                    }
                }
            }
            Ok(response) => {
                if let Some(pending) = clipboard.take() {
                    pending.reject("invitation request failed");
                }
                status(
                    &host,
                    &format!("Could not create invitation ({}).", response.status()),
                );
            }
            Err(_) => {
                if let Some(pending) = clipboard.take() {
                    pending.reject("invitation request failed");
                }
                status(&host, "Could not create invitation.");
            }
        }
    });
}

fn revoke(host: HtmlElement, target_cid: String) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(path) = revoke_invitation_endpoint(&space, &target_cid) else {
        return;
    };
    let Some(origin) = origin() else {
        return;
    };
    status(&host, "Revoking invitation…");
    spawn_local(async move {
        let result = reqwest::Client::new()
            .post(format!("{origin}{path}"))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => {
                match response.json::<RevokeInvitationAcknowledgement>().await {
                    Ok(acknowledgement) if acknowledgement.published => {
                        status(
                            &host,
                            if acknowledgement.stored {
                                "Invitation revoked."
                            } else {
                                "Invitation was already revoked."
                            },
                        );
                        mark_revoked(&host, &target_cid);
                    }
                    _ => status(&host, "The relay did not confirm publication."),
                }
            }
            Ok(response) => status(
                &host,
                &format!("Could not revoke invitation ({}).", response.status()),
            ),
            Err(_) => status(&host, "Could not revoke invitation."),
        }
    });
}

fn render(host: &HtmlElement, rows: &[InvitationSummary]) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector(".fab__invitation-list") else {
        return;
    };
    list.set_inner_html("");
    for row in rows {
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("data-invitation-cid", &row.target_cid);
        let recipient = row
            .recipient_root
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| match row.kind {
                InvitationKind::Open => "open invitation".to_string(),
                InvitationKind::Scoped => "scoped invitation".to_string(),
                InvitationKind::Unknown => "legacy invitation".to_string(),
            });
        let Ok(label) = document.create_element("span") else {
            continue;
        };
        label.set_text_content(Some(&format!("{recipient} · {}", row.status)));
        let _ = item.append_child(&label);
        if row.status == "active" {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("data-revoke-invitation", &row.target_cid);
            button.set_text_content(Some("Revoke"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

fn mark_revoked(host: &HtmlElement, target_cid: &str) {
    let selector = format!("[data-invitation-cid=\"{target_cid}\"]");
    if let Ok(Some(item)) = host.query_selector(&selector) {
        if let Ok(Some(button)) = item.query_selector("[data-revoke-invitation]") {
            button.remove();
        }
        let existing = item.text_content().unwrap_or_default();
        item.set_text_content(Some(&format!("{existing} · revoked")));
    }
}

pub(crate) fn register() {
    let Some(elements) = window().map(|window| window.custom_elements()) else {
        return;
    };
    if elements.get("tonk-invitations").is_undefined() {
        TonkInvitations::define("tonk-invitations");
    }
}
