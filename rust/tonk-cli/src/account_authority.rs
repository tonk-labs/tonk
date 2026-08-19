//! Account-bound authorization and remote-dispatch boundary.

use anyhow::{Context, Result};
use dialog_capability::access::{Access, Authorize, AuthorizeError, Prove, Retain, TimeRange};
use dialog_capability::{
    Capability, Command, Effect, Fork, ForkInvocation, Policy as _, Provider, Site, SiteFork,
    Subject,
};
use dialog_common::{ConditionalSend, ConditionalSync};
use dialog_effects::authority::{Attest, Identify};
use dialog_operator::{Operator, Profile};
use dialog_repository::RemoteSite as Network;
use dialog_storage::provider::storage::NativeSpace;
use dialog_ucan::{Ucan, UcanAuthorization};
use dialog_ucan_core::DelegationChain;

use crate::account_session::{AccountSessionReadGuard, ActiveAccount};
use crate::spot::SpotStore;

const REMOTE_AUTHORIZATION_MARGIN_SECONDS: u64 = 60;

/// Operator wrapper that forwards local effects but exclusively owns UCAN
/// authorization and every remote network fork.
pub struct AccountBoundOperator {
    inner: Operator<NativeSpace>,
    profile: Profile,
    store: SpotStore,
    require_account: bool,
}

impl AccountBoundOperator {
    /// The operator underneath the account guard.
    ///
    /// Callers that need the concrete operator — account-state resolution
    /// opens the account repository in the same storage — reach it here
    /// rather than rebuilding one against the global install store, which
    /// would mount a different profile.
    pub fn inner(&self) -> &Operator<NativeSpace> {
        &self.inner
    }

    /// Wrap a raw local operator after canonical session initialization.
    pub fn new(
        inner: Operator<NativeSpace>,
        profile: Profile,
        store: SpotStore,
        require_account: bool,
    ) -> Self {
        Self {
            inner,
            profile,
            store,
            require_account,
        }
    }

    /// Persistent profile DID.
    pub fn profile_did(&self) -> dialog_varsig::Did {
        self.inner.profile_did()
    }

    /// Derived operator DID.
    pub fn did(&self) -> dialog_varsig::Did {
        self.inner.did()
    }

    /// Borrow the raw operator for credential/session initialization only.
    pub(crate) fn local(&self) -> &Operator<NativeSpace> {
        &self.inner
    }

    #[cfg(feature = "integration-tests")]
    pub(crate) fn store(&self) -> &SpotStore {
        &self.store
    }

    async fn active(
        &self,
        guard: &AccountSessionReadGuard,
    ) -> Result<ActiveAccount, AuthorizeError> {
        crate::account_session::active_guarded(&self.profile, &self.inner, guard)
            .await
            .map_err(|error| AuthorizeError::Unavailable {
                detail: error.to_string(),
            })?
            .ok_or_else(|| AuthorizeError::Malformed {
                detail: "log in with `tonk account link` before accessing a remote".to_string(),
            })
    }

