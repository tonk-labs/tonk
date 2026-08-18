//! Customer registration: `/customer/enroll` and `/customer/activate`.
//!
//! Both commands arrive at `POST /ucan/` as UCAN invocation containers,
//! like every other invocation this service accepts. The capability
//! types live in [`tonk_account::customer`], shared with clients, so the
//! command paths and argument shapes have one definition; this module
//! verifies the chains, decodes the arguments into those capabilities,
//! and executes them as a [`Provider`] environment over [`Store`] and
//! [`EmailSender`].
//!
//! Enrollment roots at the customer's own self-certifying DID.
//! Activation roots at the service's subject: enrollment mints a
//! delegation of `/customer/activate` audience-bound to the customer,
//! emails it as a link, and the customer finalizes by invoking it, so
//! the link is not a bearer credential.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_capability::{Capability, Provider, Subject};
use dialog_credentials::{Ed25519KeyResolver, Ed25519Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{
    Container, Delegation, DelegationBuilder, DelegationChain, Invocation, InvocationChain,
};
use dialog_varsig::algorithm::eddsa::Ed25519Signature;
use dialog_varsig::{Did, Principal};
use ipld_core::ipld::Ipld;
use ipld_core::serde::from_ipld;
use serde::de::DeserializeOwned;
use tonk_account::customer::{
    Activate, Customer, CustomerStatus, Enroll, Receipt, RegistrationError,
};

use crate::email::EmailSender;
use crate::store::{SIGNUP_PLAN, Store, StoreError};

/// The command path segments of [`Enroll`], as they appear in an
/// invocation. Pinned to the capability-derived ability by a test.
pub const ENROLL_COMMAND: [&str; 2] = ["customer", "enroll"];

/// The command path segments of [`Activate`].
pub const ACTIVATE_COMMAND: [&str; 2] = ["customer", "activate"];

/// How far in the future a registration invocation's mandatory
/// expiration may sit: the five-minute ceremony window plus a one-minute
/// allowance for clock skew. Mirrors the account service.
const CEREMONY_WINDOW_SECONDS: u64 = 5 * 60 + 60;

/// A registration command recognized at the `/ucan/` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationCommand {
    /// `/customer/enroll`
    Enroll,
    /// `/customer/activate`
    Activate,
}

/// Peek at a container's invocation command without verifying anything.
/// `None` on any parse failure or unrecognized command, so the caller
/// falls through to the presign path and its own error mapping.
pub fn registration_command(container_bytes: &[u8]) -> Option<RegistrationCommand> {
    let tokens = Container::from_bytes(container_bytes).ok()?.into_tokens();
    let invocation: Invocation<Ed25519Signature> =
        serde_ipld_dagcbor::from_slice(tokens.first()?).ok()?;
    let segments: Vec<&str> = invocation.command().0.iter().map(String::as_str).collect();
    match segments.as_slice() {
        segments if segments == ENROLL_COMMAND => Some(RegistrationCommand::Enroll),
        segments if segments == ACTIVATE_COMMAND => Some(RegistrationCommand::Activate),
        _ => None,
    }
}

/// The environment a registration invocation executes against: storage,
/// email delivery, the service's signing identity, and the request's
/// origin, clock, and container.
pub struct Registration<'a, S, E> {
    /// Control-state storage.
    pub store: &'a S,
    /// Activation email delivery.
    pub email: &'a E,
    /// The service's signing identity, issuer of activation delegations.
    pub service: &'a Ed25519Signer,
    /// Origin the activation link points at, e.g. `https://hub.tonk.xyz`.
    pub origin: &'a str,
    /// Lifetime of the emailed activation delegation, in seconds.
    pub activation_ttl: u64,
    /// The current time, as a unix timestamp in seconds.
    pub now: u64,
    /// The exact container bytes of the invocation being handled.
    pub container: &'a [u8],
}

