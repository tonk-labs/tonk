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
    Activate, Add, ConsumerReceipt, Customer, CustomerSpace, CustomerStatus, Enroll,
    Provider as ProviderRole, Receipt, RegistrationError, deposit_scopes,
};

use crate::email::{EmailSender, normalize_email};
use crate::store::{SIGNUP_PLAN, Store, StoreError};
use dialog_ucan_core::revocation::RevocationChecker;
use dialog_ucan_core::{Environment, VerificationContext};

/// The command path segments of [`Enroll`], as they appear in an
/// invocation. Pinned to the capability-derived ability by a test.
pub const ENROLL_COMMAND: [&str; 2] = ["customer", "enroll"];

/// The command path segments of [`Activate`].
pub const ACTIVATE_COMMAND: [&str; 2] = ["customer", "activate"];

/// The command path segments of [`Add`].
pub const PROVIDER_ADD_COMMAND: [&str; 2] = ["provider", "add"];

/// The command path a consent delegation must grant, or a prefix of it.
pub const CONSUMER_PROVISION_COMMAND: [&str; 2] = ["consumer", "provision"];

/// The longest activation URL this service will mint. The conservative
/// floor for what mail clients and browsers carry intact; a link past it
/// is refused at enrollment rather than emailed and found broken.
const MAX_ACTIVATION_LINK: usize = 2000;

/// The command a recovery invocation must invoke to write the custody
/// cell.
const CUSTODY_PUBLISH_COMMAND: [&str; 4] = ["use", "put", "memory", "cell"];

/// The custody space and cell the sealed account secret lives at. Fixed
/// names, so an enrollment naming anything else is not custody.
const CUSTODY_SPACE: &str = "custody";
/// See [`CUSTODY_SPACE`].
const CUSTODY_SECRET_CELL: &str = "secret";

/// The verified blocks an activation link carries: exactly what was
/// checked at enrollment, and nothing else the container happened to
/// hold.
pub struct CustodyMaterial {
    /// The sealed envelope, written into the custody cell.
    pub sealed: Vec<u8>,
    /// The pre-signed publish invocation that writes it.
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
pub struct Registration<'a, S, E, R> {
    /// Control-state storage.
    pub store: &'a S,
    /// Activation email delivery.
    pub email: &'a E,
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

impl<S: Store, E: EmailSender, R: RevocationChecker + ConditionalSync> Registration<'_, S, E, R> {
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
        self.verify_deposits(&effect.access, &customer).await?;
        let material = self.verify_custody(&effect, &customer).await?;
        let space = self.customer_space(&customer).await?;
        let link = self.activation_link(&customer, &material).await?;
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

        self.email
            .send_activation(&address, &link)
            .await
            .map_err(|err| RegistrationError::Internal {
                message: format!("activation email failed: {err:?}"),
            })?;