    async fn authorize_guarded(
        &self,
        input: Capability<Authorize<Ucan>>,
        guard: &AccountSessionReadGuard,
    ) -> Result<UcanAuthorization, AuthorizeError> {
        let input = require_current_window(input, current_unix_seconds());
        if !self.require_account {
            return input.perform(&self.inner).await;
        }
        // Reuse only the fresh signer/scope/duration and current
        // profile→operator session suffix. Historical account authority is
        // deliberately discarded.
        let mut authorization = input.perform(&self.inner).await?;
        let historical = authorization
            .chain
            .take()
            .ok_or_else(|| AuthorizeError::Unavailable {
                detail: "operator authorization has no session chain".to_string(),
            })?;
        // The proven chain's first issuer is the repository authority being
        // exercised. The Authorize capability's own subject is the profile
        // context, not necessarily the repository subject.
        let subject = historical.issuer().clone();
        let session =
            historical
                .proofs()
                .last()
                .cloned()
                .ok_or_else(|| AuthorizeError::Unavailable {
                    detail: "operator authorization has no session proof".to_string(),
                })?;
        if session.issuer() != &self.profile.did() || session.audience() != &self.inner.did() {
            return Err(AuthorizeError::Malformed {
                detail: "operator session proof does not match this profile".to_string(),
            });
        }

        let active = self.active(guard).await?;
        let root: dialog_varsig::Did =
            active
                .root_did
                .parse()
                .map_err(|_| AuthorizeError::Malformed {
                    detail: "active account root DID is invalid".to_string(),
                })?;
        let grant_bytes =
            hex::decode(&active.delegation_hex).map_err(|_| AuthorizeError::Malformed {
                detail: "active account grant hex is invalid".to_string(),
            })?;
        let grant = DelegationChain::try_from(grant_bytes.as_slice()).map_err(|error| {
            AuthorizeError::Malformed {
                detail: format!("active account grant is invalid: {error}"),
            }
        })?;
        if grant.proof_cids().len() != 1
            || grant.proof_cids()[0].to_string() != active.delegation_cid
            || grant.issuer() != &root
            || grant.audience() != &self.profile.did()
        {
            return Err(AuthorizeError::Malformed {
                detail: "active account grant does not match canonical session state".to_string(),
            });
        }
        grant
            .proofs()
            .next()
            .unwrap()
            .verify_signature(&dialog_credentials::DidKeyResolver)
            .await
            .map_err(|_| AuthorizeError::InvalidSignature {
                issuer: root.clone(),
            })?;

        let mut chain = if subject == root {
            grant
        } else {
            // Resolving rather than reading: a spot predating this account,
            // or created by a release that stored no prefix, still has to
            // reach its own remote. Recovery is the same one every other
            // account path uses, so the authority a spot syncs with is the
            // authority it gets backed up with.
            let prefix =
                crate::site::account_root_prefix_for(&self.profile, &self.inner, &subject, &root)
                    .await
                    .map_err(|_| AuthorizeError::UnprovenSubject {
                        claimed: root.clone(),
                        authorized: subject.clone(),
                    })?;
            let active_proof = grant.proofs().next().unwrap().clone();
            prefix
                .push(active_proof)
                .map_err(|error| AuthorizeError::Malformed {
                    detail: format!("account authority chain is invalid: {error}"),
                })?
        };
        chain = chain
            .push(session)
            .map_err(|error| AuthorizeError::Malformed {
                detail: format!("operator authority chain is invalid: {error}"),
            })?;
        authorization.chain = Some(chain);
        Ok(authorization)
    }
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn require_current_window(
    input: Capability<Authorize<Ucan>>,
    now: u64,
) -> Capability<Authorize<Ucan>> {
    let subject = input.subject().clone();
    let request = Authorize::<Ucan>::of(&input);
    let margin = now.saturating_add(REMOTE_AUTHORIZATION_MARGIN_SECONDS);
    let duration = TimeRange {
        not_before: Some(
            request
                .duration
                .not_before
                .map_or(now, |bound| bound.min(now)),
        ),
        expiration: Some(
            request
                .duration
                .expiration
                .map_or(margin, |bound| bound.max(margin)),
        ),
    };
    Subject::from(subject).attenuate(Access).invoke(
        Authorize::<Ucan>::new(request.principal.clone(), request.access.clone()).during(duration),
    )
}

impl dialog_varsig::Principal for AccountBoundOperator {
    fn did(&self) -> dialog_varsig::Did {
        self.inner.did()
    }
}

macro_rules! forward {
    ($command:ty) => {
        #[async_trait::async_trait]
        impl Provider<$command> for AccountBoundOperator
        where
            Operator<NativeSpace>: Provider<$command> + ConditionalSync,
            <$command as Command>::Input: ConditionalSend,
            <$command as Command>::Output: ConditionalSend,
        {
            async fn execute(
                &self,
                input: <$command as Command>::Input,
            ) -> <$command as Command>::Output {
                <Operator<NativeSpace> as Provider<$command>>::execute(&self.inner, input).await
            }
        }
    };
}

forward!(Identify);
forward!(Attest);
forward!(dialog_effects::archive::Get);
forward!(dialog_effects::archive::Put);
forward!(dialog_effects::archive::Import);
forward!(dialog_effects::blob::Read);
forward!(dialog_effects::blob::Write);
forward!(dialog_effects::blob::Import);
forward!(dialog_effects::credential::Load<dialog_credentials::Credential>);
forward!(dialog_effects::credential::Save<dialog_credentials::Credential>);
forward!(dialog_effects::credential::Load<dialog_effects::credential::Secret>);
forward!(dialog_effects::credential::Save<dialog_effects::credential::Secret>);
forward!(dialog_effects::memory::Resolve);
forward!(dialog_effects::memory::Publish);
forward!(dialog_effects::memory::Retract);
forward!(dialog_effects::space::Load);
forward!(dialog_effects::space::Create);
forward!(Prove<Ucan>);
forward!(Retain<Ucan>);

#[async_trait::async_trait]
impl Provider<Authorize<Ucan>> for AccountBoundOperator {
    async fn execute(
        &self,
        input: Capability<Authorize<Ucan>>,
    ) -> Result<UcanAuthorization, AuthorizeError> {
        let guard = crate::account_session::shared_remote_guard(&self.store).map_err(|error| {
            AuthorizeError::Unavailable {
                detail: error.to_string(),
            }
        })?;
        self.authorize_guarded(input, &guard).await
    }
}

struct Guarded<'a> {
    operator: &'a AccountBoundOperator,
    guard: &'a AccountSessionReadGuard,
}

