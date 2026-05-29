//! A [`QueryEnv`] that is never executed.
//!
//! Local analysis (`analyze_local`, used by the compile-time
//! `claim!` macro) resolves a document against its own in-document
//! definitions only — no branch, no running system. The analyzer's
//! resolution plumbing is generic over a [`QueryEnv`] and takes it
//! as `Option<&Env>`; the local path passes `None`, so the env is
//! never consulted. But Rust still needs a concrete `Env` type to
//! name for the `None::<&Env>` value.
//!
//! [`NeverEnv`] is that type. It satisfies the [`QueryEnv`] bound
//! with provider methods that `unreachable!()`, since the local
//! path never calls them. It is never constructed and never
//! executed — it exists only to give the turbofish a name.
//!
//! This is a deliberate stopgap for landing the env-free local
//! path; the planned dependency-graph compiler resolves the same
//! need structurally (a sync compile with a separate optional
//! async resolve) and will retire this type.

use async_trait::async_trait;
use dialog_capability::{Command, Fork, Provider};
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_repository::RemoteSite;

/// Uninhabited-in-practice env: names the `Env` type for the
/// `None` resolution path without ever running.
pub(crate) struct NeverEnv;

macro_rules! never_provider {
    ($command:ty) => {
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        impl Provider<$command> for NeverEnv {
            async fn execute(
                &self,
                _input: <$command as Command>::Input,
            ) -> <$command as Command>::Output {
                unreachable!(
                    "NeverEnv is only used for env-free local analysis and is never executed"
                )
            }
        }
    };
}

never_provider!(Get);
never_provider!(Put);
never_provider!(Resolve);
never_provider!(Identify);
never_provider!(Fork<RemoteSite, Get>);
never_provider!(Fork<RemoteSite, Resolve>);
