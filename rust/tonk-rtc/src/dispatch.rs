//! Choosing which browser tab carries an operation, and what to do when
//! it does not answer.
//!
//! `RTCPeerConnection` is `[Exposed=Window]`, so a service worker cannot
//! hold a peer connection. The replica and the sync engine live in the
//! worker; the channels live in the pages. Every operation therefore
//! crosses worker → page → CLI and back, and the worker has to decide
//! *which* page.
//!
//! This module is that decision and nothing else: no worker, no
//! `postMessage`, no WebRTC. It is a state machine fed events and
//! producing [`Action`]s, so every failure path — a frozen tab, a tab
//! closed mid-operation, every tab gone — is reachable in a test
//! without a browser.
//!
//! # Why pages are ranked by visibility
//!
//! Browsers freeze and throttle background tabs. A frozen tab cannot
//! service a dispatch, and it does not announce that it has been
//! frozen — it simply stops answering. The visible tab is the one the
//! browser guarantees is running, which makes "most recently visible"
//! a liveness heuristic rather than an arbitrary tiebreak.
//!
//! # Why a whole session is pinned to one page
//!
//! Dialog's push writes blocks in reference order — children before
//! parents — so that every prefix of an interrupted push leaves the
//! remote closure-complete. That ordering is a protocol invariant, not
//! a nicety: another pusher's existence probes prune a whole subtree on
//! one positive answer, which is sound only if a block's presence
//! implies the presence of everything it references.
//!
//! Spreading one push across pages would break it, because two pages
//! write over separate SCTP associations with no ordering between
//! them. So a [`Session`] is pinned to a page for its whole life.
//!
//! The same invariant is what makes failover safe: because every prefix
//! is closure-complete, a session interrupted halfway can simply be
//! restarted on another page. Nothing has to be undone.
//!
//! # Why failing is a normal outcome
//!
//! When every page is frozen — the person switched to another
//! application — there is no live carrier, and the answer is not to
//! wait. [`Action::Abandon`] tells the caller to fall back to the
//! ordinary remote. WebRTC here is an accelerator, never the only path.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A page holding a channel to the CLI, as the worker knows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PageId(pub String);

/// A run of operations that must travel over one page, in order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// One operation awaiting an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(pub u64);

/// Milliseconds on a monotonic clock the caller owns.
pub type Instant = u64;

/// What the caller should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Hand this request to this page.
    Send {
        /// The page to post to.
        page: PageId,
        /// The request being carried.
        request: RequestId,
    },
    /// The answer arrived; the request is finished.
    Complete {
        /// The request that finished.
        request: RequestId,
    },
    /// No page could carry this. Fall back to the ordinary remote.
    Abandon {
        /// The request that could not be carried.
        request: RequestId,
        /// Why, for a log the operator will actually read.
        reason: Abandoned,
    },
}

/// Why a request could not be carried over any page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abandoned {
    /// There were no pages with a channel at all.
    NoPage,
    /// Every page was tried and none answered.
    Exhausted,
}

/// How long the caller is willing to wait, at each of two stages.
///
/// Two deadlines rather than one, because they detect different
/// failures. A page that has been frozen never acknowledges at all, and
/// should be abandoned quickly. A page that acknowledged is alive and
/// the CLI is merely working, which may legitimately take much longer.
/// One deadline would have to be either so short it sheds healthy slow
/// work, or so long a frozen tab stalls sync behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines {
    /// From posting to the page until it confirms it sent.
    pub ack: u64,
    /// From that confirmation until the CLI's answer comes back.
    pub response: u64,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            ack: 1_000,
            response: 30_000,
        }
    }
}

#[derive(Debug, Clone)]
struct Page {
    /// When this page was last visible. Pages never yet visible sort
    /// last, which is what `0` achieves.
    last_visible: Instant,
    /// Whether it currently holds a channel to the CLI. A page without
    /// one is not a candidate.
    connected: bool,
}

#[derive(Debug, Clone)]
struct Inflight {
    session: Option<SessionId>,
    /// Pages already tried and found wanting, so failover does not
    /// circle back to them.
    exhausted: Vec<PageId>,
    carrier: PageId,
    /// `None` until the carrier confirms it sent.
    acked: bool,
    deadline: Instant,
}

/// Routes operations to pages and fails them over.
#[derive(Debug, Default)]
pub struct Dispatch {
    pages: HashMap<PageId, Page>,
    inflight: HashMap<RequestId, Inflight>,
    /// Which page each session is pinned to, for as long as it holds.
    pinned: HashMap<SessionId, PageId>,
    deadlines: Deadlines,
}

