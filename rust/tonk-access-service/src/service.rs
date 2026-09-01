//! The service's own signing identity.
//!
//! Registration mints activation delegations issued by the service, so
//! the service holds an ed25519 keypair, derived from the
//! `SERVICE_SECRET_KEY` secret: 32 seed bytes, hex encoded. Invocations
//! address the key's `did:key` form today; the DID document served at
//! `/.well-known/did.json` publishes the same key under the host's
//! `did:web` name so the service DID can move there once resolution
//! support lands.

use dialog_credentials::Ed25519Signer;
use dialog_varsig::{Did, Principal};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use serde_json::{Value, json};
use sha2_0_10::Sha256;

use crate::store::Customer;

/// Build the service signer from its hex-encoded 32-byte seed.
pub fn signer_from_hex(seed_hex: &str) -> Result<Ed25519Signer, String> {
    let bytes = hex::decode(seed_hex.trim())
        .map_err(|err| format!("SERVICE_SECRET_KEY is not valid hex: {err}"))?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "SERVICE_SECRET_KEY must be 32 bytes of hex".to_string())?;
    Ok(Ed25519Signer::from(SigningKey::from_bytes(&seed)))
}

/// HKDF info for a customer space's signing seed. Bumping the version
/// re-derives every customer space, so it is a deliberate rotation
/// rather than a routine change.
const LEDGER_CONTEXT: &[u8] = b"tonk/customer/ledger/v1";

/// Derive the signer for `account`'s ledger — the space this service
/// owns and replicates its metering and billing into.
///
/// Derived rather than generated so nothing has to be stored: no key at
/// rest, no sealing, no rotation table, and the DID is recomputable from
/// the account DID whenever it is needed. The service seed is the only
/// secret involved, which is also what makes this compatible with moving
/// the service identity to a hardware key later: the derivation goes
/// away in favour of delegations from it, and no stored key has to be
/// migrated.
///
/// Binding the account DID into the info means one customer cannot
/// derive another's space key even knowing this construction, since the
/// service seed is the only unknown.
pub fn ledger_signer(seed_hex: &str, account: &Did) -> Result<Ed25519Signer, String> {
    let seed = hex::decode(seed_hex.trim())
        .map_err(|err| format!("SERVICE_SECRET_KEY is not valid hex: {err}"))?;
    let hkdf = Hkdf::<Sha256>::new(None, &seed);
    let mut derived = [0u8; 32];
    hkdf.expand(
        &[LEDGER_CONTEXT, account.to_string().as_bytes()].concat(),
        &mut derived,
    )
    .map_err(|err| format!("customer space derivation failed: {err}"))?;
    Ok(Ed25519Signer::from(SigningKey::from_bytes(&derived)))
}

/// The multibase key of a `did:key`, which is its method-specific part.
/// A DID that is not a `did:key` has none.
fn multibase_of(did_key: &str) -> Option<&str> {
    did_key.strip_prefix("did:key:")
}

/// A DID document for `id`, carrying every `did:key` in `keys` as a
/// `Multikey` verification method under that name.
///
/// Keys are embedded rather than referenced: each method is identified by
/// `{id}#{multibase}` and controlled by `id`, so the document verifies
/// standalone without a resolver having to dereference a second DID. The
/// multibase is the `did:key` identifier's method-specific part, so a
/// name here and the `did:key` it came from verify against one key.
///
/// A key that is not a `did:key` is skipped: there is no multibase to
/// publish, and emitting a method without key material would produce a
/// document that resolves but cannot verify anything.
fn document(id: &str, keys: impl IntoIterator<Item = String>) -> Value {
    let methods: Vec<Value> = keys
        .into_iter()
        .filter_map(|did_key| {
            let multibase = multibase_of(&did_key)?;
            Some(json!({
                "id": format!("{id}#{multibase}"),
                "type": "Multikey",
                "controller": id,
                "publicKeyMultibase": multibase
            }))
        })
        .collect();
    let references: Vec<Value> = methods.iter().map(|method| method["id"].clone()).collect();
    json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/multikey/v1"
        ],
        "id": id,
        "verificationMethod": methods,
        "authentication": references,
        "assertionMethod": references
    })
}

/// The DID document for `did:web:{host}`, carrying the service's ed25519
/// key.
pub fn did_document(host: &str, origin: &str, signer: &Ed25519Signer) -> Value {
    // `did:web` separates path segments with `:`, so a host carrying a
    // port percent-encodes it (`localhost%3A8090`), per the method spec.
    // Raw, `did:web:localhost:8090` reads `8090` as a path segment and a
    // resolver fetches `https://localhost/8090/did.json`.
    let id = format!("did:web:{}", crate::lookup::encode_host(host));
    let mut document = document(&id, [signer.did().to_string()]);
    // Where this service is reached, in the document that already answers
    // who it is. That is what a `service` block is for, and publishing it
    // here means a client resolving the service needs nothing but its DID
    // — no side channel naming the endpoint, and no second well-known
    // path to keep in step with this one.
    document["service"] = json!([{
        "id": format!("{id}#ucan"),
        "type": "TonkAccessService",
        "serviceEndpoint": format!("{}/ucan/", origin.trim_end_matches('/')),
    }]);
    document
}

