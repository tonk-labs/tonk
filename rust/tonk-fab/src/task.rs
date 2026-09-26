//! Pure state for one contained FABB task.
//!
//! The DOM presenter lives in `contained_tasks`; this module owns the part
//! that must remain deterministic and native-testable: one active request,
//! monotonically increasing identities, explicit terminal outcomes and stale
//! completion rejection.

/// The identity of one contained request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId(u64);

impl RequestId {
    /// The stable numeric value exposed to the browser-side presenter.
    pub fn get(self) -> u64 {
        self.0
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

/// Whether the reader can dismiss a task without completing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissal {
    Optional,
    Required,
}

/// Every way a contained task can finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed(String),
    Cancelled,
    Acknowledged,
    Disconnected,
}

impl Outcome {
    /// The string contract used by the imperative browser API.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed(value) => value,
            Self::Cancelled => "cancelled",
            Self::Acknowledged => "acknowledged",
            Self::Disconnected => "disconnected",
        }
    }
}

/// A completed task and the snapshot its presenter must restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion<S> {
    pub id: RequestId,
    pub snapshot: S,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Active<S> {
    id: RequestId,
    snapshot: S,
    dismissal: Dismissal,
}

/// Mutual exclusion and stale-result protection for a contained presenter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller<S> {
    next_id: u64,
    active: Option<Active<S>>,
}

impl<S> Default for Controller<S> {
    fn default() -> Self {
        Self {
            next_id: 0,
            active: None,
        }
    }
}

impl<S> Controller<S> {
    /// Begin a request, or return `Busy` without disturbing the current one.
    pub fn begin(&mut self, snapshot: S, dismissal: Dismissal) -> Result<RequestId, Busy> {
        if self.active.is_some() {
            return Err(Busy);
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = RequestId(self.next_id);
        self.active = Some(Active {
            id,
            snapshot,
            dismissal,
        });
        Ok(id)
    }

    pub fn active_id(&self) -> Option<RequestId> {
        self.active.as_ref().map(|active| active.id)
    }

    pub fn dismissal(&self) -> Option<Dismissal> {
        self.active.as_ref().map(|active| active.dismissal)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn active_snapshot(&self) -> Option<&S> {
        self.active.as_ref().map(|active| &active.snapshot)
    }

    /// Finish only the matching request. A late result is ignored.
    pub fn resolve(&mut self, id: RequestId, outcome: Outcome) -> Option<Completion<S>> {
        if self.active.as_ref().map(|active| active.id) != Some(id) {
            return None;
        }
        let active = self.active.take()?;
        Some(Completion {
            id,
            snapshot: active.snapshot,
            outcome,
        })
    }

    /// Finish the current request on presenter teardown.
    pub fn disconnect(&mut self) -> Option<Completion<S>> {
        let id = self.active_id()?;
        self.resolve(id, Outcome::Disconnected)
    }
}

/// A second request cannot replace the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_request_owns_its_snapshot_until_it_resolves() {
        let mut controller = Controller::default();
        let first = controller
            .begin("members", Dismissal::Optional)
            .expect("first request");
        assert_eq!(controller.begin("share", Dismissal::Required), Err(Busy));

        let done = controller
            .resolve(first, Outcome::Completed("continue".into()))
            .expect("matching result");
        assert_eq!(done.snapshot, "members");
        assert_eq!(done.outcome.as_str(), "continue");
        assert_eq!(controller.active_id(), None);
    }

    #[test]
    fn a_stale_completion_cannot_mutate_the_next_request() {
        let mut controller = Controller::default();
        let first = controller
            .begin(1, Dismissal::Optional)
            .expect("first request");
        controller
            .resolve(first, Outcome::Cancelled)
            .expect("finish first");
        let second = controller
            .begin(2, Dismissal::Required)
            .expect("second request");

        assert_eq!(controller.resolve(first, Outcome::Acknowledged), None);
        assert_eq!(controller.active_id(), Some(second));
        assert_eq!(controller.dismissal(), Some(Dismissal::Required));
    }

    #[test]
    fn disconnect_is_terminal_and_restores_the_owned_snapshot() {
        let mut controller = Controller::default();
        controller
            .begin("menu", Dismissal::Optional)
            .expect("request");
        let done = controller.disconnect().expect("disconnect completion");
        assert_eq!(done.snapshot, "menu");
        assert_eq!(done.outcome, Outcome::Disconnected);
        assert_eq!(controller.disconnect(), None);
    }
}
