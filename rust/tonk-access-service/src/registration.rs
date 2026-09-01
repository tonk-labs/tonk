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
use dialog_ucan_core::container::bundle::InvocationBundle;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{
    Container, Delegation, DelegationBuilder, DelegationChain, Invocation, InvocationBuilder,
    InvocationChain,
};
use dialog_varsig::AnySignature;
use dialog_varsig::{Did, Principal};
use ipld_core::cid::Cid;
use ipld_core::ipld::Ipld;
use ipld_core::serde::from_ipld;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tonk_account::customer::{
    Activate, Add, ConsumerReceipt, Customer, CustomerStatus, Enroll, Ledger,
    Provider as ProviderRole, Receipt, RegistrationError,
};
use tonk_account::customer::{RESEND_INTERVAL_SECONDS, Resend as ResendActivation};
use tonk_account::subscription::{
    Archive as ArchiveSubscription, Resume as ResumeSubscription, Suspend as SuspendSubscription,
};

use crate::email::{EmailSender, normalize_email};
use crate::store::{Enrollment, SIGNUP_PLAN, Store, StoreError, SubscriptionKind};
use crate::vault::Vault;
use dialog_ucan_core::revocation::RevocationChecker;
use dialog_ucan_core::{Environment, VerificationContext};

/// The command path segments of [`Enroll`], as they appear in an
/// invocation. Pinned to the capability-derived ability by a test.
pub const ENROLL_COMMAND: [&str; 2] = ["customer", "enroll"];

/// The command path segments of [`Activate`].
pub const ACTIVATE_COMMAND: [&str; 2] = ["customer", "activate"];

/// The command that sends an activation link again.
pub const RESEND_COMMAND: [&str; 2] = ["customer", "resend"];

/// The largest sealed account secret an enrollment may carry.
///
/// The real envelope is 68 bytes and fixed; this is headroom for a
/// format change, not capacity.
pub const MAX_SEALED_BYTES: usize = 256;

/// The command path segments of [`Add`].
pub const PROVIDER_ADD_COMMAND: [&str; 2] = ["provider", "add"];

/// The operator commands on a subscription. Their subject is the
/// service's own DID, so only a key the service delegated to can invoke
/// one.
pub const SUSPEND_COMMAND: [&str; 4] = ["use", "put", "subscription", "suspend"];
/// See [`SUSPEND_COMMAND`].
pub const RESUME_COMMAND: [&str; 4] = ["use", "put", "subscription", "resume"];
/// See [`SUSPEND_COMMAND`].
pub const ARCHIVE_COMMAND: [&str; 4] = ["use", "put", "subscription", "archive"];

/// The command path a consent delegation must grant, or a prefix of it.
pub const CONSUMER_PROVISION_COMMAND: [&str; 2] = ["consumer", "provision"];

/// The longest activation URL this service will mint. The conservative
/// floor for what mail clients and browsers carry intact; a link past it
/// is refused at enrollment rather than emailed and found broken.
const MAX_ACTIVATION_LINK: usize = 2000;

/// The command a recovery invocation must invoke to write the custody
/// cell. The `/use` spelling, not the legacy `/memory/publish` the
/// authorizer still dispatches: a `/use/put/memory` delegation covers
/// this one and not that one, so accepting the legacy form here would
/// take an invocation no delegation can authorize.
pub const CUSTODY_PUBLISH_COMMAND: [&str; 4] = ["use", "put", "memory", "cell"];

/// The custody space and cell the sealed account secret lives at. Fixed
/// names, so an enrollment naming anything else is not custody.
const CUSTODY_SPACE: &str = "custody";
/// See [`CUSTODY_SPACE`].
const CUSTODY_SECRET_CELL: &str = "secret";

/// The verified custody material an enrollment carries: exactly what
/// was checked, and nothing else the container happened to hold.
pub struct CustodyMaterial {
    /// The sealed envelope, written into the custody cell.
    pub sealed: Vec<u8>,
    /// The pre-signed publish invocation WITH its proofs, re-encoded as
    /// a redeemable container: what the vault presents to storage.
    pub recovery: Vec<u8>,
    /// The custody space's consent to being provisioned.
    pub consent: Vec<u8>,
}