impl Dispatch {
    /// A dispatcher with the given deadlines.
    pub fn new(deadlines: Deadlines) -> Self {
        Self {
            deadlines,
            ..Default::default()
        }
    }

    /// A page connected, or reported itself still present.
    pub fn connected(&mut self, page: PageId, at: Instant) {
        let entry = self.pages.entry(page).or_insert(Page {
            last_visible: 0,
            connected: false,
        });
        entry.connected = true;
        entry.last_visible = entry.last_visible.max(at);
    }

    /// A page became visible. Visibility is the liveness signal, so
    /// this is what reorders the candidates.
    pub fn visible(&mut self, page: PageId, at: Instant) {
        if let Some(entry) = self.pages.get_mut(&page) {
            entry.last_visible = at;
        }
    }

    /// A page went away — closed, or its channel dropped.
    ///
    /// Any request it was carrying is re-dispatched, and any session
    /// pinned to it is unpinned so it can be restarted elsewhere.
    pub fn gone(&mut self, page: &PageId, now: Instant) -> Vec<Action> {
        self.pages.remove(page);
        self.pinned.retain(|_, pinned| pinned != page);

        let orphaned: Vec<RequestId> = self
            .inflight
            .iter()
            .filter(|(_, flight)| &flight.carrier == page)
            .map(|(request, _)| *request)
            .collect();

        orphaned
            .into_iter()
            .map(|request| self.redirect(request, now))
            .collect()
    }

    /// Carry a request, optionally as part of an ordered session.
    ///
    /// A session already pinned to a live page goes to that page; the
    /// ordering invariant on a push means its operations must not be
    /// spread across pages.
    pub fn dispatch(
        &mut self,
        request: RequestId,
        session: Option<SessionId>,
        now: Instant,
    ) -> Action {
        let pinned = session
            .as_ref()
            .and_then(|session| self.pinned.get(session))
            .filter(|page| {
                self.pages
                    .get(*page)
                    .is_some_and(|carrier| carrier.connected)
            })
            .cloned();

        let Some(carrier) = pinned.or_else(|| self.best(&[])) else {
            return Action::Abandon {
                request,
                reason: Abandoned::NoPage,
            };
        };

        if let Some(session) = session.clone() {
            self.pinned.insert(session, carrier.clone());
        }
        self.inflight.insert(
            request,
            Inflight {
                session,
                exhausted: Vec::new(),
                carrier: carrier.clone(),
                acked: false,
                deadline: now + self.deadlines.ack,
            },
        );
        Action::Send {
            page: carrier,
            request,
        }
    }

    /// The carrier confirmed it put the request on the wire.
    ///
    /// This is what distinguishes a frozen page from a busy CLI, and it
    /// swaps the short deadline for the long one.
    pub fn acked(&mut self, request: RequestId, now: Instant) {
        if let Some(flight) = self.inflight.get_mut(&request) {
            flight.acked = true;
            flight.deadline = now + self.deadlines.response;
        }
    }

    /// The CLI's answer arrived, by way of the carrier.
    pub fn answered(&mut self, request: RequestId) -> Option<Action> {
        self.inflight
            .remove(&request)
            .map(|_| Action::Complete { request })
    }

    /// Advance the clock. Anything past its deadline fails over.
    ///
    /// Actions come back in request order so a caller's log — and a
    /// test's assertions — do not depend on map iteration order.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut overdue: Vec<RequestId> = self
            .inflight
            .iter()
            .filter(|(_, flight)| now >= flight.deadline)
            .map(|(request, _)| *request)
            .collect();
        overdue.sort_unstable();

