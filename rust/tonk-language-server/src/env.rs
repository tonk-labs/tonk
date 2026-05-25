//! The environment seam — the port the language server defines
//! and the host implements.
//!
//! The language server resolves diagnostics, completion, and
//! hover against the *live environment* the document belongs to.
//! It must not know how that environment is opened — that is the
//! host's job (a reactor session in the worker, nothing at all in
//! a standalone editor). So the language server defines the port;
//! the host implements it; the dependency points host → language
//! server.
//!
//! Per-request, the language server hands the host an
//! [`EnvProvider`] and asks it to [`open`](EnvProvider::open)
//! the live `(source, env)` pair for the document's branch. The
//! returned [`Opened`] keeps both halves borrowable for the
//! request's lifetime — the LSP threads `opened.source()` into
//! every resolution chain and `opened.env()` into its
//! `.perform(env)`, matching the dialog idiom.

use async_trait::async_trait;
use dialog_capability::{Command, Provider};
use dialog_common::ConditionalSync;
use tonk_schema::concept::QueryEnv;
use tonk_schema::query_source::Source;

/// The port the host implements so the language server can open
/// the live `(source, env)` pair for a document.
///
/// The language server parses a document URI to `(repo, branch)`
/// and calls [`open`](Self::open) per request. Returning `None`
/// is fine: the language server then sees only the document's
/// own declarations.
///
/// `?Send` on wasm because the host's opened value often borrows
/// a non-`Send` reactor session.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait EnvProvider {
    /// What [`open`](Self::open) hands back when the host knows
    /// the branch — a paired source + env value the LSP borrows
    /// from for the request's lifetime.
    type Opened: Opened;

    /// Open the live `(source, env)` pair for `(repo, branch)`,
    /// or `None` when the host knows no such branch.
    async fn open(&self, repo: &str, branch: &str) -> Option<Self::Opened>;
}

/// A live source + env pair, held just long enough for one LSP
/// request. The host returns one from
/// [`EnvProvider::open`]; the LSP threads
/// [`source`](Self::source) into resolution-chain builders and
/// [`env`](Self::env) into their `.perform(env)`.
pub trait Opened {
    /// The execution env the host pairs with the source — what
    /// resolution chains' `.perform(env)` takes.
    type Env: QueryEnv + ConditionalSync;

    /// The branch (or transaction overlay) the host opened, wrapped
    /// as a resolution [`Source`].
    fn source(&self) -> Source<'_>;

    /// The execution env the resolution chains run against.
    fn env(&self) -> &Self::Env;
}

/// An [`EnvProvider`] with no host behind it —
/// [`open`](Self::open) always returns `None`. Tests and a
/// standalone editor pass this so the language server resolves
/// only the document's own declarations.
pub struct NoEnv;

/// The never-constructed [`Opened`] value [`NoEnv`] would return.
///
/// `NoEnv::open` is hardwired to `None`, so no value of this type
/// is ever produced; the [`Opened`] impl exists only to satisfy
/// the associated-type bound. Every method is unreachable —
/// `match *self {}` discharges the uninhabited type.
pub enum NoOpened {}

/// The never-constructed env [`NoOpened`] would carry.
///
/// Uninhabited; the [`QueryEnv`] supertraits are satisfied via a
/// blanket [`Provider`] impl over any [`Command`], plus
/// [`ConditionalSync`] which is itself a blanket trait.
pub enum NoQueryEnv {}

// `Provider` is async-trait-generated; on native it boxes the
// future as `Pin<Box<dyn Future + Send>>`, which requires the
// captured `C::Input` to be `Send`. The bound is unreachable —
// `NoQueryEnv` is uninhabited so `execute` is never called — but
// the trait dispatch still inspects it. On wasm the trait is
// `?Send`, so the bound is unneeded.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl<C: Command> Provider<C> for NoQueryEnv
where
    C::Input: Send + 'static,
{
    async fn execute(&self, _input: C::Input) -> C::Output {
        match *self {}
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl<C: Command> Provider<C> for NoQueryEnv
where
    C::Input: 'static,
{
    async fn execute(&self, _input: C::Input) -> C::Output {
        match *self {}
    }
}

impl Opened for NoOpened {
    type Env = NoQueryEnv;

    fn source(&self) -> Source<'_> {
        match *self {}
    }

    fn env(&self) -> &Self::Env {
        match *self {}
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl EnvProvider for NoEnv {
    type Opened = NoOpened;

    async fn open(&self, _repo: &str, _branch: &str) -> Option<Self::Opened> {
        None
    }
}