#[async_trait::async_trait]
impl<'a> Provider<Identify> for Guarded<'a> {
    async fn execute(&self, input: <Identify as Command>::Input) -> <Identify as Command>::Output {
        <Operator<NativeSpace> as Provider<Identify>>::execute(&self.operator.inner, input).await
    }
}

macro_rules! forward_guarded {
    ($command:ty) => {
        #[async_trait::async_trait]
        impl<'a> Provider<$command> for Guarded<'a>
        where
            Operator<NativeSpace>: Provider<$command> + ConditionalSync,
            <$command as Command>::Input: ConditionalSend,
            <$command as Command>::Output: ConditionalSend,
        {
            async fn execute(
                &self,
                input: <$command as Command>::Input,
            ) -> <$command as Command>::Output {
                <Operator<NativeSpace> as Provider<$command>>::execute(&self.operator.inner, input)
                    .await
            }
        }
    };
}

forward_guarded!(dialog_effects::credential::Load<dialog_effects::credential::Secret>);
forward_guarded!(dialog_effects::credential::Load<dialog_credentials::Credential>);
forward_guarded!(Prove<Ucan>);

#[async_trait::async_trait]
impl<'a> Provider<Authorize<Ucan>> for Guarded<'a> {
    async fn execute(
        &self,
        input: Capability<Authorize<Ucan>>,
    ) -> Result<UcanAuthorization, AuthorizeError> {
        self.operator.authorize_guarded(input, self.guard).await
    }
}

trait FromAuthError {
    fn from_auth_error(error: AuthorizeError) -> Self;
}

impl<T, E: From<AuthorizeError>> FromAuthError for std::result::Result<T, E> {
    fn from_auth_error(error: AuthorizeError) -> Self {
        Err(E::from(error))
    }
}

#[async_trait::async_trait]
impl<At, Fx> Provider<Fork<At, Fx>> for AccountBoundOperator
where
    At: Site,
    At::Fork<Fx>: for<'a> SiteFork<Guarded<'a>, Site = At, Effect = Fx> + ConditionalSend,
    Fx: Effect + 'static,
    Fx::Output: FromAuthError + ConditionalSend,
    Fork<At, Fx>: ConditionalSend,
    ForkInvocation<At, Fx>: ConditionalSend,
    Network: Provider<ForkInvocation<At, Fx>> + ConditionalSync,
{
    async fn execute(&self, input: Fork<At, Fx>) -> Fx::Output {
        let guard = match crate::account_session::shared_remote_guard(&self.store) {
            Ok(guard) => guard,
            Err(error) => {
                return FromAuthError::from_auth_error(AuthorizeError::Unavailable {
                    detail: error.to_string(),
                });
            }
        };
        let env = Guarded {
            operator: self,
            guard: &guard,
        };
        match input.authorize(&env).await {
            Ok(invocation) => invocation.perform(&Network::default()).await,
            Err(error) => FromAuthError::from_auth_error(error),
        }
    }
}

