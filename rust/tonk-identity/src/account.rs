//! An account and the envelope that holds it.
//!
//! The envelope is what a custody cell publishes; the secret is what
//! signs and seals while this value lives. Pairing them means a caller
//! cannot publish an envelope for one account and sign as another.
//!
//! Native as well as web: a passkey is one way to reach an account, not
//! what an account is. The CLI and the worker hold accounts the same
//! way, differing only in which custodian opened the envelope.

use crate::clearance::Recovery;
use crate::custodian::Custodian;
use crate::envelope::capability::{Opening, Sealing};
use crate::envelope::{AccountSecret, Envelope, Kek, KekMethod};
use crate::sealed::AccountSecretKey;

/// An account and the envelope that holds it, kept together.
pub struct Account {
    secret: AccountSecret,
    envelope: Envelope<Recovery>,
}

impl Account {
    /// Adopt a secret and the envelope that holds it.
    ///
    /// Both halves at once, on purpose: an account whose envelope
    /// belongs to different bytes is the failure this type exists to
    /// make unrepresentable.
    pub fn new(secret: AccountSecret, envelope: Envelope<Recovery>) -> Self {
        Self { secret, envelope }
    }

    /// The account's signer, as dialog's algorithm-agnostic `Signer`.
    ///
    /// Enough for everything downstream: signing, and the DID through
    /// `Principal`.
    pub async fn signer(&self) -> anyhow::Result<dialog_credentials::Signer> {
        Ok(dialog_credentials::Signer::from(
            self.secret.signer().await?,
        ))
    }

    /// The sealed form, for the custody cell.
    pub fn envelope(&self) -> &Envelope<Recovery> {
        &self.envelope
    }

    /// Give up the secret, to seal it under another custodian.
    ///
    /// Consuming, and the only way out: an account whose secret can be
    /// borrowed is one a caller can copy. Adding a passkey is the sole
    /// reason it exists — the same secret is sealed again so either
    /// credential opens the account.
    pub fn into_secret(self) -> AccountSecret {
        self.secret
    }

    /// Seal to and open for this account.
    pub fn secret(&self) -> AccountSecretKey<'_> {
        self.secret.secret()
    }
}

/// Every way in is a command carrying the custodian it belongs to, so
/// the passkey that seals and the passkey that opens are named in the
/// same value. Performed rather than called, because two of the four
/// need something the caller has to supply: `load` reaches the network,
/// and all of them need somewhere WebCrypto lives.
pub struct AccountBuilder<'a>(pub(crate) &'a Custodian);

impl AccountBuilder<'_> {
    /// Generate an account and seal it under this passkey.
    ///
    /// The secret exists for the length of the command. Where that is —
    /// page or worker — is where the handles were sent, which is the
    /// point of sending them: nothing but the envelope survives, and
    /// only this passkey opens it.
    pub fn create(self) -> CreateAccount {
        CreateAccount(self.0.clone())
    }

    /// Seal an account that already exists under this passkey. Used
    /// when a second passkey is enrolled: same account, another way in.
    pub fn adopt(self, secret: AccountSecret) -> AdoptAccount {
        AdoptAccount(self.0.clone(), secret)
    }

    /// Open an envelope sealed under this passkey.
    ///
    /// Takes the envelope rather than fetching it — where it comes from
    /// is the caller's business, and it arrives from the custody cell or
    /// from a profile row depending on who is asking.
    pub fn import(self, envelope: Envelope<Recovery>) -> ImportAccount {
        ImportAccount(self.0.clone(), envelope)
    }

    /// Fetch this passkey's custody cell and open what is there.
    ///
    /// The whole recovery in one command: derive the signer that names
    /// the space, read the cell, unseal it. A device holding nothing but
    /// the passkey gets the account back.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn load(self, endpoint: impl Into<String>) -> LoadAccount {
        LoadAccount(self.0.clone(), endpoint.into())
    }
}

/// Generate an account under a custodian. See [`AccountBuilder::create`].
pub struct CreateAccount(Custodian);

/// Seal an existing account under a custodian. See
/// [`AccountBuilder::adopt`].
pub struct AdoptAccount(Custodian, AccountSecret);

/// Open an envelope under a custodian. See [`AccountBuilder::import`].
pub struct ImportAccount(Custodian, Envelope<Recovery>);

/// Fetch and open a custodian's cell. See [`AccountBuilder::load`].
///
/// Only where the fetch exists: every other account command is pure
/// crypto and runs anywhere, but this one reaches the network.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub struct LoadAccount(Custodian, String);

impl dialog_capability::Command for CreateAccount {
    type Input = Self;
    type Output = anyhow::Result<Account>;
}

impl dialog_capability::Command for AdoptAccount {
    type Input = Self;
    type Output = anyhow::Result<Account>;
}