impl<S: Store, E: EmailSender> Registration<'_, S, E> {
    /// Verify the container, decode its capability, and perform it
    /// against this environment.
    pub async fn handle(&self) -> Result<Receipt, RegistrationError>
    where
        Self: Provider<Enroll> + Provider<Activate>,
    {
        match registration_command(self.container) {
            Some(RegistrationCommand::Enroll) => {
                let chain = self.verified_chain(&ENROLL_COMMAND).await?;
                let effect: Enroll = deserialize_arguments(chain.arguments())?;
                Subject::from(chain.subject().clone())
                    .attenuate(Customer)
                    .invoke(effect)
                    .perform(self)
                    .await
            }
            Some(RegistrationCommand::Activate) => {
                let chain = self.verified_chain(&ACTIVATE_COMMAND).await?;
                let service = self.service.did();
                if chain.subject() != &service {
                    return Err(RegistrationError::Forbidden {
                        message: format!(
                            "activation subject must be this service, got {}",
                            chain.subject()
                        ),
                    });
                }
                let effect: Activate = deserialize_arguments(chain.arguments())?;
                Subject::from(service)
                    .attenuate(Customer)
                    .invoke(effect)
                    .perform(self)
                    .await
            }
            None => Err(RegistrationError::Invalid {
                message: "not a registration invocation".to_string(),
            }),
        }
    }

    /// Execute a verified `/customer/enroll`: validate the deposited
    /// access delegation, write the customer with its self-provided
    /// account consumer, and email an activation link. Re-enrolling while
    /// `Registered` is idempotent and resends the link.
    pub async fn enroll(
        &self,
        capability: Capability<Enroll>,
    ) -> Result<Receipt, RegistrationError> {
        let customer = capability.subject().clone();
        let effect = capability.into_effect();
        let address = effect.email.trim().to_string();
        if !address.contains('@') || address.len() > 254 {
            return Err(RegistrationError::Invalid {
                message: "email must be a plausible address".to_string(),
            });
        }

        let deposit = self.deposited_delegation(&effect.access.to_string())?;
        self.verify_deposit(&deposit.delegation, &customer).await?;

        match self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?
        {
            None => {
                self.store
                    .enroll_customer(
                        customer.as_str(),
                        &address,
                        &deposit.bytes,
                        SIGNUP_PLAN,
                        self.now,
                    )
                    .await
                    .map_err(internal)?;
            }
            Some(existing) => match existing.status {
                CustomerStatus::Registered => {
                    if existing.email != address {
                        self.store
                            .update_registered_email(customer.as_str(), &address)
                            .await
                            .map_err(internal)?;
                    }
                }
                CustomerStatus::Active => return Err(RegistrationError::CustomerActive),
                CustomerStatus::Suspended => return Err(RegistrationError::CustomerSuspended),
            },
        }

        let link = self.activation_link(&customer).await?;
        self.email
            .send_activation(&address, &link)
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("activation email failed: {err:?}"),
            })?;

        Ok(Receipt {
            customer,
            status: CustomerStatus::Registered,
        })
    }

    /// Execute a verified `/customer/activate`: enforce the emailed
    /// delegation's window and audience, record terms acceptance, and
    /// promote the customer to `Active`. Activating twice is a no-op
    /// success.
    pub async fn activate(
        &self,
        capability: Capability<Activate>,
    ) -> Result<Receipt, RegistrationError> {
        let effect = capability.into_effect();
        let customer = effect.customer.clone();
        let service = self.service.did();

        // `InvocationChain::verify` checks that time windows are
        // structurally coherent, not that they contain the present, so
        // the emailed delegation's `exp` is enforced here against the
        // clock. The same walk pins the activated customer to the
        // audience of the service-issued link, so a link minted for one
        // customer cannot activate another.
        let mut link_audience = None;
        for token in self.delegation_tokens()? {
            self.check_window(&token.delegation)?;
            if token.delegation.issuer() == &service {
                link_audience = Some(token.delegation.audience().clone());
            }
        }
        match link_audience {
            Some(audience) if audience == customer => {}
            Some(audience) => {
                return Err(RegistrationError::Forbidden {
                    message: format!("activation link was issued to {audience}, not {customer}"),
                });
            }
            None => {
                return Err(RegistrationError::Unauthorized {
                    message: "activation requires the emailed service delegation".to_string(),
                });
            }
        }

        if self
            .store
            .activate_customer(customer.as_str(), &effect.terms, self.now)
            .await
            .map_err(internal)?
        {
            return Ok(Receipt {
                customer,
                status: CustomerStatus::Active,
            });
        }
        match self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?
        {
            Some(existing) if existing.status == CustomerStatus::Active => Ok(Receipt {
                customer,
                status: CustomerStatus::Active,
            }),
            Some(_) => Err(RegistrationError::CustomerSuspended),
            None => Err(RegistrationError::UnknownCustomer),
        }
    }

    /// Mint the activation delegation and wrap it into a link.
    pub async fn activation_link(&self, customer: &Did) -> Result<String, RegistrationError> {
        let expiration = timestamp(self.now + self.activation_ttl)?;
        let delegation = DelegationBuilder::new()
            .issuer(self.service.clone())
            .audience(customer)
            .subject(DelegatedSubject::Specific(self.service.did()))
            .command(ACTIVATE_COMMAND.iter().map(ToString::to_string).collect())
            .expiration(expiration)
            .try_build()
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("minting activation failed: {err}"),
            })?;
        let bytes = DelegationChain::new(delegation).to_bytes().map_err(|err| {
            RegistrationError::Internal {
                message: format!("encoding activation failed: {err}"),
            }
        })?;
        Ok(format!(
            "{}/activate?ucan={}",
            self.origin.trim_end_matches('/'),
            URL_SAFE_NO_PAD.encode(bytes)
        ))
    }

    /// Parse and cryptographically verify the container, require the
    /// exact command, and require a fresh, bounded invocation expiration.
    async fn verified_chain(
        &self,
        expected_command: &[&str],
    ) -> Result<InvocationChain<Ed25519Signature>, RegistrationError> {
        let chain = InvocationChain::try_from(self.container).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("bad invocation container: {err}"),
            }
        })?;
        chain
            .verify(&Ed25519KeyResolver)
            .await
            .map_err(|err| RegistrationError::Unauthorized {
                message: format!("invocation failed to verify: {err}"),
            })?;

        let segments: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
        if segments.as_slice() != expected_command {
            return Err(RegistrationError::Forbidden {
                message: format!("expected command {expected_command:?}, got {segments:?}"),
            });
        }

        let expiration =
            chain
                .invocation
                .expiration()
                .ok_or_else(|| RegistrationError::Unauthorized {
                    message: "invocation must carry an expiration".to_string(),
                })?;
        let expiration = expiration.to_unix();
        if expiration < self.now {
            return Err(RegistrationError::Unauthorized {
                message: "invocation has expired".to_string(),
            });
        }
        if expiration > self.now + CEREMONY_WINDOW_SECONDS {
            return Err(RegistrationError::Unauthorized {
                message: "invocation expiration exceeds the ceremony window".to_string(),
            });
        }
        Ok(chain)
    }

    /// Decode every delegation token in the container. The invocation
    /// token is skipped; a token that does not decode as a delegation is
    /// an error.
    fn delegation_tokens(&self) -> Result<Vec<DelegationToken>, RegistrationError> {
        let tokens = Container::from_bytes(self.container)
            .map_err(|err| RegistrationError::Invalid {
                message: format!("bad container: {err}"),
            })?
            .into_tokens();
        tokens
            .into_iter()
            .skip(1)
            .enumerate()
            .map(|(index, bytes)| {
                let delegation: Delegation<Ed25519Signature> =
                    serde_ipld_dagcbor::from_slice(&bytes).map_err(|err| {
                        RegistrationError::Invalid {
                            message: format!("failed to decode delegation {index}: {err}"),
                        }
                    })?;
                Ok(DelegationToken { delegation, bytes })
            })
            .collect()
    }

    /// Find the deposited access delegation the `access` argument names.
    fn deposited_delegation(&self, cid: &str) -> Result<DelegationToken, RegistrationError> {
        self.delegation_tokens()?
            .into_iter()
            .find(|token| token.delegation.to_cid().to_string() == cid)
            .ok_or_else(|| RegistrationError::Invalid {
                message: format!(
                    "the access argument names {cid}, which the container does not carry"
                ),
            })
    }

    /// Validate the deposited access delegation: issued by the customer
    /// to this service, signature-valid, and inside its own time window.
    /// It is an argument being deposited, not a proof, so it never
    /// extends the invocation's chain.
    async fn verify_deposit(
        &self,
        deposit: &Delegation<Ed25519Signature>,
        customer: &Did,
    ) -> Result<(), RegistrationError> {
        let service = self.service.did();
        if deposit.issuer() != customer {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "access delegation must be issued by {customer}, got {}",
                    deposit.issuer()
                ),
            });
        }
        if deposit.audience() != &service {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "access delegation must be issued to this service, got {}",
                    deposit.audience()
                ),
            });
        }
        if let DelegatedSubject::Specific(subject) = deposit.subject()
            && subject != customer
        {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "access delegation must cover the customer's account space, got {subject}"
                ),
            });
        }
        self.check_window(deposit)?;
        deposit
            .verify_signature(&Ed25519KeyResolver)
            .await
            .map_err(|err| RegistrationError::Unauthorized {
                message: format!("access delegation failed to verify: {err}"),
            })
    }

    /// Refuse a delegation whose time window does not contain the
    /// present.
    fn check_window(
        &self,
        delegation: &Delegation<Ed25519Signature>,
    ) -> Result<(), RegistrationError> {
        if let Some(expiration) = delegation.expiration()
            && expiration.to_unix() < self.now
        {
            return Err(RegistrationError::Unauthorized {
                message: "a presented delegation has expired".to_string(),
            });
        }
        if let Some(not_before) = delegation.not_before()
            && not_before.to_unix() > self.now
        {
            return Err(RegistrationError::Unauthorized {
                message: "a presented delegation is not yet valid".to_string(),
            });
        }
        Ok(())
    }
}

