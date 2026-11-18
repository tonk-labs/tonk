//! A recovery phrase: twelve words that open the account the way a
//! passkey does.
//!
//! A passkey reaches the account through two PRF outputs: one names the
//! custody space, the other derives the KEK that wraps the account
//! secret (see [`crate::envelope`]). A recovery phrase stands in for the
//! authenticator. Its 128 random bits are stretched with HKDF into one
//! 32-byte entry value, and [`crate::envelope::custody_signer`] and
//! [`crate::envelope::custody_kek`] derive the custody keypair and the
//! KEK from that value at the same two salts a passkey's PRF is
//! evaluated at. From there the phrase is an ordinary custodian:
//! adding one seals the existing account secret under it, exactly as
//! adding a second passkey does, and signing in opens that envelope.
//!
//! The words are BIP-39's English encoding of the 128 bits, checksum
//! included, used only as an encoding: the words decode back to the
//! same bits. BIP-39's own seed derivation (PBKDF2 over the words) is
//! not used, and neither is BIP-32: there is no wallet hierarchy here,
//! only one secret.
//!
//! There is no stretching beyond HKDF. Argon2id protects secrets a
//! person chooses; these bits come from the platform CSPRNG, and 128 of
//! them cannot be guessed.
//!
//! A phrase is permanent. It opens the account secret itself, so
//! nothing can take back what a holder of the words may already have:
//! deleting its envelope would stop new sign-ins and promise more than
//! it delivers. It stops working only when the account secret is
//! rotated.

use anyhow::Result;
use bip39::{Language, Mnemonic};
use hkdf::Hkdf;
use sha2_0_10::Sha256;
use zeroize::Zeroizing;

use crate::custodian::Custodian;

/// How many words a phrase has: 128 bits of entropy plus a 4-bit
/// checksum, at 11 bits a word.
pub const PHRASE_WORDS: usize = 12;

/// Bytes of entropy behind [`PHRASE_WORDS`] words.
pub const PHRASE_ENTROPY: usize = 16;

/// HKDF info stretching a phrase's entropy into the 32-byte entry value
/// that stands in for a passkey's PRF output. Bumping the version makes
/// every existing phrase open nothing, so it never changes routinely.
pub const PHRASE_ENTRY_CONTEXT: &[u8] = b"tonk/recovery-phrase/entry/v1";

/// Why some text is not a recovery phrase.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PhraseError {
    /// The text has a different number of words.
    #[error("a recovery phrase has {expected} words, this has {found}")]
    WordCount {
        /// [`PHRASE_WORDS`].
        expected: usize,
        /// How many words the text had.
        found: usize,
    },
    /// A word is not in the BIP-39 English list. Zero-based.
    #[error("word {} is not a recovery phrase word", .position + 1)]
    UnknownWord {
        /// Where in the phrase the word is, counting from zero.
        position: usize,
    },
    /// Every word is real but they do not add up: a word is wrong or
    /// two are swapped.
    #[error("these words are not a recovery phrase; check each one")]
    Checksum,
}

/// The 128 bits a recovery phrase spells. Zeroized on drop.
pub struct RecoveryPhrase(Zeroizing<[u8; PHRASE_ENTROPY]>);

impl RecoveryPhrase {
    /// A new phrase from the platform CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut entropy = Zeroizing::new([0u8; PHRASE_ENTROPY]);
        getrandom::fill(entropy.as_mut())
            .map_err(|error| anyhow::anyhow!("no entropy for a recovery phrase: {error}"))?;
        Ok(Self(entropy))
    }

    /// Read a phrase someone typed or pasted.
    ///
    /// Case and whitespace do not matter, so a phrase copied with line
    /// breaks or capitalised by a phone keyboard still reads.
    ///
    /// # Errors
    ///
    /// [`PhraseError`] for the wrong number of words, a word outside the
    /// list, or words whose checksum does not match.
    pub fn parse(text: &str) -> Result<Self, PhraseError> {
        let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if words.len() != PHRASE_WORDS {
            return Err(PhraseError::WordCount {
                expected: PHRASE_WORDS,
                found: words.len(),
            });
        }
        let normalized = Zeroizing::new(words.join(" "));
        let mnemonic =
            Mnemonic::parse_in_normalized(Language::English, &normalized).map_err(|error| {
                match error {
                    bip39::Error::UnknownWord(position) => PhraseError::UnknownWord { position },
                    bip39::Error::BadWordCount(found) => PhraseError::WordCount {
                        expected: PHRASE_WORDS,
                        found,
                    },
                    _ => PhraseError::Checksum,
                }
            })?;
        let (bytes, length) = mnemonic.to_entropy_array();
        let bytes = Zeroizing::new(bytes);
        let mut entropy = Zeroizing::new([0u8; PHRASE_ENTROPY]);
        if length != PHRASE_ENTROPY {
            return Err(PhraseError::WordCount {
                expected: PHRASE_WORDS,
                found: words.len(),
            });
        }
        entropy.copy_from_slice(&bytes[..PHRASE_ENTROPY]);
        Ok(Self(entropy))
    }

    /// The twelve words, space-separated, to show once.
    pub fn words(&self) -> Zeroizing<String> {
        let mnemonic = Mnemonic::from_entropy_in(Language::English, self.0.as_ref())
            .expect("16 bytes is a valid BIP-39 entropy length");
        Zeroizing::new(mnemonic.words().collect::<Vec<_>>().join(" "))
    }

    /// The 32-byte value that stands in for a passkey's PRF output.
    fn entry(&self) -> Zeroizing<[u8; 32]> {
        let hkdf = Hkdf::<Sha256>::new(None, self.0.as_ref());
        let mut entry = Zeroizing::new([0u8; 32]);
        hkdf.expand(PHRASE_ENTRY_CONTEXT, entry.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        entry
    }

    /// The custodian this phrase is: the custody keypair that names its
    /// custody space and the KEK that opens its envelope.
    pub async fn custodian(&self) -> Result<Custodian> {
        let entry = self.entry();
        let signer = crate::envelope::custody_signer(&entry).await?;
        Ok(Custodian::Phrase(Box::new(custodian::Custodian::new(
            signer,
            crate::envelope::custody_kek_bytes(&entry),
        ))))
    }
}

