//! Sealed command fields: an attribute declared `as: secret`.
//!
//! A secret is stored as the sealed bytes an invoker produced with
//! [`tonk_identity::secret`], never as its plaintext. In Rust such a field
//! holds a [`Sealed<T>`]: a provider asks its environment to reveal it,
//! its `Debug` prints `[sealed]`, and nothing else can read it.
//!
//! The attributes that hold secrets are listed in [`SEALED`] and declared
//! `as: secret` in the library, which records them as sealed
//! (`tonk.dialog.attribute/sealed`). Invokers read that record to know
//! which fields to seal; the worker reads [`SEALED`] to refuse a command
//! whose secret arrived unsealed. A test keeps the two in step.

use std::marker::PhantomData;

use dialog_artifacts::{TypeError, Value, ValueDataType};
use dialog_query::{AttributeDescriptor, Descriptor, Typed};
pub use tonk_identity::sealed::RecipientKey;
pub use tonk_identity::secret::{SecretError, SecretKey};
use zeroize::Zeroizing;

/// The attribute that records an attribute as sealed: `true` on the
/// attribute entity of every attribute declared `as: secret`.
pub const SEALED_MARKER: &str = "tonk.dialog.attribute/sealed";

/// Every attribute that holds a sealed secret, by relation. The worker
/// refuses a command carrying one of these unsealed.
pub const SEALED: [&str; 4] = [
    "xyz.tonk.command.join/sealed-url",
    "xyz.tonk.command.finish-sign-in-via/sealed-url",
    "xyz.tonk.command.send-email-invitation/sealed-id",
    "xyz.tonk.command.claim-email-invitation/sealed-id",
];

/// Whether `relation` holds a sealed secret.
pub fn is_sealed(relation: &str) -> bool {
    SEALED.contains(&relation)
}

/// Attributes that held a secret in plaintext in an earlier command
/// shape, which still decodes while older pages are around. An
/// invocation copies none of them into the overlay, whatever command
/// they arrive on.
pub const PLAINTEXT_LEGACY: [&str; 5] = [
    "dom.event.detail/href",
    "xyz.tonk.command.join/url",
    "xyz.tonk.command.finish-sign-in-via/url",
    "xyz.tonk.send-email-invitation/id",
    "xyz.tonk.claim-email-invitation/id",
];

/// The key invokers seal to, published by the worker on the overlay of
/// the profile's meta branch at [`CommandSecretRecipient::ENTITY`].
/// Declared as `command-secret/recipient` in `profile.yaml`.
#[derive(dialog_query::Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommandSecretRecipient {
    /// Always [`CommandSecretRecipient::ENTITY`].
    pub this: dialog_artifacts::Entity,
    /// The X25519 public key, as a `did:key:z6LS…`.
    pub recipient: crate::domain::command_secret::Recipient,
}

impl CommandSecretRecipient {
    /// The one entity the recipient is published on.
    pub const ENTITY: &str = "tonk:command-secret";

    /// The fact publishing `recipient`.
    pub fn new(recipient: &tonk_identity::sealed::RecipientKey) -> Self {
        use dialog_varsig::Principal as _;
        Self {
            this: Self::ENTITY.parse().expect("a valid entity"),
            recipient: crate::domain::command_secret::Recipient(recipient.did().to_string()),
        }
    }
}

/// A secret as sealed bytes, standing for a `T` only the worker can
/// reveal.
pub struct Sealed<T> {
    bytes: Vec<u8>,
    plain: PhantomData<fn() -> T>,
}

impl<T> Sealed<T> {
    /// Adopt bytes an invoker sealed.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            plain: PhantomData,
        }
    }

    /// The sealed bytes, as stored.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl<T> Clone for Sealed<T> {
    fn clone(&self) -> Self {
        Self::from_bytes(self.bytes.clone())
    }
}

impl<T> PartialEq for Sealed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<T> Eq for Sealed<T> {}

impl<T> PartialOrd for Sealed<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Sealed<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bytes.cmp(&other.bytes)
    }
}

impl<T> std::hash::Hash for Sealed<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl<T> std::fmt::Debug for Sealed<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[sealed]")
    }
}

impl<T> Typed for Sealed<T> {
    type Descriptor = <Vec<u8> as Typed>::Descriptor;
}

impl<T: 'static> dialog_query::Scalar for Sealed<T> {}

impl<T> From<Sealed<T>> for Value {
    fn from(sealed: Sealed<T>) -> Self {
        Value::Bytes(sealed.bytes)
    }
}

impl<T> TryFrom<Value> for Sealed<T> {
    type Error = TypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Bytes(bytes) => Ok(Self::from_bytes(bytes)),
            other => Err(TypeError::TypeMismatch(
                ValueDataType::Bytes,
                other.data_type(),
            )),
        }
    }
}

/// What a secret reveals as.
pub trait Plaintext: Sized {
    /// Read the opened bytes.
    fn from_plaintext(bytes: Zeroizing<Vec<u8>>) -> Result<Self, SecretError>;
}

