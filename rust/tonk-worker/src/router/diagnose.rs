//! `diagnose:expand` — the search-tree inspector's one command.
//!
//! The inspector is otherwise entirely derived: its concepts are rules over
//! dialog's `tree/*` resolvers (see
//! `tonk-core/assets/library/diagnose.yaml`), so the worker answers them
//! through the ordinary query path and has nothing to publish. This handler
//! exists for the two things a resolver cannot say.
//!
//! **What to show.** A resolver is not enumerable — it answers about a node
//! you name — so an outline of a whole tree is either a full walk or a
//! choice about where to look. Expansion is that choice, recorded as a fact
//! so the children rule can read it as an ordinary premise.
//!
//! **Where the bytes are.** Whether a block is held here or would have to be
//! fetched is not a property of the (immutable) block, so
//! `dialog_artifacts::inspect` refuses to serve it through `Load`. This
//! probes the local archive directly and publishes the answer as a dated
//! snapshot — the same shape the console publishes reactor state in.
//!
//! Both are overlay facts: folded into every read of the branch, never
//! committed, never replicated. Browsing a tree is not a change to it.

use dialog_artifacts::Entity;
use dialog_artifacts::inspect::inspect_spans;
use dialog_repository::{LocalIndex, RepositoryArchiveExt};
use dialog_storage::{Blake3Hash, StorageBackend};
use tonk_common::log;
use tonk_schema::{DiagnoseOutline, DiagnoseStatus};

/// Run `diagnose:expand`: record the expansion, then probe the locality of
/// everything it reveals.
///
/// Probing at expansion time is what keeps the snapshot honest without a
/// sweep: the only moment a node's status starts mattering is when the
/// outline puts it on screen, and that is exactly this moment.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::DiagnoseExpand>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::DiagnoseExpand) {
        let node = command.node.0;
        let Some(hash) = node.node_hash() else {
            log!("diagnose:expand: {node} is not a `tree:` node entity; skipping");
            return;
        };

        let origin = self.origin();
        let tonk = self.state().read().await;
        let session = match tonk
            .reactor
            .repository(&origin.repo)
            .branch(&origin.branch)
            .acquire(&tonk.operator)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                log!(
                    "diagnose:expand: failed to acquire {}/{}: {error}",
                    origin.repo,
                    origin.branch
                );
                return;
            }
        };

        // The expansion itself. Cardinality-many, so this accumulates and
        // the children rule sees every node the viewer has opened.
        session
            .state
            .assert_overlay(DiagnoseOutline::expanding(node.clone()));

        // The locality of what the expansion reveals — the node itself and
        // each child it delegates to. A child is probed WITHOUT fetching
        // it: that is the whole question the marker answers.
        let index = LocalIndex::new(&tonk.operator, session.state.branch.archive().index());
        let probed = stamp();
        let mut statuses = vec![(node, probe(&index, &hash).await)];
        if let Some(bytes) = read_local(&index, &hash).await {
            for child in children(bytes) {
                match Entity::from_node(&child) {
                    Ok(entity) => statuses.push((entity, probe(&index, &child).await)),
                    Err(error) => log!("diagnose:expand: child entity: {error}"),
                }
            }
        }
        for (entity, local) in statuses {
            session
                .state
                .assert_overlay(DiagnoseStatus::new(entity, local, probed.clone()));
        }

        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&session.state));
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    }
}

/// Whether the block behind `hash` is in this device's archive.
///
/// A LOCAL read, deliberately: the networked read every other path uses
/// would fetch the block and make the answer trivially true.
async fn probe<Env>(index: &LocalIndex<'_, Env>, hash: &Blake3Hash) -> bool
where
    Env: dialog_capability::Provider<dialog_effects::archive::Get>
        + dialog_capability::Provider<dialog_effects::archive::Put>
        + dialog_common::ConditionalSync
        + 'static,
{
    read_local(index, hash).await.is_some()
}

/// The block behind `hash`, if it is held here.
async fn read_local<Env>(index: &LocalIndex<'_, Env>, hash: &Blake3Hash) -> Option<Vec<u8>>
where
    Env: dialog_capability::Provider<dialog_effects::archive::Get>
        + dialog_capability::Provider<dialog_effects::archive::Put>
        + dialog_common::ConditionalSync
        + 'static,
{
    index.get(hash).await.ok().flatten()
}

/// The children an index node delegates to. A segment (or a block that
/// does not decode as a node) has none.
fn children(bytes: Vec<u8>) -> Vec<Blake3Hash> {
    inspect_spans(bytes)
        .map(|spans| spans.into_iter().map(|span| span.node).collect())
        .unwrap_or_default()
}

/// The probe's timestamp: seconds since the Unix epoch, as text. A status is
/// only as true as its stamp, so it always carries one.
///
/// `web_time` rather than `std::time`, because this runs in the service
/// worker as well as natively.
fn stamp() -> String {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