impl dialog_capability::Command for ImportAccount {
    type Input = Self;
    type Output = anyhow::Result<Account>;
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl dialog_capability::Command for LoadAccount {
    /// `None` when the space holds no cell yet, which is not a failure:
    /// it is what a passkey enrolled but never published looks like, and
    /// the caller decides whether that is a problem.
    type Input = Self;
    type Output = anyhow::Result<Option<Account>>;
}

impl CreateAccount {
    /// Run this against a provider that can reach WebCrypto.
    pub async fn perform<Env>(self, env: &Env) -> anyhow::Result<Account>
    where
        Env: dialog_capability::Provider<Self>,
    {
        env.execute(self).await
    }
}

impl AdoptAccount {
    /// Run this against a provider that can reach WebCrypto.
    pub async fn perform<Env>(self, env: &Env) -> anyhow::Result<Account>
    where
        Env: dialog_capability::Provider<Self>,
    {
        env.execute(self).await
    }
}

impl ImportAccount {
    /// Run this against a provider that can reach WebCrypto.
    pub async fn perform<Env>(self, env: &Env) -> anyhow::Result<Account>
    where
        Env: dialog_capability::Provider<Self>,
    {
        env.execute(self).await
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl LoadAccount {
    /// Run this against a provider that can reach the access service.
    pub async fn perform<Env>(self, env: &Env) -> anyhow::Result<Option<Account>>
    where
        Env: dialog_capability::Provider<Self>,
    {
        env.execute(self).await
    }
}

/// Runs the account commands.
///
/// Sealing, opening and generating are the custodian's own work — a
/// passkey reaches WebCrypto, a native keypair does not — so `create`,
/// `adopt` and `import` are provided on every target. [`LoadAccount`]
/// additionally fetches the custody cell, which is why it is provided
/// only where that fetch exists.
pub struct Crypto;

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<CreateAccount> for Crypto {
    async fn execute(&self, input: CreateAccount) -> anyhow::Result<Account> {
        seal_under(&input.0, AccountSecret::generate()?).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<AdoptAccount> for Crypto {
    async fn execute(&self, input: AdoptAccount) -> anyhow::Result<Account> {
        seal_under(&input.0, input.1).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<ImportAccount> for Crypto {
    async fn execute(&self, input: ImportAccount) -> anyhow::Result<Account> {
        open_under(&input.0, input.1).await
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<LoadAccount> for Crypto {
    async fn execute(&self, input: LoadAccount) -> anyhow::Result<Option<Account>> {
        let signer = input.0.signer().await?;
        let Some(bytes) = crate::custody::resolve_secret(signer, &input.1).await? else {
            return Ok(None);
        };
        let envelope = Envelope::decode(&bytes)
            .map_err(|error| anyhow::anyhow!("the custody cell is unreadable: {error}"))?;
        open_under(&input.0, envelope).await.map(Some)
    }
}

/// Seal a secret under a custodian, which is what creating and adopting
/// both come down to.
async fn seal_under(custodian: &Custodian, secret: AccountSecret) -> anyhow::Result<Account> {
    let sealer = custodian
        .sealer()
        .await
        .map_err(|error| anyhow::anyhow!("deriving the sealer failed: {error:?}"))?;
    let envelope = seal(&sealer, &secret.material(), KekMethod::Passkey).await?;
    Ok(Account::new(secret, envelope))
}

/// Open an envelope under a custodian.
async fn open_under(
    custodian: &Custodian,
    envelope: Envelope<Recovery>,
) -> anyhow::Result<Account> {
    let opener = custodian
        .opener()
        .await
        .map_err(|error| anyhow::anyhow!("deriving the opener failed: {error:?}"))?;
    let seed = open(&opener, &envelope).await?;
    Ok(Account::new(AccountSecret::from_bytes(seed), envelope))
}

/// Seal a seed under a KEK, whichever way its key is held.
///
/// On the web a handle-backed KEK goes through WebCrypto, so no raw key
/// is materialised; everywhere else, and for a bytes-backed KEK, the
/// `aes_gcm` path produces the identical wire format.
async fn seal(
    kek: &Kek<Recovery, Sealing>,
    seed: &zeroize::Zeroizing<[u8; 32]>,
    method: KekMethod,
) -> anyhow::Result<Envelope<Recovery>> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        crate::webcrypto_kek::seal_seed(kek, seed, method).await
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        kek.seal_seed(seed, method)
    }
}

/// Open an envelope under a KEK, whichever way its key is held.
async fn open(
    kek: &Kek<Recovery, Opening>,
    envelope: &Envelope<Recovery>,
) -> anyhow::Result<zeroize::Zeroizing<[u8; 32]>> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        Ok(crate::webcrypto_kek::open_seed(kek, envelope).await?)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Ok(kek.open_seed(envelope)?)
    }
}
