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

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_capability::{Capability, Provider, Subject};
use dialog_common::ConditionalSync;
use dialog_credentials::{DidKeyResolver, Ed25519Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{Container, Delegation, Invocation, InvocationBuilder, InvocationChain};
use dialog_varsig::AnySignature;
use dialog_varsig::{Did, Principal};
use ipld_core::cid::Cid;
use ipld_core::ipld::Ipld;
use ipld_core::serde::from_ipld;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tonk_account::customer::{
    Activate, Add, ConsumerReceipt, Customer, CustomerStatus, Enroll, Provider as ProviderRole,
    Receipt, RegistrationError, deposit_scopes,
};

use crate::email::EmailSender;
use crate::store::{SIGNUP_PLAN, Store, StoreError};

/// The command path segments of [`Enroll`], as they appear in an
/// invocation. Pinned to the capability-derived ability by a test.
pub const ENROLL_COMMAND: [&str; 2] = ["customer", "enroll"];

/// The command path segments of [`Activate`].
pub const ACTIVATE_COMMAND: [&str; 2] = ["customer", "activate"];

/// The command path segments of [`Add`].
pub const PROVIDER_ADD_COMMAND: [&str; 2] = ["provider", "add"];

/// The command path a consent delegation must grant, or a prefix of it.
pub const CONSUMER_PROVISION_COMMAND: [&str; 2] = ["consumer", "provision"];

/// How far in the future an enrollment invocation's mandatory
/// expiration may sit: the five-minute ceremony window plus a one-minute
/// allowance for clock skew. Mirrors the account service. Activation is
/// exempt: its invocation is the one this service minted, alive for
/// `EMAIL_TOKEN_TTL`.
const CEREMONY_WINDOW_SECONDS: u64 = 5 * 60 + 60;

/// The terms-of-service version the activation page presents, baked into
/// the minted activation invocation so the recorded acceptance names it.
pub const SIGNUP_TERMS: &str = "2026-08";

/// A registration command recognized at the `/ucan/` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationCommand {
    /// `/customer/enroll`
    Enroll,
    /// `/customer/activate`
    Activate,
    /// `/provider/add`
    ProviderAdd,
}

/// Peek at a container's invocation command without verifying anything.
/// `None` on any parse failure or unrecognized command, so the caller
/// falls through to the presign path and its own error mapping.
pub fn registration_command(container_bytes: &[u8]) -> Option<RegistrationCommand> {
    let tokens = Container::from_bytes(container_bytes).ok()?.into_tokens();
    let invocation: Invocation<AnySignature> =
        serde_ipld_dagcbor::from_slice(tokens.first()?).ok()?;
    let segments: Vec<&str> = invocation.command().0.iter().map(String::as_str).collect();
    match segments.as_slice() {
        segments if segments == ENROLL_COMMAND => Some(RegistrationCommand::Enroll),
        segments if segments == ACTIVATE_COMMAND => Some(RegistrationCommand::Activate),
        segments if segments == PROVIDER_ADD_COMMAND => Some(RegistrationCommand::ProviderAdd),
        _ => None,
    }
}

