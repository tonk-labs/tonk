//! Account-bound authorization and remote-dispatch boundary.

use anyhow::{Context, Result};
use dialog_capability::access::{Authorize, AuthorizeError, Prove, Retain};
use dialog_capability::{
    Capability, Command, Effect, Fork, ForkInvocation, Provider, Site, SiteFork,
};
use dialog_common::{ConditionalSend, ConditionalSync};
use dialog_effects::authority::Identify;
use dialog_operator::{Operator, Profile};
use dialog_repository::RemoteSite as Network;
use dialog_storage::provider::storage::NativeSpace;
use dialog_ucan::{Ucan, UcanAuthorization};
use dialog_ucan_core::DelegationChain;

use crate::account_session::{AccountSessionReadGuard, ActiveAccount};
use crate::spot::SpotStore;

/// Operator wrapper that forwards local effects but exclusively owns UCAN
/// authorization and every remote network fork.
pub struct AccountBoundOperator {
    inner: Operator<NativeSpace>,
    profile: Profile,
    store: SpotStore,
    require_account: bool,
}

impl AccountBoundOperator {
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
            .map_err(|error| AuthorizeError::Configuration(error.to_string()))?
            .ok_or_else(|| {
                AuthorizeError::Denied(
                    "log in with `tonk account link` before accessing a remote".to_string(),
                )
            })
    }

    async fn authorize_guarded(
        &self,
        input: Capability<Authorize<Ucan>>,
        guard: &AccountSessionReadGuard,
    ) -> Result<UcanAuthorization, AuthorizeError> {
        if !self.require_account {
            return input.perform(&self.inner).await;
        }
        // Reuse only the fresh signer/scope/duration and current
        // profile→operator session suffix. Historical account authority is
        // deliberately discarded.
        let mut authorization = input.perform(&self.inner).await?;
        let historical = authorization.chain.take().ok_or_else(|| {
            AuthorizeError::Denied("operator authorization has no session chain".to_string())
        })?;
        // The proven chain's first issuer is the repository authority being
        // exercised. The Authorize capability's own subject is the profile
        // context, not necessarily the repository subject.
        let subject = historical.issuer().clone();
        let session = historical.proofs().last().cloned().ok_or_else(|| {
            AuthorizeError::Denied("operator authorization has no session proof".to_string())
        })?;
        if session.issuer() != &self.profile.did() || session.audience() != &self.inner.did() {
            return Err(AuthorizeError::Denied(
                "operator session proof does not match this profile".to_string(),
            ));
        }

        let active = self.active(guard).await?;
        let root: dialog_varsig::Did = active.root_did.parse().map_err(|_| {
            AuthorizeError::Denied("active account root DID is invalid".to_string())
        })?;
        let grant_bytes = hex::decode(&active.delegation_hex).map_err(|_| {
            AuthorizeError::Denied("active account grant hex is invalid".to_string())
        })?;
        let grant = DelegationChain::try_from(grant_bytes.as_slice()).map_err(|error| {
            AuthorizeError::Denied(format!("active account grant is invalid: {error}"))
        })?;
        if grant.proof_cids().len() != 1
            || grant.proof_cids()[0].to_string() != active.delegation_cid
            || grant.issuer() != &root
            || grant.audience() != &self.profile.did()
        {
            return Err(AuthorizeError::Denied(
                "active account grant does not match canonical session state".to_string(),
            ));
        }
        grant
            .proofs()
            .next()
            .unwrap()
            .verify_signature(&dialog_credentials::Ed25519KeyResolver)
            .await
            .map_err(|error| {
                AuthorizeError::Denied(format!(
                    "active account grant signature is invalid: {error}"
                ))
            })?;

        let mut chain = if subject == root {
            grant
        } else {
            let bytes = self
                .profile
                .credential()
                .site(tonk_account::backup::space_root_site(&subject, &root))
                .load::<Vec<u8>>()
                .perform(&self.inner)
                .await
                .map_err(|error| {
                    AuthorizeError::Denied(format!(
                        "spot is not delegated to the active account: {error}"
                    ))
                })?;
            let backup = tonk_account::backup::AccountSpotBackup {
                chain_hex: hex::encode(bytes),
                remote_url: None,
                revocation_url: None,
                name: None,
            };
            let prefix = backup.validate_for(&root).await.map_err(|error| {
                AuthorizeError::Denied(format!(
                    "spot is not delegated to the active account: {error}"
                ))
            })?;
            let active_proof = grant.proofs().next().unwrap().clone();
            prefix.chain.push(active_proof).map_err(|error| {
                AuthorizeError::Denied(format!("account authority chain is invalid: {error}"))
            })?
        };
        chain = chain.push(session).map_err(|error| {
            AuthorizeError::Denied(format!("operator authority chain is invalid: {error}"))
        })?;
        authorization.chain = Some(chain);
        Ok(authorization)
    }
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
        let guard = crate::account_session::shared_remote_guard(&self.store)
            .map_err(|error| AuthorizeError::Configuration(error.to_string()))?;
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
                return FromAuthError::from_auth_error(AuthorizeError::Configuration(
                    error.to_string(),
                ));
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