impl Plaintext for String {
    fn from_plaintext(bytes: Zeroizing<Vec<u8>>) -> Result<Self, SecretError> {
        String::from_utf8(bytes.to_vec()).map_err(|_| SecretError::Refused)
    }
}

/// An attribute holding a [`Sealed`] secret.
pub trait SealedAttribute:
    dialog_query::Attribute<Type = Sealed<Self::Plain>> + Descriptor<AttributeDescriptor>
{
    /// What the secret reveals as.
    type Plain: Plaintext + 'static;

    /// The relation the secret was sealed for.
    fn relation() -> String {
        <Self as Descriptor<AttributeDescriptor>>::descriptor()
            .the()
            .to_string()
    }
}

impl<A, P> SealedAttribute for A
where
    A: dialog_query::Attribute<Type = Sealed<P>> + Descriptor<AttributeDescriptor>,
    P: Plaintext + 'static,
{
    type Plain = P;
}

/// The key an operator signing as `signer` opens sealed fields with:
/// what the worker reveals with, and what the CLI seals to when it
/// evaluates a command on its own site.
pub async fn derive_key(signer: &dialog_credentials::Signer) -> Result<SecretKey, String> {
    // Irrefutable unless another signing algorithm is compiled in.
    #[allow(irrefutable_let_patterns)]
    let dialog_credentials::Signer::Ed25519(signer) = signer else {
        return Err("the operator key is not Ed25519".into());
    };
    SecretKey::derive(signer)
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Reveal `field` with `key`. What a provider's `reveal` calls.
pub fn reveal<A: SealedAttribute>(key: &SecretKey, field: &A) -> Result<A::Plain, SecretError> {
    let opened = key.reveal(field.value().as_bytes(), &A::relation())?;
    A::Plain::from_plaintext(opened)
}

/// Check every sealed attribute among `facts` (`(relation, value)`
/// pairs of one command) opens with `key`. A secret that is plain text,
/// malformed, sealed for another session, or does not open refuses the
/// whole command.
pub fn verify<'a>(
    key: Option<&SecretKey>,
    facts: impl IntoIterator<Item = (String, &'a Value)>,
) -> Result<(), SecretError> {
    for (relation, value) in facts {
        if !is_sealed(&relation) {
            continue;
        }
        let Value::Bytes(bytes) = value else {
            return Err(SecretError::NotSealed);
        };
        // A worker with no key cannot open anything sealed to it, which
        // is what a blob for another session amounts to.
        let key = key.ok_or(SecretError::EarlierSession)?;
        key.verify(bytes, &relation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn a_sealed_value_debugs_as_sealed() {
        let sealed: Sealed<String> = Sealed::from_bytes(b"https://x#seed".to_vec());
        assert_eq!(format!("{sealed:?}"), "[sealed]");
    }

    #[dialog_common::test]
    fn a_sealed_value_round_trips_as_bytes_and_refuses_text() {
        let sealed: Sealed<String> = Sealed::from_bytes(vec![1, 2, 3]);
        let value: Value = sealed.clone().into();
        assert_eq!(value, Value::Bytes(vec![1, 2, 3]));
        assert_eq!(Sealed::<String>::try_from(value).unwrap(), sealed);
        assert!(Sealed::<String>::try_from(Value::String("plain".into())).is_err());
    }

    #[dialog_common::test]
    fn verify_refuses_text_in_a_sealed_attribute() {
        let text = Value::String("https://tonk.example/join#seed".into());
        assert_eq!(
            verify(None, [(SEALED[0].to_owned(), &text)]),
            Err(SecretError::NotSealed)
        );
    }

    /// The attributes the profile library declares `as: secret` are the
    /// ones the worker refuses unsealed: invokers seal what the library
    /// records, and the worker checks what [`SEALED`] lists.
    #[dialog_common::test]
    fn the_library_declares_exactly_the_sealed_attributes_as_secret() {
        let library = include_str!("../../tonk-core/assets/library/profile.yaml");
        let mut declared = std::collections::BTreeSet::new();
        let mut relation: Option<&str> = None;
        for line in library.lines() {
            let line = line.trim();
            if let Some(the) = line.strip_prefix("the: ") {
                relation = Some(the.trim());
            } else if line == "as: secret" {
                declared.insert(relation.expect("`as: secret` follows a `the:`").to_owned());
            } else if line.ends_with(':') && !line.starts_with('#') {
                relation = None;
            }
        }
        let listed: std::collections::BTreeSet<String> =
            SEALED.iter().map(|relation| relation.to_string()).collect();
        assert_eq!(declared, listed);
    }

    #[dialog_common::test]
    fn verify_ignores_attributes_that_are_not_sealed() {
        let text = Value::String("Plans".into());
        assert_eq!(
            verify(None, [("xyz.tonk.command.rename/name".to_owned(), &text)]),
            Ok(())
        );
    }
}
