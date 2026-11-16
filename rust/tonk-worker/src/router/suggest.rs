//! `intent/suggest`: what a line typed in the command palette could mean,
//! answered by a provider on the branch's stack.
//!
//! The page asserts the command through the transact path, on each
//! branch it asks (the space's and the profile's), once that branch has
//! recorded the line's `intent/interpret`: an effect runs its commands'
//! providers in the order they landed, which is no order the page sets,
//! so an ask sent in the interpretation's own commit could be answered
//! before the line was recorded. Once the commit has published, the
//! command's effect performs the provider registered here: it decodes the ask from the command's
//! rows, reads the branch's palette vocabulary and what rules derived
//! onto the line's intents (through [`dialog_reactor::intent::suggest`]),
//! and answers with the rows [`dialog_reactor::intent::answer`] makes,
//! placed in the stack's own `session:` layer: one `intent/reading` per
//! reading and one `intent/answer`, every row naming the ask, so the tab
//! that asked reads its own answer and no other's through the query it
//! already has. An answer replaces the asker's previous one. Nothing is
//! stored, and it goes with the process.
//!
//! The provider is one registration of the worker's vocabulary
//! ([`command_providers`](crate::router::command_providers)): a handler
//! over the [`CommandEnv`] of the command's tick, registered on each
//! branch's stack with the rest, so it reaches the operator and the
//! reactor the way every other command handler does, and compiles for
//! every target.

use dialog_corpus::Changes;
use dialog_query::{ConceptDescriptor, Descriptor};
use dialog_reactor::intent::{Ask, answer, suggest};
use dialog_repository::{Command, Failure, Layer, session_layer};

use crate::router::CommandEnv;

/// The concept the provider is registered for: `intent/suggest`.
pub(crate) fn concept() -> &'static ConceptDescriptor {
    <tonk_schema::command::Suggest as Descriptor<ConceptDescriptor>>::descriptor()
}

/// Answer one landed `intent/suggest`.
pub(crate) async fn respond(env: CommandEnv, command: Command) -> Result<Changes, Failure> {
    let ask = Ask::from_rows(command.entity().clone(), command.rows());
    // A clone of the state, not its guard: parsing takes a while, and a
    // guard held across it would keep every writer waiting.
    let tonk = env.state().read().await.clone();
    let origin = env.origin();
    let branch = if origin.repo.is_empty() {
        tonk.reactor.profile_repository().branch(&origin.branch)
    } else {
        tonk.reactor.repository(&origin.repo).branch(&origin.branch)
    };
    let session = branch
        .acquire(&tonk.operator)
        .await
        .map_err(|error| Failure::new(format!("branch unavailable: {error}")))?;
    let readings = suggest(session.handle(), &tonk.operator, &ask).await?;
    // The asker's previous answer, to replace: the rows the stack's own
    // layer holds now.
    let held = match session.state.stack().layer(&session_layer()) {
        Some(Layer::Ephemeral(memory)) => memory.export(),
        _ => Changes::new(),
    };
    Ok(answer(&ask, &readings, &held))
}