// `Provider` is how dialog capabilities are performed, and its native
// declaration requires `Send` futures, which a generic `S: Store` cannot
// promise. So the impls are per concrete environment: the worker's D1 +
// Resend pair, and the helpers' sqlite + captured pair.

#[cfg(target_arch = "wasm32")]
mod worker_provider {
    use async_trait::async_trait;
    use dialog_capability::{Capability, Provider};
    use tonk_account::customer::{Activate, Enroll, Receipt, RegistrationError};

    use super::Registration;
    use crate::email::Resend;
    use crate::store::d1::D1Store;

    #[async_trait(?Send)]
    impl Provider<Enroll> for Registration<'_, D1Store, Resend> {
        async fn execute(&self, input: Capability<Enroll>) -> Result<Receipt, RegistrationError> {
            self.enroll(input).await
        }
    }

    #[async_trait(?Send)]
    impl Provider<Activate> for Registration<'_, D1Store, Resend> {
        async fn execute(&self, input: Capability<Activate>) -> Result<Receipt, RegistrationError> {
            self.activate(input).await
        }
    }
}

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
mod helper_provider {
    use async_trait::async_trait;
    use dialog_capability::{Capability, Provider};
    use tonk_account::customer::{Activate, Enroll, Receipt, RegistrationError};