/// The multihash form the memory protocol names content by: varint code
/// `0x12` (sha2-256), varint length `0x20`, digest.
fn sha256_multihash(content: &[u8]) -> Vec<u8> {
    use sha2_0_10::{Digest, Sha256};
    let digest = Sha256::digest(content);
    let mut bytes = Vec::with_capacity(2 + digest.len());
    bytes.push(0x12);
    bytes.push(0x20);
    bytes.extend_from_slice(&digest);
    bytes
}

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
    /// `/use/put/subscription/suspend`
    Suspend,
    /// `/use/put/subscription/resume`
    Resume,
    /// `/use/put/subscription/archive`
    Archive,
    /// `/customer/resend`
    Resend,
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
        segments if segments == SUSPEND_COMMAND => Some(RegistrationCommand::Suspend),
        segments if segments == RESUME_COMMAND => Some(RegistrationCommand::Resume),
        segments if segments == ARCHIVE_COMMAND => Some(RegistrationCommand::Archive),
        segments if segments == RESEND_COMMAND => Some(RegistrationCommand::Resend),
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
    Subscription(ConsumerReceipt),
    /// The operator commands answer with nothing to say: the effect is
    /// the row, and the gate is where it shows.
    Done,
}

/// The environment a registration invocation executes against: storage,
/// email delivery, the service's signing identity, and the request's
/// origin, clock, and container.
pub struct Registration<'a, S, E, R, V> {
    /// Control-state storage.
    pub store: &'a S,
    /// Activation email delivery.
    pub email: &'a E,
    /// Where the custody cell is written.
    pub vault: &'a V,
    /// The service's signing identity, issuer of activation delegations.
    pub service: &'a Ed25519Signer,
    /// The hex seed `service` was built from, which customer spaces
    /// derive from. Held rather than re-read so the derivation has one
    /// source, and because a signer cannot give its seed back.
    pub service_seed: &'a str,
    /// Origin the activation link points at, e.g. `https://tonk.network`.
    pub origin: &'a str,
    /// Lifetime of the emailed activation delegation, in seconds.
    pub activation_ttl: u64,
    /// The current time, as a unix timestamp in seconds.
    pub now: u64,
    /// The exact container bytes of the invocation being handled.
    pub container: &'a [u8],
    /// Revocation lookup, consulted per link by the chain walk.
    ///
    /// Registration and activation are as revocable as any other
    /// invocation: a device whose delegation was revoked must not be
    /// able to enroll or activate a customer. Dialog asks this per
    /// proof, so passing a no-op checker here would silently accept a
    /// revoked chain — see [`UnverifiedRevocations`], whose `query`
    /// answers "not revoked" to everything.
    pub revocations: &'a R,
}