        overdue
            .into_iter()
            .map(|request| self.redirect(request, now))
            .collect()
    }

    /// Move a request to the next candidate, or give up on it.
    fn redirect(&mut self, request: RequestId, now: Instant) -> Action {
        let Some(mut flight) = self.inflight.remove(&request) else {
            return Action::Abandon {
                request,
                reason: Abandoned::Exhausted,
            };
        };

        // A page that never acknowledged is not slow, it is not running
        // — a frozen tab stops answering without saying so. Demote it,
        // or it stays the most-recently-visible candidate and every
        // subsequent request pays the ack deadline again before
        // reaching the same conclusion. Ten queued operations would
        // spend ten ack deadlines discovering the same dead tab.
        //
        // A page that DID acknowledge is demonstrably alive and the CLI
        // is merely working, so it keeps its standing: the timeout says
        // nothing about the page.
        if !flight.acked
            && let Some(carrier) = self.pages.get_mut(&flight.carrier)
        {
            carrier.connected = false;
        }

        flight.exhausted.push(flight.carrier.clone());
        // A session whose carrier failed is no longer pinned: the whole
        // session restarts elsewhere, which is sound because every
        // prefix of an interrupted push is closure-complete.
        if let Some(session) = &flight.session {
            self.pinned.remove(session);
        }

        let Some(carrier) = self.best(&flight.exhausted) else {
            return Action::Abandon {
                request,
                reason: Abandoned::Exhausted,
            };
        };

        if let Some(session) = flight.session.clone() {
            self.pinned.insert(session, carrier.clone());
        }
        flight.carrier = carrier.clone();
        flight.acked = false;
        flight.deadline = now + self.deadlines.ack;
        self.inflight.insert(request, flight);

        Action::Send {
            page: carrier,
            request,
        }
    }

    /// The most recently visible connected page, skipping any already
    /// tried. Ties break on id so the choice is deterministic.
    fn best(&self, skip: &[PageId]) -> Option<PageId> {
        self.pages
            .iter()
            .filter(|(id, page)| page.connected && !skip.contains(id))
            .max_by(|(left_id, left), (right_id, right)| {
                left.last_visible
                    .cmp(&right.last_visible)
                    .then_with(|| right_id.cmp(left_id))
            })
            .map(|(id, _)| id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str) -> PageId {
        PageId(name.to_owned())
    }

    fn dispatcher() -> Dispatch {
        Dispatch::new(Deadlines {
            ack: 1_000,
            response: 30_000,
        })
    }

    #[test]
    fn the_most_recently_visible_page_carries_the_request() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 20);
        dispatch.visible(page("a"), 30);

        assert_eq!(
            dispatch.dispatch(RequestId(1), None, 100),
            Action::Send {
                page: page("a"),
                request: RequestId(1)
            }
        );
    }

    /// A page with no channel is not a candidate however recently it
    /// was looked at.
    #[test]
    fn a_page_without_a_channel_is_never_chosen() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.visible(page("b"), 99); // unknown page, no channel
        assert_eq!(
            dispatch.dispatch(RequestId(1), None, 100),
            Action::Send {
                page: page("a"),
                request: RequestId(1)
            }
        );
    }

    #[test]
    fn with_no_pages_at_all_the_caller_is_told_to_fall_back() {
        let mut dispatch = dispatcher();
        assert_eq!(
            dispatch.dispatch(RequestId(1), None, 0),
            Action::Abandon {
                request: RequestId(1),
                reason: Abandoned::NoPage
            }
        );
    }

    /// A frozen tab does not announce itself; it just stops answering.
    /// The short deadline is what turns that silence into a failover.
    #[test]
    fn a_page_that_never_acknowledges_is_abandoned_for_the_next_one() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        assert_eq!(
            dispatch.dispatch(RequestId(1), None, 0),
            Action::Send {
                page: page("a"),
                request: RequestId(1)
            }
        );
        assert_eq!(dispatch.tick(500), vec![]);
        assert_eq!(
            dispatch.tick(1_000),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );
    }

    /// Acknowledging buys the long deadline: the page is demonstrably
    /// alive and the CLI is merely working.
    #[test]
    fn acknowledging_buys_the_longer_deadline() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);

        dispatch.dispatch(RequestId(1), None, 0);
        dispatch.acked(RequestId(1), 100);

        // Well past the ack deadline, nowhere near the response one.
        assert_eq!(dispatch.tick(5_000), vec![]);
        assert_eq!(
            dispatch.tick(30_100),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );
    }

    /// The answer rides the same channel that carried the request, so a
    /// page dying after sending loses it. Re-issuing the whole request
    /// is the only recovery, and it is safe because the effects are
    /// idempotent.
    #[test]
    fn a_page_closing_mid_flight_moves_its_request() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        dispatch.dispatch(RequestId(1), None, 0);
        dispatch.acked(RequestId(1), 10);

        assert_eq!(
            dispatch.gone(&page("a"), 20),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );
    }

    #[test]
    fn once_every_page_has_been_tried_the_request_is_abandoned() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);

        dispatch.dispatch(RequestId(1), None, 0);
        assert!(matches!(
            dispatch.tick(1_000).as_slice(),
            [Action::Send { .. }]
        ));
        assert_eq!(
            dispatch.tick(2_000),
            vec![Action::Abandon {
                request: RequestId(1),
                reason: Abandoned::Exhausted
            }]
        );
    }

    /// Dialog's push writes children before parents so that every
    /// prefix is closure-complete. Two pages write over separate SCTP
    /// associations with no ordering between them, so one session must
    /// not be spread across pages — even when a better candidate
    /// appears mid-session.
    #[test]
    fn a_session_stays_on_one_page_even_when_a_better_one_appears() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.visible(page("a"), 30);

        let session = Some(SessionId("push-1".into()));
        assert_eq!(
            dispatch.dispatch(RequestId(1), session.clone(), 0),
            Action::Send {
                page: page("a"),
                request: RequestId(1)
            }
        );

        // A second page connects and is looked at. It is now the best
        // candidate by every rule — and must still not be used.
        dispatch.connected(page("b"), 40);
        dispatch.visible(page("b"), 50);

        assert_eq!(
            dispatch.dispatch(RequestId(2), session, 60),
            Action::Send {
                page: page("a"),
                request: RequestId(2)
            }
        );
    }

    /// When the pinned page does fail, the whole session moves — which
    /// is sound precisely because an interrupted push left the remote
    /// closure-complete.
    #[test]
    fn a_session_moves_wholesale_when_its_page_fails() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        let session = Some(SessionId("push-1".into()));
        dispatch.dispatch(RequestId(1), session.clone(), 0);
        assert_eq!(
            dispatch.gone(&page("a"), 10),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );

        // The next operation in the session follows it.
        assert_eq!(
            dispatch.dispatch(RequestId(2), session, 20),
            Action::Send {
                page: page("b"),
                request: RequestId(2)
            }
        );
    }

    #[test]
    fn an_answered_request_stops_being_tracked() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.dispatch(RequestId(1), None, 0);

        assert_eq!(
            dispatch.answered(RequestId(1)),
            Some(Action::Complete {
                request: RequestId(1)
            })
        );
        assert_eq!(dispatch.tick(1_000_000), vec![]);
        assert_eq!(dispatch.answered(RequestId(1)), None);
    }

    /// A frozen tab stays the most-recently-visible page, so without
    /// demoting it every queued request pays the ack deadline again
    /// before reaching the same conclusion.
    #[test]
    fn a_page_that_froze_stops_being_chosen_for_later_requests() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        dispatch.dispatch(RequestId(1), None, 0);
        assert_eq!(
            dispatch.tick(1_000),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );

        // The next request must not rediscover the same frozen page.
        assert_eq!(
            dispatch.dispatch(RequestId(2), None, 1_100),
            Action::Send {
                page: page("b"),
                request: RequestId(2)
            }
        );
    }

    /// A slow CLI is not a dead page. A carrier that acknowledged keeps
    /// its standing, so a long-running operation does not cost the page
    /// its place for everything else.
    #[test]
    fn a_slow_response_does_not_demote_the_page_that_carried_it() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        dispatch.dispatch(RequestId(1), None, 0);
        dispatch.acked(RequestId(1), 10);
        assert_eq!(
            dispatch.tick(30_100),
            vec![Action::Send {
                page: page("b"),
                request: RequestId(1)
            }]
        );

        // "a" answered once, so it is alive and still the best pick.
        assert_eq!(
            dispatch.dispatch(RequestId(2), None, 30_200),
            Action::Send {
                page: page("a"),
                request: RequestId(2)
            }
        );
    }

    /// A page demoted for freezing must not keep a session pinned to it.
    #[test]
    fn a_demoted_page_does_not_hold_a_session_hostage() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.visible(page("a"), 30);

        let session = Some(SessionId("push-1".into()));
        dispatch.dispatch(RequestId(1), session.clone(), 0);
        dispatch.tick(1_000);

        assert_eq!(
            dispatch.dispatch(RequestId(2), session, 1_100),
            Action::Send {
                page: page("b"),
                request: RequestId(2)
            }
        );
    }

    /// Two requests timing out in the same tick must come back in a
    /// stable order, or a caller's behaviour depends on map iteration.
    #[test]
    fn simultaneous_timeouts_are_reported_in_request_order() {
        let mut dispatch = dispatcher();
        dispatch.connected(page("a"), 10);
        dispatch.connected(page("b"), 5);
        dispatch.dispatch(RequestId(2), None, 0);
        dispatch.dispatch(RequestId(1), None, 0);

        let actions = dispatch.tick(1_000);
        assert_eq!(
            actions,
            vec![
                Action::Send {
                    page: page("b"),
                    request: RequestId(1)
                },
                Action::Send {
                    page: page("b"),
                    request: RequestId(2)
                },
            ]
        );
    }
}