        Ok(Receipt {
            customer,
            status: CustomerStatus::Registered,
            // Enrollment names no provider. The address is what says
            // "this service serves you", and it does not yet: an
            // unactivated customer gets neither service nor
            // provisioning. Naming it here would let a client record an
            // endpoint it cannot use, and erase the difference between
            // "enrolled, email unconfirmed" and "ready to sync" — which
            // is exactly the distinction the share flow needs in order
            // to say "check your email" rather than "turn on sync".
            // Activation is where it lands.
            provider: None,
            // The space, though, is named now: it is derived rather than
            // allocated, so it exists as soon as the account does, and a
            // client that records it here needs nothing from activation
            // to know where its own record will live.
            customer_space: Some(space),
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
            let customer_space = Some(self.customer_space(&customer).await?);
            return Ok(Receipt {
                customer,
                status: CustomerStatus::Active,
                provider: Some(self.provider_address()),
                customer_space,
            });
        }
        match self
            .store
            .customer(customer.as_str())
            .await
            .map_err(internal)?
        {
            Some(existing) if existing.status == CustomerStatus::Active => {
                let customer_space = Some(self.customer_space(&customer).await?);
                Ok(Receipt {
                    customer,
                    status: CustomerStatus::Active,
                    provider: Some(self.provider_address()),
                    customer_space,
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
                CustomerStatus::Active => {}
                // An unactivated customer gets nothing: not service, and
                // not provisioning either. The client holds the add as
                // pending work and replays it once the email is
                // confirmed; re-enrolling resends that email.
                CustomerStatus::Registered => return Err(RegistrationError::CustomerInactive),
                CustomerStatus::Suspended => return Err(RegistrationError::CustomerSuspended),
            },
        }
        let consent = self.deposited_delegation(&effect.consent.to_string())?;
        self.verify_consent(&consent.delegation, &effect.consumer, &provider)
            .await?;
        let kind = match effect.kind.as_deref() {
            None | Some("space") => crate::store::ConsumerKind::Space,
            Some("custody") => crate::store::ConsumerKind::Custody,
            Some(other) => {
                return Err(RegistrationError::Forbidden {
                    message: format!("unknown consumer kind: {other}"),
                });
            }
        };
        if !self
            .store
            .add_consumer(effect.consumer.as_str(), provider.as_str(), self.now, kind)
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
        let recovery_bytes = carried(&effect.recovery, "recovery")?;
        let consent_bytes = carried(&effect.consent, "consent")?;

        // The recovery invocation: self-signed by the custody key, so it
        // verifies against that key alone and any browser can redeem it.
        let recovery = bundle.resolve_invocation(&effect.recovery).map_err(|err| {
            RegistrationError::Invalid {
                message: format!("`recovery` does not decode as an invocation: {err}"),
            }
        })?;
        // Proofless by construction — issuer, audience and subject are
        // all the custody key — so this must carry no delegations, and
        // there is nothing for a revocation query to ask about. Checked
        // rather than assumed: a chain that did carry proofs would be
        // verified here without its revocations being consulted.
        if !recovery.invocation.proofs().is_empty() {
            return Err(RegistrationError::Forbidden {
                message: "`recovery` must be self-signed by the custody key and carry no proofs"
                    .to_string(),
            });
        }
        recovery
            .verify(&VerificationContext::new(&Environment::new(
                recovery.proof_store(),
                DidKeyResolver,
                &dialog_ucan_core::revocation::UnverifiedRevocations,
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
                    "`recovery` expires at {expires_at}, before the activation link it would be \
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

        Ok(CustodyMaterial {
            sealed,
            recovery: recovery_bytes,
            consent: consent_bytes,
        })
    }

    /// The bookkeeping space for `account`, and the account's authority
    /// to read it.
    ///
    /// The space is derived from the service seed, so nothing is stored
    /// and the same account always resolves to the same DID. The
    /// delegation grants `/use/get` — every read, and only reads. The
    /// service writes the customer's metering and billing there; the
    /// account can see its own record but never rewrite it, and cannot
    /// withdraw the service's own access the way a client-granted
    /// delegation could be withdrawn.
    async fn customer_space(&self, account: &Did) -> Result<CustomerSpace, RegistrationError> {
        let space = crate::service::customer_space_signer(self.service_seed, account)
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
        Ok(CustomerSpace {
            did,
            read_hex: hex::encode(bytes),
        })
    }

    /// Mint the activation invocation and wrap it into a link. The
    /// invocation is complete and service-signed: the accept button
    /// presents it as-is, so activation needs no key on the presenting
    /// device and a click on any device finalizes.
    pub async fn activation_link(
        &self,
        customer: &Did,
        material: &CustodyMaterial,
    ) -> Result<String, RegistrationError> {
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
        // Exactly the blocks enrollment verified, and nothing else the
        // enrolling container happened to carry: the link is the
        // tightest budget in this flow, and unnamed material riding
        // along is weight nobody checked.
        let mut tokens = Container::from_bytes(&bytes)
            .map_err(|err| RegistrationError::Internal {
                message: format!("reopening the activation container failed: {err}"),
            })?
            .into_tokens();
        tokens.extend([
            material.recovery.clone(),
            material.consent.clone(),
            material.sealed.clone(),
        ]);
        let bytes =
            Container::new(tokens)
                .to_bytes()
                .map_err(|err| RegistrationError::Internal {
                    message: format!("encoding the activation container failed: {err}"),
                })?;
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
    fn delegation_tokens(&self) -> Result<Vec<DelegationToken>, RegistrationError> {
        Ok(Container::from_bytes(self.container)
            .map_err(|err| RegistrationError::Invalid {
                message: format!("bad container: {err}"),
            })?
            .into_tokens()
            .into_iter()
            .skip(1)
            .filter_map(|bytes| {
                serde_ipld_dagcbor::from_slice::<Delegation<AnySignature>>(&bytes)
                    .ok()
                    .map(|delegation| DelegationToken { delegation, bytes })
            })
            .collect())
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
impl<S, E, R> Provider<Enroll> for Registration<'_, S, E, R>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
    R: RevocationChecker + ConditionalSync,
{
    async fn execute(&self, input: Capability<Enroll>) -> Result<Receipt, RegistrationError> {
        self.enroll(input).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S, E, R> Provider<Activate> for Registration<'_, S, E, R>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
    R: RevocationChecker + ConditionalSync,
{
    async fn execute(&self, input: Capability<Activate>) -> Result<Receipt, RegistrationError> {
        self.activate(input).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S, E, R> Provider<Add> for Registration<'_, S, E, R>
where
    S: Store + ConditionalSync,
    E: EmailSender + ConditionalSync,
    R: RevocationChecker + ConditionalSync,
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