impl<S: Store, E: EmailSender, R: RevocationChecker + ConditionalSync, V: Vault>
    Registration<'_, S, E, R, V>
{
    /// Verify the container, decode its capability, and perform it
    /// against this environment.
    pub async fn handle(&self) -> Result<Answer, RegistrationError>
    where
        Self: Provider<Activate>,
    {
        match registration_command(self.container) {
            Some(RegistrationCommand::Enroll) => {
                let chain = self
                    .verified_chain(&ENROLL_COMMAND, Some(CEREMONY_WINDOW_SECONDS))
                    .await?;
                let effect: Enroll = deserialize_arguments(chain.arguments())?;
                // Called directly rather than through `perform`: the
                // custody material this carries is itself an invocation
                // and a delegation, and verifying them needs the same
                // revocation checker the outer chain used. A `Provider`
                // impl is `async_trait`, whose future must be `Send`,
                // and the checker's is not — so the verification lives
                // on this side of the dispatch, where the outer chain's
                // already does.
                Ok(Answer::Customer(
                    self.enroll(
                        Subject::from(chain.subject().clone())
                            .attenuate(Customer)
                            .invoke(effect),
                    )
                    .await?,
                ))
            }
            Some(RegistrationCommand::ProviderAdd) => {
                let chain = self
                    .verified_chain(&PROVIDER_ADD_COMMAND, Some(CEREMONY_WINDOW_SECONDS))
                    .await?;
                let effect: Add = deserialize_arguments(chain.arguments())?;
                // Called directly, like enrollment: the consent it
                // carries is a delegation whose revocations are this
                // service's own, and a `Provider` impl is `async_trait`
                // — whose future must be `Send`, which the checker's is
                // not.
                Ok(Answer::Subscription(
                    self.add(
                        Subject::from(chain.subject().clone())
                            .attenuate(ProviderRole)
                            .invoke(effect),
                    )
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
            // The operator commands. Their subject is this service, so
            // the chain must root here: `verify` refuses any other
            // issuer for a proofless chain on this subject, and a
            // delegated one still has to walk back to this key.
            Some(RegistrationCommand::Suspend) => {
                let chain = self.service_command(&SUSPEND_COMMAND).await?;
                let effect: SuspendSubscription = deserialize_arguments(chain.arguments())?;
                self.store
                    .suspend_subscription(
                        effect.consumer.as_str(),
                        &effect.code,
                        &effect.reason,
                        effect.until,
                    )
                    .await
                    .map_err(internal)?;
                Ok(Answer::Done)
            }
            Some(RegistrationCommand::Resume) => {
                let chain = self.service_command(&RESUME_COMMAND).await?;
                let effect: ResumeSubscription = deserialize_arguments(chain.arguments())?;
                self.store
                    .resume_subscription(effect.consumer.as_str())
                    .await
                    .map_err(internal)?;
                Ok(Answer::Done)
            }
            Some(RegistrationCommand::Archive) => {
                let chain = self.service_command(&ARCHIVE_COMMAND).await?;
                let effect: ArchiveSubscription = deserialize_arguments(chain.arguments())?;
                self.store
                    .archive_subscription(effect.consumer.as_str(), self.now)
                    .await
                    .map_err(internal)?;
                Ok(Answer::Done)
            }
            // Resend names the account as an argument rather than the
            // subject: whoever is waiting for the mail cannot sign as
            // the account they have not activated yet. So unlike the
            // operator commands above, ANY self-subjected invocation is
            // accepted — the caller signs as themselves, which the
            // ordinary verification checks, and nothing more is asked.
            // Deliberately unauthenticated beyond that: the link only
            // ever goes to the address already on the row, the answer
            // never says whether the account exists, and the send is
            // rate limited against `activation_sent_at` — so the worst
            // a caller achieves is one mail per interval to an inbox
            // they do not control.
            Some(RegistrationCommand::Resend) => {
                let chain = self
                    .verified_chain(&RESEND_COMMAND, Some(CEREMONY_WINDOW_SECONDS))
                    .await?;
                let effect: ResendActivation = deserialize_arguments(chain.arguments())?;
                self.resend(&effect.account).await?;
                Ok(Answer::Done)
            }
            None => Err(RegistrationError::Invalid {
                message: "not a registration invocation".to_string(),
            }),
        }
    }

    /// Send a customer's activation link again.
    ///
    /// Two guards, neither of them about authorization. The customer
    /// must be `Registered`, so an active account cannot be mailed at
    /// all; and the last send must be at least
    /// [`RESEND_INTERVAL_SECONDS`] ago, so pressing the button twice
    /// sends one mail. The store decides both in one statement, and
    /// answers whether this caller won — two requests arriving together
    /// cannot both conclude they were first.
    ///
    /// Refusing tells a caller whether an account exists, which is why
    /// it does not: too soon and unknown answer alike, and the person
    /// who is genuinely waiting sees the mail rather than a message.
    async fn resend(&self, account: &Did) -> Result<(), RegistrationError> {
        let not_since = self.now.saturating_sub(RESEND_INTERVAL_SECONDS);
        if !self
            .store
            .claim_activation_resend(account.as_str(), self.now, not_since)
            .await
            .map_err(internal)?
        {
            return Ok(());
        }
        let Some(customer) = self
            .store
            .customer(account.as_str())
            .await
            .map_err(internal)?
        else {
            return Ok(());
        };
        let link = self.activation_link(account).await?;
        self.email
            .send_activation(&customer.email, &link)
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("activation email failed: {err:?}"),
            })
    }

    /// Verify a command the service issues about itself.
    ///
    /// The subject is this service, so a chain that does not root here
    /// is not one of ours however well formed it is. Refusing on the
    /// subject rather than on the issuer leaves room for a delegated
    /// operator key, which still has to walk back to this one.
    async fn service_command(
        &self,
        command: &[&str],
    ) -> Result<InvocationChain<AnySignature>, RegistrationError> {
        let chain = self
            .verified_chain(command, Some(CEREMONY_WINDOW_SECONDS))
            .await?;
        let service = self.service.did();
        if chain.subject() != &service {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "an operator command acts on this service, got subject {}",
                    chain.subject()
                ),
            });
        }
        Ok(chain)
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
        let address = normalize_email(&effect.email);
        if !address.contains('@') || address.len() > 254 {
            return Err(RegistrationError::Invalid {
                message: "email must be a plausible address".to_string(),
            });
        }

        // Everything that can refuse this enrollment refuses before any
        // of it is recorded: a customer row whose activation link cannot
        // work is an account stranded exactly the way this flow exists
        // to prevent.
        // One address holds one customer: the lookup that resolves an
        // email to a `did:key` depends on it, and the column is uniquely
        // indexed. Asked here so a taken address is a refusal the client
        // can act on rather than a constraint violation surfacing as an
        // internal error.
        if let Some(holder) = self
            .store
            .customer_by_email(&address)
            .await
            .map_err(internal)?
            && holder.account != customer.to_string()
        {
            // A `Registered` holder whose activation window lapsed no
            // longer holds the address: any keypair can enroll any
            // address it does not control, and without this the mail
            // going unanswered would squat the address forever. The
            // release re-checks status and lapse in its own statement,
            // so a holder racing their own activation keeps it.
            let lapsed = holder.status == CustomerStatus::Registered
                && self
                    .store
                    .consumer(&holder.account)
                    .await
                    .map_err(internal)?
                    .and_then(|subscription| subscription.expires_at)
                    .is_some_and(|at| at <= self.now)
                && self
                    .store
                    .release_lapsed_address(&holder.account, self.now)
                    .await
                    .map_err(internal)?;
            if !lapsed {
                return Err(RegistrationError::AddressTaken);
            }
        }
        // The status refusals land before anything durable does: an
        // enrollment the customer's own state refuses must leave the
        // vault exactly as it found it.
        let existing = self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?;
        match existing.as_ref().map(|customer| customer.status) {
            Some(CustomerStatus::Active) => return Err(RegistrationError::CustomerActive),
            Some(CustomerStatus::Suspended) => return Err(RegistrationError::CustomerSuspended),
            Some(CustomerStatus::Registered) | None => {}
        }

        let material = self.verify_custody(&effect, &customer).await?;
        let space = self.ledger(&customer).await?;
        let link = self.activation_link(&customer).await?;
        // One custody space, one account, forever. The custody DID is
        // PRF-derived from the passkey, and the sealed secret in its
        // cell IS the account — the same passkey always reopens the same
        // account, so a claim from a DIFFERENT account is never
        // legitimate, lapsed or not, and its cell could not be replaced
        // anyway (the write below is create-only). Refused here, before
        // anything durable, with an answer that says what to do instead.
        let custody_subscription = self
            .store
            .consumer(effect.custody.as_str())
            .await
            .map_err(internal)?;
        let custody_enrolled = custody_subscription
            .as_ref()
            .is_some_and(|subscription| subscription.provider == customer.to_string());
        if custody_subscription.is_some() && !custody_enrolled {
            return Err(RegistrationError::Forbidden {
                message: "this passkey's custody space is enrolled to another account; \
                          sign in with the passkey instead of creating a new account"
                    .to_string(),
            });
        }

        // The cell goes in before the customer row. Nothing serves it
        // yet — the customer is `Registered`, and the gate refuses
        // everything behind a provider in that state — so this writes
        // into a space that answers nothing until the emailed link is
        // clicked. Doing it here rather than queueing it is what stops a
        // signup finishing with an account no second device can open.
        //
        // Written ONCE per custody space. The write is create-only — a
        // custody cell must never be replaced under a passkey that
        // could already have opened it — so a `Registered` re-enrollment
        // whose custody this account already subscribed (the resend
        // path) skips the publish instead of failing it. A custody this
        // account has not enrolled before (a re-created passkey derives
        // a fresh custody DID) still gets its cell.
        if !custody_enrolled {
            self.vault
                .publish(&material.recovery, &material.sealed)
                .await
                .map_err(|error| RegistrationError::Internal {
                    message: error.to_string(),
                })?;
        }

        match existing {
            None => {
                // The subscriptions expire when the activation link
                // does, so a signup nobody finishes clears itself
                // rather than leaving rows behind.
                self.store
                    .enroll_customer(Enrollment {
                        did: customer.as_str(),
                        email: &address,
                        plan: SIGNUP_PLAN,
                        ledger: space.did.as_str(),
                        custody: effect.custody.as_str(),
                        now: self.now,
                        expires_at: self.now + self.activation_ttl,
                    })
                    .await
                    .map_err(internal)?;
            }
            Some(existing) => {
                if existing.email != address {
                    self.store
                        .update_registered_email(customer.as_str(), &address)
                        .await
                        .map_err(internal)?;
                }
                // The cell published above needs its subscription row,
                // or the gate never serves it: a `Registered` account
                // re-enrolling with a passkey it re-created brings a
                // custody space the first enrollment never named.
                if !custody_enrolled {
                    self.store
                        .add_subscription(
                            effect.custody.as_str(),
                            customer.as_str(),
                            self.now,
                            SubscriptionKind::Custody,
                        )
                        .await
                        .map_err(internal)?;
                }
            }
        }

        // Recorded as a send like any other, so resending is rate
        // limited against this one rather than treating enrollment as
        // if no mail had gone out — and limited BY the last one: an
        // enrollment repeated inside the interval keeps its rows and
        // sends nothing, so a loop of re-enrollments cannot pump mail
        // at an address its owner never confirmed.
        if self
            .store
            .claim_activation_resend(
                customer.as_str(),
                self.now,
                self.now.saturating_sub(RESEND_INTERVAL_SECONDS),
            )
            .await
            .map_err(internal)?
        {
            self.email
                .send_activation(&address, &link)
                .await
                .map_err(|err| RegistrationError::Internal {
                    message: format!("activation email failed: {err:?}"),
                })?;
        }

        Ok(Receipt {
            customer,
            status: CustomerStatus::Registered,
            // Enrollment names the provider, even though nothing is
            // served yet. It is WHERE this account syncs, which is known
            // now and does not change at activation — so a client can
            // attach its remote immediately and let the gate answer.
            //
            // That is what makes activation observable without asking:
            // the remote answers 403 while the customer is unconfirmed
            // and 200 once the emailed link is opened, so a device
            // learns it was activated from the sync it was already
            // doing. Withholding the address here forced the opposite —
            // a client with nowhere to sync had to poll a status
            // endpoint to discover it could start.
            provider: Some(self.provider_address()),
            // The space, though, is named now: it is derived rather than
            // allocated, so it exists as soon as the account does, and a
            // client that records it here needs nothing from activation
            // to know where its own record will live.
            ledger: Some(space),
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
            let ledger = Some(self.ledger(&customer).await?);
            return Ok(Receipt {
                customer,
                status: CustomerStatus::Active,
                // Activation names no provider: enrollment already did,
                // and the address does not change. A client is syncing to
                // it by now, which is how it learns it was activated —
                // the gate stops answering 403.
                provider: None,
                ledger,
            });
        }
        match self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?
        {
            Some(existing) if existing.status == CustomerStatus::Active => {
                let ledger = Some(self.ledger(&customer).await?);
                Ok(Receipt {
                    customer,
                    status: CustomerStatus::Active,
                    // Already active: same reasoning as above.
                    provider: None,
                    ledger,
                })
            }
            Some(_) => Err(RegistrationError::CustomerSuspended),
            None => Err(RegistrationError::UnknownCustomer),
        }
    }

    /// Execute a verified `/provider/add`: validate the deposited consent
    /// and provision the consumer under the invoking customer.
    /// Re-provisioning under the same customer is a no-op success; a
    /// consumer another customer provides is refused. The customer must
    /// be `Active`: an unactivated customer provisions nothing, so the
    /// same email confirmation gates adding and serving alike.
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
            Some(customer) => match customer.status {
                // Provisioning is a fact about the SPACE; activation is a
                // fact about the customer. An unconfirmed customer may
                // still claim its namespaces — the gate refuses to serve
                // them either way, and refuses with `Retry` because
                // confirming the email is what clears it.
                //
                // Refusing here instead conflated the two: a second device
                // signing in could not provision the custody space its
                // passkey needs, so the gate answered "not provisioned"
                // with no recourse — a dead end that reads as a broken
                // login rather than as an email waiting to be confirmed.
                CustomerStatus::Active | CustomerStatus::Registered => {}
                CustomerStatus::Suspended => return Err(RegistrationError::CustomerSuspended),
            },
        }
        let consent = self.carried_delegation(&effect.consent.to_string())?;
        self.verify_consent(&consent, &effect.consumer, &provider)
            .await?;
        let kind = match effect.kind.as_deref() {
            None | Some("space") => crate::store::SubscriptionKind::Space,
            Some("custody") => crate::store::SubscriptionKind::Custody,
            Some(other) => {
                return Err(RegistrationError::Forbidden {
                    message: format!("unknown consumer kind: {other}"),
                });
            }
        };
        if !self
            .store
            .add_subscription(effect.consumer.as_str(), provider.as_str(), self.now, kind)
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
            })?;
        // A consent whose authority was withdrawn is not consent. The
        // issuer here IS the subject — the checks above refuse anything
        // else — so one revoker covers both who granted it and what it
        // was granted over.
        let revokers = [consent.issuer().clone()];
        let revoked = self
            .revocations
            .query(dialog_ucan_core::revocation::RevocationSelector::new(
                consent.to_cid(),
                &revokers,
            ))
            .await
            .map_err(|error| RegistrationError::Unauthorized {
                message: format!("consent revocations could not be checked: {error}"),
            })?;
        match revoked {
            None => Ok(()),
            Some(found) => Err(RegistrationError::Unauthorized {
                message: format!("consent was revoked by {}", found.principal),
            }),
        }
    }

    /// This service's own address as a provider — the UCAN endpoint its
    /// customers' spaces attach their remotes to.
    ///
    /// Derived from the origin the service is configured with, not from
    /// the origin a request arrived on: the service decides which
    /// provider serves its customers, and answers every receipt with it,
    /// so a client records one authoritative address rather than
    /// deriving its own.
    fn provider_address(&self) -> String {
        format!("{}/ucan/", self.origin.trim_end_matches('/'))
    }

    /// Check the custody material an enrollment carries, without
    /// performing any of it.
    ///
    /// Enrollment is the only point at which this is examined —
    /// activation replays what was verified here rather than checking it
    /// again — so a refusal that belongs anywhere belongs here, while
    /// the person is still watching. Anything accepted becomes an
    /// account that cannot be opened on a second device.
    ///
    /// Returns the bytes activation will need, so the caller carries
    /// exactly the blocks it verified and nothing else.
    async fn verify_custody(
        &self,
        effect: &Enroll,
        account: &Did,
    ) -> Result<CustodyMaterial, RegistrationError> {
        // Read the enrollment as a bundle rather than a chain: it carries
        // an invocation and opaque ciphertext beside its proofs, which an
        // invocation chain refuses by design.
        let bundle = InvocationBundle::try_from(self.container).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("bad enrollment container: {err}"),
            }
        })?;
        let carried = |cid: &Cid, field: &str| {
            bundle
                .block(cid)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| RegistrationError::Invalid {
                    message: format!("`{field}` names {cid}, which the container does not carry"),
                })
        };
        let sealed = carried(&effect.sealed, "sealed")?;
        // A sealed account secret is 68 bytes: an 8-byte header, a
        // 12-byte nonce, and 48 bytes of ciphertext over a 32-byte
        // secret. The bound leaves room for a version bump or a wider
        // nonce and nothing else — the cell is written before the
        // customer is served, so a generous limit here would be a
        // write-anything store that costs nothing to use.
        if sealed.len() > MAX_SEALED_BYTES {
            return Err(RegistrationError::Invalid {
                message: format!(
                    "`sealed` is {} bytes, over the {MAX_SEALED_BYTES}-byte limit",
                    sealed.len()
                ),
            });
        }
        // Resolved for the refusal it produces: `recovery` must name a
        // block the container carries, whether or not the bytes are
        // used beyond the resolution below.
        carried(&effect.recovery, "recovery")?;
        let consent_bytes = carried(&effect.consent, "consent")?;

        // The recovery invocation, with its proofs resolved from the
        // same container. `resolve_invocation` hands back a chain with
        // an empty proof store — a carried block is a bare token — so a
        // recovery issued through a delegate resolves its own chain
        // from the blocks travelling beside it.
        let recovery = bundle.resolve_invocation(&effect.recovery).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("`recovery` does not decode as an invocation: {err}"),
            }
        })?;
        let recovery = InvocationChain::new(
            recovery.invocation.clone(),
            self.carried_proofs(&bundle, recovery.invocation.proofs())?,
        );
        // Proofs are allowed: a passkey may delegate to a profile that
        // issues the recovery setup, so the chain is verified like any
        // other rather than required to be self-signed. It verifies
        // against this service's own revocations — a nested invocation
        // is an invocation, and a link revoked anywhere above it refuses
        // the enrollment.
        recovery
            .verify(&VerificationContext::new(&Environment::new(
                recovery.proof_store(),
                DidKeyResolver,
                self.revocations,
            )))
            .await
            .map_err(|err| RegistrationError::Unauthorized {
                message: format!("`recovery` failed to verify: {err}"),
            })?;
        if recovery.subject() != &effect.custody {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "`recovery` acts on {}, not the custody space {}",
                    recovery.subject(),
                    effect.custody
                ),
            });
        }
        let command: Vec<&str> = recovery.command().0.iter().map(String::as_str).collect();
        if command.as_slice() != CUSTODY_PUBLISH_COMMAND {
            return Err(RegistrationError::Forbidden {
                message: format!(
                    "`recovery` invokes /{}, not /{}",
                    command.join("/"),
                    CUSTODY_PUBLISH_COMMAND.join("/")
                ),
            });
        }
        let arguments = recovery.invocation.arguments();
        for (name, expected) in [("space", CUSTODY_SPACE), ("cell", CUSTODY_SECRET_CELL)] {
            match arguments.get(name) {
                Some(Promised::String(value)) if value == expected => {}
                other => {
                    return Err(RegistrationError::Forbidden {
                        message: format!("`recovery` names {name} {other:?}, not the custody cell"),
                    });
                }
            }
        }
        // An overwrite could destroy an envelope the passkey has since
        // rotated, so the queued write must be a first write.
        if arguments.contains_key("when") {
            return Err(RegistrationError::Forbidden {
                message:
                    "`recovery` carries `when`, so it would overwrite the custody cell rather \
                          than write it once"
                        .to_string(),
            });
        }
        // The invocation names its content by checksum; the bytes travel
        // beside it. Binding the two here is what stops a mismatched
        // envelope being written at activation.
        match arguments.get("checksum") {
            Some(Promised::Bytes(checksum)) if checksum.as_slice() == sha256_multihash(&sealed) => {
            }
            _ => {
                return Err(RegistrationError::Forbidden {
                    message: "`recovery` checksums content other than the carried `sealed` block"
                        .to_string(),
                });
            }
        }
        // The one check that cannot wait: everything else verified here
        // stays true until activation, but an invocation that lapses
        // first cannot be re-minted, and the account is stranded exactly
        // as it would have been without any of this.
        let expiration =
            recovery
                .invocation
                .expiration()
                .ok_or_else(|| RegistrationError::Unauthorized {
                    message: "`recovery` must carry an expiration".to_string(),
                })?;
        let expires_at = expiration.to_unix();
        if expires_at < self.now + self.activation_ttl {
            return Err(RegistrationError::Unauthorized {
                message: format!(
                    "`recovery` expires_at at {expires_at}, before the activation link it would be \
                     carried by"
                ),
            });
        }

        // The consent: the same shape `/provider/add` deposits, checked
        // now so activation can provision without asking again.
        let consent = bundle.resolve_delegation(&effect.consent).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("`consent` does not decode as a delegation: {err}"),
            }
        })?;
        let head = consent
            .proofs()
            .next()
            .ok_or_else(|| RegistrationError::Invalid {
                message: "`consent` carries no delegation".to_string(),
            })?;
        self.verify_consent(head, &effect.custody, account).await?;

        // The recovery travels onward as the CHAIN, not the bare token:
        // a delegate-issued recovery names proofs, and the vault redeems
        // whatever it is handed as a container of its own. Handing it
        // the token alone dropped those proofs, so exactly the chains
        // this verification deliberately admits were then refused at the
        // write.
        let recovery_container =
            recovery
                .to_bytes()
                .map_err(|error| RegistrationError::Internal {
                    message: format!("the verified recovery did not re-encode: {error}"),
                })?;

        Ok(CustodyMaterial {
            sealed,
            recovery: recovery_container,
            consent: consent_bytes,
        })
    }

    /// The ledger space for `account`, and the account's authority to
    /// read it.
    ///
    /// The space is derived from the service seed, so nothing is stored
    /// and the same account always resolves to the same DID. The
    /// delegation grants `/use/get` — every read, and only reads. The
    /// service writes the customer's metering and billing there; the
    /// account can see its own record but never rewrite it, and cannot
    /// withdraw the service's own access the way a client-granted
    /// delegation could be withdrawn.
    async fn ledger(&self, account: &Did) -> Result<Ledger, RegistrationError> {
        let space = crate::service::ledger_signer(self.service_seed, account)
            .map_err(|message| RegistrationError::Internal { message })?;
        let did = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(account)
            .subject(DelegatedSubject::Specific(did.clone()))
            .command(vec!["use".to_string(), "get".to_string()])
            .try_build()
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("minting the customer space read failed: {err}"),
            })?;
        let bytes = DelegationChain::new(delegation).to_bytes().map_err(|err| {
            RegistrationError::Internal {
                message: format!("encoding the customer space read failed: {err}"),
            }
        })?;
        Ok(Ledger {
            did,
            read_hex: hex::encode(bytes),
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
        // The invocation alone. It used to carry the custody blocks,
        // because activation performed the cell write; enrollment does
        // that now, so the link needs nothing but the customer it names
        // — which is the whole reason resending one is cheap.
        let link = format!(
            "{}/activate?ucan={}",
            self.origin.trim_end_matches('/'),
            URL_SAFE_NO_PAD.encode(bytes)
        );
        // Checked on the finished URL rather than the material, because
        // the origin and the invocation's own fields count too. A link
        // that does not survive a mail client is an account nobody can
        // activate, so this refuses the enrollment instead — which is
        // why the caller mints before it writes anything.
        if link.len() > MAX_ACTIVATION_LINK {
            return Err(RegistrationError::Invalid {
                message: format!(
                    "the activation link is {} characters, past the {MAX_ACTIVATION_LINK} a mail \
                     client and browser can be relied on to carry",
                    link.len()
                ),
            });
        }
        Ok(link)
    }

    /// Parse and cryptographically verify the container, require the
    /// exact command, and require an expiration that has not passed and,
    /// when a window is given, does not sit further ahead than it.
    async fn verified_chain(
        &self,
        expected_command: &[&str],
        window: Option<u64>,
    ) -> Result<InvocationChain<AnySignature>, RegistrationError> {
        // Read as a bundle, not a chain: an enrollment carries the
        // recovery invocation and the sealed envelope beside its proofs,
        // and a chain refuses any token that is not a delegation. The
        // root still authorizes the ordinary way — `chain()` is the same
        // invocation with the proofs it names, and nothing else.
        let chain = InvocationBundle::try_from(self.container)
            .and_then(|bundle| bundle.chain())
            .map_err(|err| RegistrationError::Invalid {
                message: format!("bad invocation container: {err}"),
            })?;
        chain
            .verify(&VerificationContext::new(&Environment::new(
                chain.proof_store(),
                DidKeyResolver,
                self.revocations,
            )))
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

    /// The delegation tokens the container carries, invocation aside.
    ///
    /// A token that is not a delegation is skipped rather than refused:
    /// an enrollment also carries the recovery invocation and the sealed
    /// envelope, which are arguments rather than proofs. What must be
    /// present is checked where it is named — a deposit the `access`
    /// argument names and the container does not carry is an error at
    /// that point, which is where the reader can see which one it was.
    fn delegation_tokens(&self) -> Result<Vec<Delegation<AnySignature>>, RegistrationError> {
        Ok(Container::from_bytes(self.container)
            .map_err(|err| RegistrationError::Invalid {
                message: format!("bad container: {err}"),
            })?
            .into_tokens()
            .into_iter()
            .skip(1)
            .filter_map(|bytes| {
                serde_ipld_dagcbor::from_slice::<Delegation<AnySignature>>(&bytes).ok()
            })
            .collect())
    }

    /// Resolve an invocation's proofs from the blocks its container
    /// carries, following each delegation's own proofs in turn.
    ///
    /// A nested invocation arrives as a bare token, so nothing has
    /// resolved its chain: this walks it, and a link the container does
    /// not carry is a refusal rather than a chain that verifies short.
    fn carried_proofs(
        &self,
        bundle: &InvocationBundle,
        proofs: &[Cid],
    ) -> Result<
        std::collections::HashMap<Cid, std::sync::Arc<Delegation<AnySignature>>>,
        RegistrationError,
    > {
        let mut resolved = std::collections::HashMap::new();
        for link in proofs {
            let chain =
                bundle
                    .resolve_delegation(link)
                    .map_err(|err| RegistrationError::Invalid {
                        message: format!(
                            "`recovery` names proof {link}, which does not resolve: {err}"
                        ),
                    })?;
            resolved.extend(chain.export());
        }
        Ok(resolved)
    }

    /// Find a delegation the container carries, by CID.
    fn carried_delegation(&self, cid: &str) -> Result<Delegation<AnySignature>, RegistrationError> {
        self.delegation_tokens()?
            .into_iter()
            .find(|delegation| delegation.to_cid().to_string() == cid)
            .ok_or_else(|| RegistrationError::Invalid {
                message: format!("the argument names {cid}, which the container does not carry"),
            })
    }

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
impl<S, E, R, V> Provider<Activate> for Registration<'_, S, E, R, V>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
    R: RevocationChecker + ConditionalSync,
    V: Vault + ConditionalSync,
{
    async fn execute(&self, input: Capability<Activate>) -> Result<Receipt, RegistrationError> {
        self.activate(input).await
    }
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
            custody: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                .parse()
                .unwrap(),
            recovery: Cid::default(),
            consent: Cid::default(),
            sealed: Cid::default(),
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