/// The JSON answer to a registration invocation: a customer receipt for
/// the `/customer` commands, a consumer receipt for `/provider/add`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Answer {
    /// `/customer/enroll` and `/customer/activate` answer the customer.
    Customer(Receipt),
    /// `/provider/add` answers the provisioned consumer.
    Consumer(ConsumerReceipt),
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
    pub async fn handle(&self) -> Result<Answer, RegistrationError>
    where
        Self: Provider<Enroll> + Provider<Activate> + Provider<Add>,
    {
        match registration_command(self.container) {
            Some(RegistrationCommand::Enroll) => {
                let chain = self
                    .verified_chain(&ENROLL_COMMAND, Some(CEREMONY_WINDOW_SECONDS))
                    .await?;
                let effect: Enroll = deserialize_arguments(chain.arguments())?;
                Ok(Answer::Customer(
                    Subject::from(chain.subject().clone())
                        .attenuate(Customer)
                        .invoke(effect)
                        .perform(self)
                        .await?,
                ))
            }
            Some(RegistrationCommand::ProviderAdd) => {
                let chain = self
                    .verified_chain(&PROVIDER_ADD_COMMAND, Some(CEREMONY_WINDOW_SECONDS))
                    .await?;
                let effect: Add = deserialize_arguments(chain.arguments())?;
                Ok(Answer::Consumer(
                    Subject::from(chain.subject().clone())
                        .attenuate(ProviderRole)
                        .invoke(effect)
                        .perform(self)
                        .await?,
                ))
            }
            Some(RegistrationCommand::Activate) => {
                // The presented invocation is the one enrollment emailed:
                // self-issued by this service, alive for EMAIL_TOKEN_TTL
                // rather than a ceremony window. `verify` already refuses
                // any other issuer for a proofless chain on this subject;
                // the explicit check keeps the intent visible.
                let chain = self.verified_chain(&ACTIVATE_COMMAND, None).await?;
                let service = self.service.did();
                if chain.subject() != &service || chain.issuer() != &service {
                    return Err(RegistrationError::Forbidden {
                        message: format!(
                            "activation must present the invocation this service minted, got \
                             subject {} issued by {}",
                            chain.subject(),
                            chain.issuer()
                        ),
                    });
                }
                let effect: Activate = deserialize_arguments(chain.arguments())?;
                Ok(Answer::Customer(
                    Subject::from(service)
                        .attenuate(Customer)
                        .invoke(effect)
                        .perform(self)
                        .await?,
                ))
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

        self.verify_deposits(&effect.access, &customer).await?;
        // What gets stored is everything the deposit needs to be
        // exercised later: the scoped heads together with the chain
        // links that walk them back to the customer, re-encoded as a
        // container of their exact bytes.
        let access = Container::new(
            self.delegation_tokens()?
                .into_iter()
                .map(|token| token.bytes)
                .collect(),
        )
        .to_bytes()
        .map_err(|err| RegistrationError::Internal {
            message: format!("encoding the access deposit failed: {err}"),
        })?;

        match self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?
        {
            None => {
                self.store
                    .enroll_customer(customer.as_str(), &address, &access, SIGNUP_PLAN, self.now)
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

    /// Execute a verified `/customer/activate`: record terms acceptance
    /// and promote the customer to `Active`. The invocation is the one
    /// this service minted at enroll, so its arguments are trusted as
    /// written; who presents it does not matter. Activating twice is a
    /// no-op success.
    pub async fn activate(
        &self,
        capability: Capability<Activate>,
    ) -> Result<Receipt, RegistrationError> {
        let effect = capability.into_effect();
        let customer = effect.customer.clone();

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

    /// Execute a verified `/provider/add`: validate the deposited consent
    /// and provision the consumer under the invoking customer.
    /// Re-provisioning under the same customer is a no-op success; a
    /// consumer another customer provides is refused. Activation is not
    /// required to add, only to serve: consumers added while the provider
    /// is `Registered` go live with its activation.
    pub async fn add(
        &self,
        capability: Capability<Add>,
    ) -> Result<ConsumerReceipt, RegistrationError> {
        let provider = capability.subject().clone();
        let effect = capability.into_effect();
        match self
            .store
            .customer(provider.as_str())
            .await
            .map_err(internal)?
        {
            None => return Err(RegistrationError::UnknownCustomer),
            Some(customer) if customer.status == CustomerStatus::Suspended => {
                return Err(RegistrationError::CustomerSuspended);
            }
            Some(_) => {}
        }
        let consent = self.deposited_delegation(&effect.consent.to_string())?;
        self.verify_consent(&consent.delegation, &effect.consumer, &provider)
            .await?;
        if !self
            .store
            .add_consumer(effect.consumer.as_str(), provider.as_str(), self.now)
            .await
            .map_err(internal)?
        {
            return Err(RegistrationError::ConsumerProvided);
        }
        Ok(ConsumerReceipt {
            consumer: effect.consumer,
            provider,
        })
    }

    /// Validate the deposited consent: the consumer's own delegation to
    /// the invoking customer, granting `/consumer/provision` or broader,
    /// signature-valid and inside its window. The audience is what binds
    /// it, so a consent given to one customer cannot enrol a different
    /// one; a powerline delegation from the space satisfies it as-is.
    async fn verify_consent(
        &self,
        consent: &Delegation<AnySignature>,
        consumer: &Did,
        provider: &Did,
    ) -> Result<(), RegistrationError> {
        if consent.issuer() != consumer {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "consent must be issued by {consumer}, got {}",
                    consent.issuer()
                ),
            });
        }
        if consent.audience() != provider {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "consent was issued to {}, not the invoking customer",
                    consent.audience()
                ),
            });
        }
        if let DelegatedSubject::Specific(subject) = consent.subject()
            && subject != consumer
        {
            return Err(RegistrationError::Forbidden {
                message: format!("consent must cover the consumer space, got {subject}"),
            });
        }
        let granted = &consent.command().0;
        if !granted
            .iter()
            .zip(CONSUMER_PROVISION_COMMAND.iter())
            .all(|(granted, required)| granted == required)
            || granted.len() > CONSUMER_PROVISION_COMMAND.len()
        {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "consent grants /{}, which does not cover /consumer/provision",
                    granted.join("/")
                ),
            });
        }
        self.check_window(consent)?;
        consent
            .verify_signature(&DidKeyResolver)
            .await
            .map_err(|err| RegistrationError::Unauthorized {
                message: format!("consent failed to verify: {err}"),
            })
    }

    /// Mint the activation invocation and wrap it into a link. The
    /// invocation is complete and service-signed: the accept button
    /// presents it as-is, so activation needs no key on the presenting
    /// device and a click on any device finalizes.
    pub async fn activation_link(&self, customer: &Did) -> Result<String, RegistrationError> {
        let expiration = timestamp(self.now + self.activation_ttl)?;
        let service = self.service.did();
        let invocation = InvocationBuilder::new()
            .issuer(dialog_credentials::Signer::from(self.service.clone()))
            .audience(&service)
            .subject(&service)
            .command(ACTIVATE_COMMAND.iter().map(ToString::to_string).collect())
            .arguments(BTreeMap::from([
                (
                    "customer".to_string(),
                    Promised::String(customer.to_string()),
                ),
                ("terms".to_string(), Promised::String(SIGNUP_TERMS.into())),
            ]))
            .proofs(vec![])
            .expiration(expiration)
            .try_build()
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("minting activation failed: {err}"),
            })?;
        let bytes = InvocationChain::new(invocation, HashMap::new())
            .to_bytes()
            .map_err(|err| RegistrationError::Internal {
                message: format!("encoding activation failed: {err}"),
            })?;
        Ok(format!(
            "{}/activate?ucan={}",
            self.origin.trim_end_matches('/'),
            URL_SAFE_NO_PAD.encode(bytes)
        ))
    }

    /// Parse and cryptographically verify the container, require the
    /// exact command, and require an expiration that has not passed and,
    /// when a window is given, does not sit further ahead than it.
    async fn verified_chain(
        &self,
        expected_command: &[&str],
        window: Option<u64>,
    ) -> Result<InvocationChain<AnySignature>, RegistrationError> {
        let chain = InvocationChain::try_from(self.container).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("bad invocation container: {err}"),
            }
        })?;
        chain
            .verify(&DidKeyResolver)
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
        if let Some(window) = window
            && expiration > self.now + window
        {
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
                let delegation: Delegation<AnySignature> = serde_ipld_dagcbor::from_slice(&bytes)
                    .map_err(|err| {
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

    /// Validate the deposited access delegations against the expected
    /// [`deposit_scopes`]: each named deposit must match one scope
    /// exactly — its command and its policy — and together they must
    /// cover every scope, so the service ends up holding precisely its
    /// own branch of the account space and the index catalog backing it.
    /// A broader grant, `/` included, is refused rather than stored.
    async fn verify_deposits(
        &self,
        access: &[Cid],
        customer: &Did,
    ) -> Result<(), RegistrationError> {
        let expected = deposit_scopes(customer, &self.service.did());
        let mut covered = [false; 2];
        for cid in access {
            let deposit = self.deposited_delegation(&cid.to_string())?;
            let matched = expected.iter().position(|scope| {
                deposit.delegation.command().segments() == scope.command.segments()
                    && deposit.delegation.policy() == &scope.policy()
            });
            let Some(index) = matched else {
                return Err(RegistrationError::Forbidden {
                    message: format!(
                        "deposit /{} is broader than the scopes enrollment accepts",
                        deposit.delegation.command().segments().join("/")
                    ),
                });
            };
            covered[index] = true;
            self.verify_deposit(&deposit.delegation, customer).await?;
        }
        if covered != [true; 2] {
            return Err(RegistrationError::Forbidden {
                message: "the deposit must cover the service's branch in memory and the index \
                          catalog"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Validate one deposited access delegation: issued under the
    /// customer's authority to this service, signature-valid, and inside
    /// its own time window. It is an argument being deposited, not a
    /// proof, so it never extends the invocation's chain.
    ///
    /// The head need not be issued by the customer directly: the device
    /// holds the customer's root through a delegation, so the deposit
    /// arrives as a chain (customer → device → service) whose links
    /// travel in the same container. The walk follows issuers back to
    /// the customer through those links.
    async fn verify_deposit(
        &self,
        deposit: &Delegation<AnySignature>,
        customer: &Did,
    ) -> Result<(), RegistrationError> {
        let service = self.service.did();
        if deposit.audience() != &service {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "access delegation must be issued to this service, got {}",
                    deposit.audience()
                ),
            });
        }
        self.check_deposit_link(deposit, customer).await?;

        let links = self.delegation_tokens()?;
        let mut issuer = deposit.issuer().clone();
        let mut depth = 0;
        while issuer != *customer {
            depth += 1;
            if depth > 4 {
                return Err(RegistrationError::Forbidden {
                    message: "access delegation chain is too deep".to_string(),
                });
            }
            let link = links
                .iter()
                .find(|token| token.delegation.audience() == &issuer)
                .ok_or_else(|| RegistrationError::Forbidden {
                    message: format!(
                        "access delegation is not issued under {customer}: nothing in the \
                         container grants {issuer}"
                    ),
                })?;
            self.check_deposit_link(&link.delegation, customer).await?;
            issuer = link.delegation.issuer().clone();
        }
        Ok(())
    }

    /// Validate one link of a deposit chain: its subject covers the
    /// customer, its window contains the present, and its signature is
    /// its issuer's.
    async fn check_deposit_link(
        &self,
        delegation: &Delegation<AnySignature>,
        customer: &Did,
    ) -> Result<(), RegistrationError> {
        if let DelegatedSubject::Specific(subject) = delegation.subject()
            && subject != customer
        {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "access delegation must cover the customer's account space, got {subject}"
                ),
            });
        }
        self.check_window(delegation)?;
        delegation
            .verify_signature(&DidKeyResolver)
            .await
            .map_err(|err| RegistrationError::Unauthorized {
                message: format!("access delegation failed to verify: {err}"),
            })
    }

    /// Refuse a delegation whose time window does not contain the
    /// present.
    fn check_window(&self, delegation: &Delegation<AnySignature>) -> Result<(), RegistrationError> {
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

// [`Store`] and [`EmailSender`] are declared through the same dual
// `async_trait` forms as `Provider` itself, so their futures carry the
// platform-conditional `Send` and one generic impl serves every
// environment. `ConditionalSync` bounds cover the `&self` captures.

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S, E> Provider<Enroll> for Registration<'_, S, E>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
{
    async fn execute(&self, input: Capability<Enroll>) -> Result<Receipt, RegistrationError> {
        self.enroll(input).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S, E> Provider<Activate> for Registration<'_, S, E>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
{
    async fn execute(&self, input: Capability<Activate>) -> Result<Receipt, RegistrationError> {
        self.activate(input).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S, E> Provider<Add> for Registration<'_, S, E>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
{
    async fn execute(&self, input: Capability<Add>) -> Result<ConsumerReceipt, RegistrationError> {
        self.add(input).await
    }
}

/// A delegation token as it appeared in the container: the decoded
/// delegation together with its exact bytes.
struct DelegationToken {
    delegation: Delegation<AnySignature>,
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
            access: vec![Cid::default()],
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