    use super::Registration;
    use crate::email::CapturedEmail;
    use crate::store::sqlite::SqliteStore;

    #[async_trait]
    impl Provider<Enroll> for Registration<'_, SqliteStore, CapturedEmail> {
        async fn execute(&self, input: Capability<Enroll>) -> Result<Receipt, RegistrationError> {
            self.enroll(input).await
        }
    }

    #[async_trait]
    impl Provider<Activate> for Registration<'_, SqliteStore, CapturedEmail> {
        async fn execute(&self, input: Capability<Activate>) -> Result<Receipt, RegistrationError> {
            self.activate(input).await
        }
    }
}

/// A delegation token as it appeared in the container: the decoded
/// delegation together with its exact bytes.
struct DelegationToken {
    delegation: Delegation<Ed25519Signature>,
    bytes: Vec<u8>,
}

/// Decode a capability's caveats from an invocation's argument map.
/// Promised values become IPLD, then serde; unknown fields are ignored.
fn deserialize_arguments<T: DeserializeOwned>(
    arguments: &BTreeMap<String, Promised>,
) -> Result<T, RegistrationError> {
    let map: BTreeMap<String, Ipld> = arguments
        .iter()
        .map(|(key, value)| {
            Ipld::try_from(value)
                .map(|ipld| (key.clone(), ipld))
                .map_err(|err| RegistrationError::Invalid {
                    message: format!("unresolved promise for '{key}': {err}"),
                })
        })
        .collect::<Result<_, _>>()?;
    from_ipld(Ipld::Map(map)).map_err(|err| RegistrationError::Invalid {
        message: format!("invalid arguments: {err}"),
    })
}

/// Map a storage failure onto the wire refusal.
fn internal(err: StoreError) -> RegistrationError {
    RegistrationError::Internal {
        message: err.to_string(),
    }
}

/// Construct a [`Timestamp`] from unix seconds.
fn timestamp(seconds: u64) -> Result<Timestamp, RegistrationError> {
    Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).map_err(|err| {
        RegistrationError::Internal {
            message: format!("timestamp out of range: {err:?}"),
        }
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use ipld_core::cid::Cid;

    use super::*;

    #[dialog_common::test]
    fn it_pins_the_command_constants_to_the_capability_abilities() {
        let subject = Subject::from(
            "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                .parse::<Did>()
                .unwrap(),
        );
        let enroll = subject.clone().attenuate(Customer).invoke(Enroll {
            email: "alice@example.com".into(),
            access: Cid::default(),
        });
        assert_eq!(enroll.ability(), format!("/{}", ENROLL_COMMAND.join("/")));
        let activate = subject.attenuate(Customer).invoke(Activate {
            customer: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                .parse()
                .unwrap(),
            terms: "2026-08".into(),
        });
        assert_eq!(
            activate.ability(),
            format!("/{}", ACTIVATE_COMMAND.join("/"))
        );
    }
}