/// A custodian derived from a recovery phrase.
pub mod custodian {
    use anyhow::Result;
    use dialog_credentials::{Ed25519Signer, Signer};
    use zeroize::Zeroizing;

    use crate::clearance::Recovery;
    use crate::envelope::Kek;
    use crate::envelope::capability::{Opening, Sealing};

    /// The custody keypair and KEK a phrase derives.
    ///
    /// The KEK is held as bytes: the phrase was typed into this page, so
    /// there is no authenticator boundary a WebCrypto handle would keep
    /// it behind, and the bytes zeroize on drop.
    #[derive(Clone)]
    pub struct Custodian {
        signer: Ed25519Signer,
        kek: Zeroizing<[u8; 32]>,
    }

    impl Custodian {
        pub(super) fn new(signer: Ed25519Signer, kek: Zeroizing<[u8; 32]>) -> Self {
            Self { signer, kek }
        }

        pub(crate) async fn signer(&self) -> Result<Signer> {
            Ok(Signer::from(self.signer.clone()))
        }

        pub(crate) async fn opener(&self) -> Result<Kek<Recovery, Opening>> {
            Ok(Kek::from_bytes(self.kek.clone()))
        }

        pub(crate) async fn sealer(&self) -> Result<Kek<Recovery, Sealing>> {
            Ok(Kek::from_bytes(self.kek.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;

    use super::*;
    use crate::account::Crypto;
    use crate::clearance::Recovery;
    use crate::custodian::native;
    use crate::envelope::{Kek, KekMethod};

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    fn phrase(byte: u8) -> RecoveryPhrase {
        RecoveryPhrase(Zeroizing::new([byte; PHRASE_ENTROPY]))
    }

    async fn did(custodian: &Custodian) -> String {
        custodian.signer().await.unwrap().did().to_string()
    }

    async fn native_custodian() -> Custodian {
        let signer = Ed25519Signer::generate().await.unwrap();
        let (_, sealed) = Kek::<Recovery>::seal_to_custodian(&signer).await.unwrap();
        Custodian::Native(Box::new(native::Custodian::new(signer, sealed)))
    }

    /// The BIP-39 test vector for all-zero entropy, so the encoding is
    /// the standard one and another implementation reads our words.
    #[dialog_common::test]
    fn it_spells_entropy_as_standard_bip39_words() {
        assert_eq!(
            &*phrase(0).words(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );
        assert_eq!(phrase(0).words().split(' ').count(), PHRASE_WORDS);
    }

    #[dialog_common::test]
    fn it_reads_back_the_words_it_shows() {
        let generated = RecoveryPhrase::generate().unwrap();
        let read = RecoveryPhrase::parse(&generated.words()).unwrap();
        assert_eq!(*read.0, *generated.0);
    }

    #[dialog_common::test]
    fn it_reads_a_phrase_whatever_its_case_and_spacing() {
        let words = phrase(7).words();
        let messy = format!("  {}\n", words.to_uppercase().replace(' ', "\n\t "));
        assert_eq!(*RecoveryPhrase::parse(&messy).unwrap().0, *phrase(7).0);
    }

    #[dialog_common::test]
    fn it_refuses_a_phrase_with_one_word_wrong() {
        let words = phrase(7).words();
        let mut swapped: Vec<&str> = words.split(' ').collect();
        let last = swapped.len() - 1;
        swapped[last] = if swapped[last] == "zoo" {
            "zebra"
        } else {
            "zoo"
        };
        let refused = RecoveryPhrase::parse(&swapped.join(" ")).err();
        assert!(
            matches!(refused, Some(PhraseError::Checksum)),
            "{refused:?}"
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_word_outside_the_list() {
        let words = phrase(7).words();
        let mut wrong: Vec<&str> = words.split(' ').collect();
        wrong[3] = "tonk";
        let refused = RecoveryPhrase::parse(&wrong.join(" ")).err();
        assert!(
            matches!(refused, Some(PhraseError::UnknownWord { position: 3 })),
            "{refused:?}"
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_phrase_of_another_length() {
        let words = phrase(7).words();
        let short: Vec<&str> = words.split(' ').take(11).collect();
        let refused = RecoveryPhrase::parse(&short.join(" ")).err();
        assert!(
            matches!(
                refused,
                Some(PhraseError::WordCount {
                    expected: 12,
                    found: 11
                })
            ),
            "{refused:?}"
        );
        let long = format!("{} {}", *words, *words);
        assert!(matches!(
            RecoveryPhrase::parse(&long),
            Err(PhraseError::WordCount { found: 24, .. })
        ));
    }

    #[dialog_common::test]
    async fn the_same_phrase_names_the_same_custody_space() {
        let first = phrase(5).custodian().await.unwrap();
        let again = phrase(5).custodian().await.unwrap();
        let other = phrase(6).custodian().await.unwrap();
        assert_eq!(did(&first).await, did(&again).await);
        assert_ne!(did(&first).await, did(&other).await);
    }

    #[dialog_common::test]
    async fn an_account_sealed_under_a_phrase_opens_with_it_and_no_other() {
        let custodian = phrase(5).custodian().await.unwrap();
        let account = custodian.account().create().perform(&Crypto).await.unwrap();
        assert!(matches!(account.envelope().method(), KekMethod::Phrase));
        let decoded =
            crate::envelope::Envelope::<Recovery>::decode(&account.envelope().encode()).unwrap();
        assert!(matches!(decoded.method(), KekMethod::Phrase));
        let root = account.signer().await.unwrap().did().to_string();

        let reopened = phrase(5)
            .custodian()
            .await
            .unwrap()
            .account()
            .import(account.envelope().clone())
            .perform(&Crypto)
            .await
            .unwrap();
        assert_eq!(reopened.signer().await.unwrap().did().to_string(), root);

        let refused = phrase(6)
            .custodian()
            .await
            .unwrap()
            .account()
            .import(account.envelope().clone())
            .perform(&Crypto)
            .await;
        assert!(refused.is_err(), "another phrase opened the envelope");
    }

    /// Adding a phrase to an account another custodian holds, the way a
    /// second passkey is added: both open the same account.
    #[dialog_common::test]
    async fn a_phrase_and_another_custodian_open_the_same_account() {
        let holder = native_custodian().await;
        let account = holder.account().create().perform(&Crypto).await.unwrap();
        let root = account.signer().await.unwrap().did().to_string();
        let held = account.envelope().clone();

        let phrase_custodian = phrase(9).custodian().await.unwrap();
        let sealed = phrase_custodian
            .account()
            .adopt(account.into_secret())
            .perform(&Crypto)
            .await
            .unwrap();

        let by_phrase = phrase(9)
            .custodian()
            .await
            .unwrap()
            .account()
            .import(sealed.envelope().clone())
            .perform(&Crypto)
            .await
            .unwrap();
        let by_holder = holder
            .account()
            .import(held)
            .perform(&Crypto)
            .await
            .unwrap();
        assert_eq!(by_phrase.signer().await.unwrap().did().to_string(), root);
        assert_eq!(by_holder.signer().await.unwrap().did().to_string(), root);
    }

    /// The browser case the feature is for: an account a passkey holds
    /// gains a phrase, and from then on the phrase opens it too.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn a_phrase_added_to_a_passkey_account_opens_it() {
        let passkey = Custodian::Passkey(
            crate::webcrypto_kek::Custodian::adopt(vec![1], &[11u8; 32], &[22u8; 32])
                .await
                .unwrap(),
        );
        let account = passkey.account().create().perform(&Crypto).await.unwrap();
        let root = account.signer().await.unwrap().did().to_string();

        let sealed = phrase(4)
            .custodian()
            .await
            .unwrap()
            .account()
            .adopt(account.into_secret())
            .perform(&Crypto)
            .await
            .unwrap();
        assert!(matches!(sealed.envelope().method(), KekMethod::Phrase));

        let opened = phrase(4)
            .custodian()
            .await
            .unwrap()
            .account()
            .import(sealed.envelope().clone())
            .perform(&Crypto)
            .await
            .unwrap();
        assert_eq!(opened.signer().await.unwrap().did().to_string(), root);
    }
}
