//! Trusted-page parsing for account tasks requested by an in-space FABB.
//!
//! Portal transport validates the wire and translates coordinates. This
//! module applies the UI capability boundary: the account shell accepts only
//! account-purpose requests and keeps the request identity for stale-result
//! checks.

use tonk_portal::task::{AccountContext, Action, Presentation, Purpose, Request};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::cell::RefCell;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
struct ActivePresenter {
    request_id: String,
    reply: Option<tonk_portal::ContainedTaskReturn>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
thread_local! {
    static ACTIVE: RefCell<Option<ActivePresenter>> = const { RefCell::new(None) };
}

/// A portal request accepted by the top-page account presenter.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountTask {
    /// Identity allocated by the guest FABB.
    pub request_id: String,
    /// Lifecycle operation for the standing account task.
    pub action: Action,
    /// The trusted flow and original space action to resume.
    pub context: Option<AccountContext>,
    /// Translated viewport geometry, required for open and reseat.
    pub presentation: Option<Presentation>,
}

impl TryFrom<Request> for AccountTask {
    type Error = Rejection;

    fn try_from(request: Request) -> Result<Self, Self::Error> {
        request.validate().map_err(|_| Rejection::Invalid)?;
        if request.purpose != Purpose::Account {
            return Err(Rejection::UnsupportedPurpose);
        }
        Ok(Self {
            request_id: request.request_id,
            action: request.action,
            context: request.account,
            presentation: request.presentation,
        })
    }
}

/// Why the trusted page refused a task before opening account UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The portal request failed its version, identity or geometry contract.
    Invalid,
    /// This page does not grant the requested task capability.
    UnsupportedPurpose,
}

/// Apply one validated portal lifecycle message to the trusted account UI.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn handle(request: Request, reply: Option<tonk_portal::ContainedTaskReturn>) {
    let task = match AccountTask::try_from(request) {
        Ok(task) => task,
        Err(_) => {
            if let Some(reply) = reply {
                reply.finish("invalid");
            }
            return;
        }
    };
    match task.action {
        Action::Open => open(task, reply),
        Action::Reseat => {
            if is_active(&task.request_id)
                && let Some(presentation) = task.presentation.as_ref()
            {
                crate::register_dialog::reseat_fabb_task(presentation);
            }
        }
        Action::Suspend if is_active(&task.request_id) => crate::register_dialog::suspend(),
        Action::Show if is_active(&task.request_id) => crate::register_dialog::resume(),
        Action::Dismiss if is_active(&task.request_id) => {
            finish(&task.request_id, "disconnected");
            crate::register_dialog::close();
        }
        _ => {}
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn open(task: AccountTask, reply: Option<tonk_portal::ContainedTaskReturn>) {
    let (Some(presentation), Some(context), Some(reply)) =
        (task.presentation.as_ref(), task.context.as_ref(), reply)
    else {
        return;
    };
    let request_id = task.request_id.clone();
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some(ActivePresenter {
            request_id: request_id.clone(),
            reply: Some(reply),
        });
    });
    let return_id = request_id.clone();
    crate::register_dialog::open_fabb_task(presentation, move || {
        let result = if crate::register_dialog::take_fabb_task_success() {
            "completed"
        } else {
            "cancelled"
        };
        finish(&return_id, result);
    });
    crate::register_dialog::describe(
        &serde_json::json!({ "reason": context.reason, "space": context.space }).to_string(),
    );
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn is_active(request_id: &str) -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|active| active.request_id == request_id)
    })
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn finish(request_id: &str, result: &str) {
    let active = ACTIVE.with(|active| {
        if active
            .borrow()
            .as_ref()
            .is_some_and(|active| active.request_id == request_id)
        {
            active.borrow_mut().take()
        } else {
            None
        }
    });
    if let Some(reply) = active.and_then(|mut active| active.reply.take()) {
        reply.finish(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonk_portal::task::{Anchor, Dismissal, Horizontal, VERSION, Vertical};

    fn request(purpose: Purpose, action: Action) -> Request {
        Request {
            version: VERSION,
            request_id: "account-1".into(),
            purpose,
            action,
            account: (purpose == Purpose::Account).then_some(AccountContext {
                reason: "needs-account".into(),
                space: "did:key:space".into(),
            }),
            presentation: Some(Presentation {
                anchor: Anchor {
                    left: 100.0,
                    top: 200.0,
                    right: 460.0,
                    bottom: 248.0,
                    width: 360.0,
                    height: 48.0,
                },
                horizontal: Horizontal::Right,
                vertical: Vertical::Bottom,
                dismissal: Dismissal::Optional,
            }),
        }
    }

    #[test]
    fn it_accepts_only_account_tasks_and_preserves_the_translated_seat() {
        let accepted =
            AccountTask::try_from(request(Purpose::Account, Action::Open)).expect("account task");
        assert_eq!(accepted.request_id, "account-1");
        let presentation = accepted.presentation.expect("presentation");
        assert_eq!(presentation.horizontal, Horizontal::Right);
        assert_eq!(presentation.vertical, Vertical::Bottom);
        assert_eq!(presentation.anchor.right, 460.0);
        assert_eq!(presentation.anchor.bottom, 248.0);
    }

    #[test]
    fn it_rejects_a_transport_probe_at_the_ui_capability_boundary() {
        assert_eq!(
            AccountTask::try_from(request(Purpose::Probe, Action::Open)),
            Err(Rejection::UnsupportedPurpose)
        );
    }
}
