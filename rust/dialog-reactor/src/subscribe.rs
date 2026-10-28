//! [`Subscribe`] — open or attach to a subscription.

use dialog_query::ConceptQuery;

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;
use super::subscription::Subscriber;

/// Open or attach to a subscription, run an initial poll, and
/// return the [`Subscriber`] handle. The receiver's first message
/// is the current snapshot (delivered by the initial poll);
/// subsequent messages are change broadcasts.
pub struct Subscribe<'a> {
    /// The branch the subscription is scoped to.
    pub branch: BranchReference<'a>,
    /// The query the subscription re-evaluates on every change.
    pub query: ConceptQuery,
    /// Concept bookmark whose persisted typed edges shape this subscription.
    pub graph: Option<String>,
    /// Explicit root matching and edge shaping constraints.
    pub graph_constraints: tonk_schema::graph::GraphConstraints,
    /// The client this subscriber serves, when known — see
    /// [`BranchState::retain_subscribers`](crate::BranchState::retain_subscribers).
    pub client: Option<String>,
    /// Consumer nesting level, notified outermost first — see
    /// [`BranchState::poll`](crate::BranchState::poll).
    pub level: u32,
}

impl<'a> Subscribe<'a> {
    /// Build a new `Subscribe` effect.
    pub fn new(branch: BranchReference<'a>, query: ConceptQuery) -> Self {
        Self {
            branch,
            query,
            graph: None,
            graph_constraints: Default::default(),
            client: None,
            level: 0,
        }
    }

    /// Shape this subscription using the named concept's typed edges.
    pub fn graph(mut self, name: impl Into<String>) -> Self {
        self.graph = Some(name.into());
        self
    }

    /// Narrow a graph subscription at its matching and shaping stages.
    pub fn graph_constraints(mut self, constraints: tonk_schema::graph::GraphConstraints) -> Self {
        self.graph_constraints = constraints;
        self
    }

    /// Tag the subscriber with the client it serves, so the owner
    /// can prune it once that client is no longer alive.
    pub fn client(mut self, client: impl Into<String>) -> Self {
        self.client = Some(client.into());
        self
    }

    /// Set how deeply the consumer is nested, so its branch notifies
    /// outer consumers of a change before inner ones.
    pub fn level(mut self, level: u32) -> Self {
        self.level = level;
        self
    }

    /// Resolve the branch (opening it if needed), attach a
    /// fresh subscriber to the subscription, and poll so the
    /// new subscriber's first message is the current snapshot.
    pub async fn perform<Env>(self, env: &Env) -> Result<Subscriber, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + SelectProvider,
    {
        let session = self.branch.acquire(env).await?;
        let subscriber = if let Some(name) = self.graph {
            let mut graph =
                tonk_schema::graph::resolve_graph(self.query, &name, &session.handle().into(), env)
                    .await
                    .map_err(|error| ReactorError::Graph(error.to_string()))?
                    .ok_or_else(|| {
                        ReactorError::Graph(format!("concept {name:?} has no typed edges"))
                    })?;
            self.graph_constraints
                .apply(&mut graph)
                .map_err(|error| ReactorError::Graph(error.to_string()))?;
            self.branch
                .reactor()
                .subscription_lifecycle
                .register(|| {
                    session
                        .state
                        .subscribe_graph(graph, self.client, self.level)
                })
                .ok_or(ReactorError::Shutdown)??
        } else {
            self.branch.reactor().register_subscription(
                &session,
                self.query,
                self.client,
                self.level,
            )?
        };
        session
            .subscription(subscriber.hash.clone())
            .poll()
            .perform(env)
            .await;
        Ok(subscriber)
    }
}