/// Initialize canonical session state before exposing an account-bound
/// operator.
pub async fn wrap(
    inner: Operator<NativeSpace>,
    profile: Profile,
    store: SpotStore,
    require_account: bool,
) -> Result<AccountBoundOperator> {
    let guard = crate::account_session::exclusive_transition_guard(&store)?;
    crate::account_session::ensure_initialized(&profile, &inner, &guard)
        .await
        .context("failed to initialize account-session state")?;
    drop(guard);
    Ok(AccountBoundOperator::new(
        inner,
        profile,
        store,
        require_account,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_capability::access::{Certificate as _, CertificateStore};
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan::{Parameters, Scope, UcanCertificate, UcanDelegation, UcanProof};
    use dialog_ucan_core::command::Command as UcanCommand;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::time::Timestamp;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;

    struct OrderedStore(Vec<UcanCertificate>);

    #[async_trait::async_trait]
    impl CertificateStore<Ucan> for OrderedStore {
        async fn list(
            &self,
            audience: &dialog_varsig::Did,
            subject: Option<&dialog_varsig::Did>,
        ) -> Result<Vec<UcanCertificate>, AuthorizeError> {
            Ok(self
                .0
                .iter()
                .filter(|certificate| {
                    certificate.audience() == audience && certificate.subject() == subject
                })
                .cloned()
                .collect())
        }

        async fn save(&self, _delegation: &UcanDelegation) -> Result<(), AuthorizeError> {
            Ok(())
        }

        async fn export(&self) -> Result<Vec<UcanCertificate>, AuthorizeError> {
            Ok(self.0.clone())
        }

        async fn forget(&self, _certificates: &[UcanCertificate]) -> Result<(), AuthorizeError> {
            Ok(())
        }
    }

    #[test]
    fn remote_authorization_requires_a_current_in_flight_window() {
        let resource = dialog_capability::did!("key:zResource");
        let input = Subject::from(dialog_capability::did!("key:zProfile"))
            .attenuate(Access)
            .invoke(Authorize::<Ucan>::new(
                dialog_capability::did!("key:zOperator"),
                Scope {
                    subject: UcanSubject::Specific(resource),
                    command: UcanCommand::new(vec!["storage".to_string()]),
                    parameters: Parameters::default(),
                },
            ));

        let bounded = require_current_window(input, 1_000);
        let request = Authorize::<Ucan>::of(&bounded);

        assert_eq!(request.duration.not_before, Some(1_000));
        assert_eq!(request.duration.expiration, Some(1_060));
    }

    #[dialog_common::test]
    async fn remote_authorization_skips_an_expired_historical_session() {
        let resource = Ed25519Signer::import(&[21; 32])
            .await
            .expect("resource signer should import");
        let operator = Ed25519Signer::import(&[22; 32])
            .await
            .expect("operator signer should import");
        let session = |expiration| {
            DelegationBuilder::new()
                .issuer(dialog_credentials::Signer::from(resource.clone()))
                .audience(&operator.did())
                .subject(UcanSubject::Specific(resource.did()))
                .command(vec![])
                .expiration(Timestamp::try_from(expiration as i128).unwrap())
        };
        let expired = session(900)
            .try_build()
            .await
            .expect("expired session should build");
        let fresh = session(2_000)
            .try_build()
            .await
            .expect("fresh session should build");
        let expected = fresh.to_cid();
        let store = OrderedStore(vec![UcanCertificate(expired), UcanCertificate(fresh)]);
        let input = Subject::from(resource.did())
            .attenuate(Access)
            .invoke(Authorize::<Ucan>::new(
                operator.did(),
                Scope {
                    subject: UcanSubject::Specific(resource.did()),
                    command: UcanCommand::new(vec![]),
                    parameters: Parameters::default(),
                },
            ));
        let bounded = require_current_window(input, 1_000);
        let request = Authorize::<Ucan>::of(&bounded);
        let proof_request = Prove::<Ucan>::new(request.principal.clone(), request.access.clone())
            .during(request.duration);

        let proof: UcanProof = store
            .prove(proof_request)
            .await
            .expect("a fresh session should prove access");
        let selected = DelegationChain::new(proof.proofs[0].0.clone());

        assert_eq!(selected.proof_cids()[0], expected);
    }
}
