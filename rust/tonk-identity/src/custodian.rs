//! What holds an account: a passkey in a browser, a keypair on a
//! device.
//!
//! Both answer the same three questions — what names the custody space,
//! what opens its envelopes, what seals one — and reach them
//! differently. A passkey derives from PRF outputs the authenticator
//! evaluates; a local custodian reveals a KEK it sealed to itself.
//!
//! One enum with a variant per backing, the way
//! `dialog_credentials::Ed25519SigningKey` splits native from
//! WebCrypto. Not a trait: nothing is generic over custodians, and a
//! trait would cost indirection for callers that always know which they
//! hold.

use dialog_credentials::Signer;

use crate::clearance::Recovery;
use crate::envelope::capability::{Opening, Sealing};
use crate::envelope::{Kek, KekMethod};

/// Whatever holds the account: a passkey, or a key on this device.
#[derive(Clone)]
pub enum Custodian {
    /// A locally generated keypair. What a device has before it signs
    /// in, and what the CLI has always.
    ///
    /// Boxed: it is far larger than the passkey variant, and an unboxed
    /// one would make every `Custodian` pay its size.
    Native(Box<native::Custodian>),

    /// A passkey, reached through the two handles its PRF evaluates to.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    Passkey(crate::webcrypto_kek::Custodian),

    /// A recovery phrase, which derives what a passkey's PRF would (see
    /// [`crate::phrase`]). Works everywhere: it needs no authenticator.
    ///
    /// Boxed for the same reason as [`Self::Native`].
    Phrase(Box<crate::phrase::custodian::Custodian>),
}

impl Custodian {
    /// The signer that names this custodian's custody space.
    ///
    /// Dialog's algorithm-agnostic [`Signer`], not the concrete Ed25519
    /// one: every builder downstream takes it, and what algorithm a
    /// custodian happens to use is not a caller's business.
    ///
    /// No `seed` beside it: a passkey derives one on the way here, but
    /// a native custodian is a stored non-extractable credential and has
    /// no seed to give back.
    pub async fn signer(&self) -> anyhow::Result<Signer> {
        match self {
            Self::Native(custodian) => custodian.signer().await,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Self::Passkey(custodian) => custodian.signer().await,
            Self::Phrase(custodian) => custodian.signer().await,
        }
    }

    /// A KEK that opens this custodian's envelopes and cannot seal.
    pub async fn opener(&self) -> anyhow::Result<Kek<Recovery, Opening>> {
        match self {
            Self::Native(custodian) => custodian.opener().await,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Self::Passkey(custodian) => custodian
                .opener()
                .await
                .map_err(|error| anyhow::anyhow!("deriving the opener failed: {error:?}")),
            Self::Phrase(custodian) => custodian.opener().await,
        }
    }

    /// The WebAuthn credential this custodian is reached through, when
    /// it is a passkey. A native custodian has none: nothing picks it
    /// from a list.
    pub fn credential_id(&self) -> Option<&[u8]> {
        match self {
            Self::Native(_) | Self::Phrase(_) => None,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Self::Passkey(custodian) => Some(&custodian.credential_id),
        }
    }

    /// How an envelope sealed under this custodian records its KEK.
    ///
    /// A native custodian writes [`KekMethod::Passkey`], as it always
    /// has: the method is bound into every envelope's associated data,
    /// and changing what one already writes would gain nothing.
    pub fn method(&self) -> KekMethod {
        match self {
            Self::Native(_) => KekMethod::Passkey,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Self::Passkey(_) => KekMethod::Passkey,
            Self::Phrase(_) => KekMethod::Phrase,
        }
    }

    /// The account this custodian holds.
    ///
    /// A builder rather than a value because the ways to reach one are
    /// not interchangeable: `create` makes a new account, `import`
    /// opens one already sealed under this custodian, `adopt` seals an
    /// existing account under it, and `load` fetches the custody cell.
    /// Naming the custodian first is what makes it impossible to seal
    /// under one and open with another.
    pub fn account(&self) -> crate::account::AccountBuilder<'_> {
        crate::account::AccountBuilder(self)
    }

    /// A KEK that seals under this custodian.
    pub async fn sealer(&self) -> anyhow::Result<Kek<Recovery, Sealing>> {
        match self {
            Self::Native(custodian) => custodian.sealer().await,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Self::Passkey(custodian) => custodian
                .sealer()
                .await
                .map_err(|error| anyhow::anyhow!("deriving the sealer failed: {error:?}")),
            Self::Phrase(custodian) => custodian.sealer().await,
        }
    }
}

/// A custodian backed by a keypair this device holds.
pub mod native {
    use anyhow::Result;
    use dialog_credentials::{Ed25519Signer, Signer};

    use crate::clearance::Recovery;
    use crate::envelope::Kek;
    use crate::envelope::capability::{Opening, Sealing};
    use dialog_credentials::secret::SealedSecret;

    /// A keypair and the KEK it sealed to itself.
    ///
    /// The keypair is what the custody space is named by; the sealed
    /// secret is what the KEK comes back from. Separate on purpose —
    /// retiring a custodian demotes the keypair to its public half and
    /// leaves an envelope that is deliberately unopenable rather than
    /// absent.
    #[derive(Clone)]
    pub struct Custodian {
        signer: Ed25519Signer,
        kek: SealedSecret,
    }

    impl Custodian {
        /// Adopt a keypair and the KEK it holds.
        pub fn new(signer: Ed25519Signer, kek: SealedSecret) -> Self {
            Self { signer, kek }
        }

        pub(super) async fn signer(&self) -> Result<Signer> {
            Ok(Signer::from(self.signer.clone()))
        }

        pub(super) async fn opener(&self) -> Result<Kek<Recovery, Opening>> {
            Kek::from_custodian_sealed(&self.signer, &self.kek)
                .await
                .map_err(|error| anyhow::anyhow!("revealing the custody KEK failed: {error}"))
        }

        pub(super) async fn sealer(&self) -> Result<Kek<Recovery, Sealing>> {
            Kek::from_custodian_sealed(&self.signer, &self.kek)
                .await
                .map_err(|error| anyhow::anyhow!("revealing the custody KEK failed: {error}"))
        }
    }
}