/// The DID document for an email address, carrying the `did:key` of the
/// customer registered under it.
///
/// The key is embedded under the `did:web` name rather than referenced,
/// so the document verifies standalone. `alsoKnownAs` names the same key
/// as a `did:key`, which is the form the rest of the system uses, so a
/// caller need not rebuild it from the multibase.
///
/// `deactivated` marks a suspended customer. The key stays in the
/// document: a suspension is reversible, and dropping the mapping would
/// make a resumed customer unresolvable to anyone holding the old answer.
pub fn customer_document(id: &str, customer: &Customer, deactivated: bool, origin: &str) -> Value {
    let mut document = document(id, [customer.account.clone()]);
    document["alsoKnownAs"] = json!([customer.account]);
    document["status"] = json!(customer.status.as_str());
    // Where this account syncs, in the document that already answers
    // who it is. A device holding only the address resolves both in one
    // fetch, so nothing has to publish an endpoint separately or freeze
    // one at signup.
    document["service"] = json!([{
        "id": format!("{id}#sync"),
        "type": "TonkAccessService",
        "serviceEndpoint": format!("{}/ucan/", origin.trim_end_matches('/')),
    }]);
    if deactivated {
        document["deactivated"] = Value::Bool(true);
    }
    document
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_derives_a_stable_did_from_the_seed() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let again = signer_from_hex(&"11".repeat(32)).unwrap();
        assert_eq!(signer.did(), again.did());
        assert!(signer.did().to_string().starts_with("did:key:z6Mk"));
    }

    #[dialog_common::test]
    fn it_rejects_a_malformed_seed() {
        assert!(signer_from_hex("not hex").is_err());
        assert!(signer_from_hex("11").is_err());
    }

    /// The whole point of deriving rather than storing: the same
    /// account always yields the same space, so nothing has to be
    /// persisted to find it again.
    #[dialog_common::test]
    fn it_derives_the_same_ledger_every_time() {
        let seed = "11".repeat(32);
        let account: Did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
            .parse()
            .unwrap();
        let once = ledger_signer(&seed, &account).unwrap();
        let again = ledger_signer(&seed, &account).unwrap();
        assert_eq!(once.did(), again.did());
    }

    /// One customer's space must not be reachable from another's, and
    /// neither from the service identity everything derives from.
    #[dialog_common::test]
    fn it_separates_customers_and_the_service_identity() {
        let seed = "11".repeat(32);
        let alice: Did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
            .parse()
            .unwrap();
        let bob: Did = "did:key:z6MkrZ1r5XBFZjBU34qyD8fueMbMRkKw17BZaq2ivKFjnz2z"
            .parse()
            .unwrap();
        let alice_space = ledger_signer(&seed, &alice).unwrap();
        let bob_space = ledger_signer(&seed, &bob).unwrap();
        assert_ne!(alice_space.did(), bob_space.did());
        assert_ne!(
            alice_space.did(),
            signer_from_hex(&seed).unwrap().did(),
            "a customer space is not the service itself"
        );
    }

    /// A different service seed derives different spaces, so a
    /// deployment cannot reach another's — and a seed rotation is a
    /// deliberate act with visible consequences.
    #[dialog_common::test]
    fn it_binds_the_ledger_to_the_service_seed() {
        let account: Did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
            .parse()
            .unwrap();
        let one = ledger_signer(&"11".repeat(32), &account).unwrap();
        let other = ledger_signer(&"22".repeat(32), &account).unwrap();
        assert_ne!(one.did(), other.did());
    }

    #[dialog_common::test]
    fn it_documents_the_key_under_the_web_did() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let document = did_document("tonk.network", "https://tonk.network", &signer);
        assert_eq!(document["id"], "did:web:tonk.network");
        let multibase = document["verificationMethod"][0]["publicKeyMultibase"]
            .as_str()
            .unwrap();
        assert_eq!(format!("did:key:{multibase}"), signer.did().to_string());
    }

    /// The service announces where it is reached, in its own document.
    ///
    /// That is what a DID `service` block is for, and it is what lets a
    /// client resolve the endpoint from the DID alone. Without it the
    /// endpoint had to come from somewhere else — a separate well-known
    /// path carrying one field — which is a second thing to keep in step
    /// with this document for no gain.
    #[dialog_common::test]
    fn it_announces_where_the_service_is_reached() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let document = did_document("tonk.network", "https://tonk.network", &signer);
        let service = &document["service"][0];
        assert_eq!(service["type"], "TonkAccessService");
        assert_eq!(service["serviceEndpoint"], "https://tonk.network/ucan/");
        assert_eq!(
            service["id"], "did:web:tonk.network#ucan",
            "the fragment names the endpoint within the document"
        );
    }

    /// A non-default port survives into the endpoint.
    ///
    /// The origin is configuration, not the request's `Host` header: a dev
    /// proxy forwards `Host: 127.0.0.1`, and a document built from that
    /// published `http://127.0.0.1/ucan/` — port 80, nothing listening,
    /// every fetch failing.
    #[dialog_common::test]
    fn it_keeps_the_port_the_service_is_reached_on() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let document = did_document("localhost:8090", "http://localhost:8090", &signer);
        assert_eq!(
            document["service"][0]["serviceEndpoint"],
            "http://localhost:8090/ucan/"
        );
        // The DID itself percent-encodes the port: `did:web` separates
        // path segments with `:`, so a raw one would resolve to
        // `https://localhost/8090/did.json`.
        assert_eq!(
            document["id"], "did:web:localhost%3A8090",
            "a port is one segment, not a path"
        );
    }
}
