//! What a credential signs, as the world's `payload` names it, and the bytes
//! its signature covers.
//!
//! Dialog DB builds what it signs itself, and hands its signer the bytes. A
//! guest kernel reads those bytes into a [`Payload`] ([`read`]), and its host
//! writes the same bytes from it ([`write`]) and signs them, so the host
//! signs only what it can read, and each kind in its own encoding:
//!
//! - a UCAN invocation or delegation: the DAG-CBOR map
//!   `{ "h": <varsig header>, "ucan/…": <payload> }`;
//! - a head of a branch: the tag `dialog/head@1\n`, then its fields;
//! - a revision record: the tag `dialog/revision-record@1\n`, then the
//!   record in DAG-CBOR, with an empty signature.
//!
//! The issuer of each is the credential that signs it, so a [`Payload`]
//! leaves it out, and [`write`] puts it back.

use std::collections::BTreeMap;

use ipld_core::cid::Cid;
use ipld_core::ipld::Ipld;

/// The tag that opens what a head's signature covers.
pub const HEAD_TAG: &[u8] = b"dialog/head@1\n";
/// The tag that opens what a revision record's signature covers.
pub const RECORD_TAG: &[u8] = b"dialog/revision-record@1\n";

const INVOCATION: &str = "ucan/inv@1.0.0-rc.1";
const DELEGATION: &str = "ucan/dlg@1.0.0-rc.1";

/// What a credential signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    Invocation(Invocation),
    Delegation(Delegation),
    Head(Head),
    RevisionRecord(RevisionRecord),
}

/// A UCAN invocation, without its issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub audience: Option<String>,
    pub subject: String,
    pub command: String,
    /// DAG-CBOR of the map of arguments.
    pub arguments: Vec<u8>,
    /// The CIDs of the delegations that prove it, as bytes.
    pub proofs: Vec<Vec<u8>>,
    pub cause: Option<Vec<u8>>,
    pub issued_at: Option<u64>,
    pub expiration: Option<u64>,
    /// DAG-CBOR of the map of metadata.
    pub meta: Option<Vec<u8>>,
    pub nonce: Vec<u8>,
}

/// A UCAN delegation, without its issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    pub audience: String,
    /// None delegates for each subject the issuer can reach.
    pub subject: Option<String>,
    pub command: String,
    /// DAG-CBOR of the list of predicates.
    pub policy: Vec<u8>,
    pub expiration: Option<u64>,
    pub not_before: Option<u64>,
    /// DAG-CBOR of the map of metadata.
    pub meta: Option<Vec<u8>>,
    pub nonce: Vec<u8>,
}

/// The head of a branch, without its issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub branch: String,
    pub tree: [u8; 32],
    pub edition: u64,
    pub context: Option<Vec<Watermark>>,
}

/// What a head's context says of one origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    pub origin: [u8; 32],
    pub edition: u64,
    pub count: u64,
}

/// A revision record, without its issuer and its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionRecord {
    /// DAG-CBOR of the record's other fields.
    pub body: Vec<u8>,
}

/// The varsig header of an Ed25519 signature over DAG-CBOR, as a UCAN
/// carries it under `h`.
const ED25519_DAG_CBOR: &[u8] = &[0x34, 0x01, 0xed, 0x01, 0xed, 0x01, 0x13, 0x71];

/// The payload `bytes` are, as `issuer` would sign them.
///
/// # Errors
///
/// If the bytes are none of the kinds a credential signs, or name another
/// issuer.
pub fn read(bytes: &[u8], issuer: &str) -> Result<Payload, String> {
    if let Some(head) = bytes.strip_prefix(HEAD_TAG) {
        return read_head(head, issuer).map(Payload::Head);
    }
    if let Some(record) = bytes.strip_prefix(RECORD_TAG) {
        return read_record(record, issuer).map(Payload::RevisionRecord);
    }
    read_ucan(bytes, issuer)
}

/// The bytes `issuer` signs for `payload`.
///
/// # Errors
///
/// If a field that holds DAG-CBOR does not.
pub fn write(payload: &Payload, issuer: &str) -> Result<Vec<u8>, String> {
    match payload {
        Payload::Head(head) => Ok(write_head(head, issuer)),
        Payload::RevisionRecord(record) => write_record(record, issuer),
        Payload::Invocation(invocation) => {
            write_ucan(INVOCATION, invocation_map(invocation, issuer)?)
        }
        Payload::Delegation(delegation) => {
            write_ucan(DELEGATION, delegation_map(delegation, issuer)?)
        }
    }
}

fn cbor(bytes: &[u8]) -> Result<Ipld, String> {
    serde_ipld_dagcbor::from_slice(bytes).map_err(|error| error.to_string())
}

fn encode(ipld: &Ipld) -> Result<Vec<u8>, String> {
    serde_ipld_dagcbor::to_vec(ipld).map_err(|error| error.to_string())
}

fn text(map: &mut BTreeMap<String, Ipld>, key: &str) -> Result<String, String> {
    match map.remove(key) {
        Some(Ipld::String(text)) => Ok(text),
        other => Err(format!("`{key}` is {other:?}, not text")),
    }
}

fn integer(ipld: Ipld, key: &str) -> Result<u64, String> {
    match ipld {
        Ipld::Integer(value) => u64::try_from(value).map_err(|_| format!("`{key}` is negative")),
        other => Err(format!("`{key}` is {other:?}, not an integer")),
    }
}

/// A field that is absent or null when it says nothing.
fn optional(map: &mut BTreeMap<String, Ipld>, key: &str) -> Option<Ipld> {
    map.remove(key).filter(|value| *value != Ipld::Null)
}

fn optional_integer(map: &mut BTreeMap<String, Ipld>, key: &str) -> Result<Option<u64>, String> {
    optional(map, key)
        .map(|value| integer(value, key))
        .transpose()
}

fn optional_cbor(map: &mut BTreeMap<String, Ipld>, key: &str) -> Result<Option<Vec<u8>>, String> {
    optional(map, key).map(|value| encode(&value)).transpose()
}

fn link(ipld: Ipld, key: &str) -> Result<Vec<u8>, String> {
    match ipld {
        Ipld::Link(cid) => Ok(cid.to_bytes()),
        other => Err(format!("`{key}` holds {other:?}, not a link")),
    }
}

fn bytes_of(map: &mut BTreeMap<String, Ipld>, key: &str) -> Result<Vec<u8>, String> {
    match map.remove(key) {
        Some(Ipld::Bytes(bytes)) => Ok(bytes),
        other => Err(format!("`{key}` is {other:?}, not bytes")),
    }
}

fn issued_by(map: &mut BTreeMap<String, Ipld>, issuer: &str) -> Result<(), String> {
    let named = text(map, "iss")?;
    if named == issuer {
        Ok(())
    } else {
        Err(format!("the payload is issued by {named}, not {issuer}"))
    }
}

fn nothing_else(map: BTreeMap<String, Ipld>) -> Result<(), String> {
    match map.keys().next() {
        None => Ok(()),
        Some(key) => Err(format!(
            "the payload has a field a credential does not sign: `{key}`"
        )),
    }
}

fn read_ucan(bytes: &[u8], issuer: &str) -> Result<Payload, String> {
    let Ipld::Map(mut envelope) = cbor(bytes)? else {
        return Err("the payload is not a UCAN".into());
    };
    let header = bytes_of(&mut envelope, "h")?;
    if header != ED25519_DAG_CBOR {
        return Err(format!(
            "the payload is not for an Ed25519 signature over DAG-CBOR: its header is {header:02x?}"
        ));
    }
    let (tag, payload) = match (envelope.pop_first(), envelope.is_empty()) {
        (Some((tag, Ipld::Map(payload))), true) => (tag, payload),
        _ => return Err("the payload is not a UCAN".into()),
    };
    match tag.as_str() {
        INVOCATION => read_invocation(payload, issuer).map(Payload::Invocation),
        DELEGATION => read_delegation(payload, issuer).map(Payload::Delegation),
        _ => Err(format!("a credential does not sign a `{tag}`")),
    }
}

fn read_invocation(mut map: BTreeMap<String, Ipld>, issuer: &str) -> Result<Invocation, String> {
    issued_by(&mut map, issuer)?;
    let audience = optional(&mut map, "aud")
        .map(|value| match value {
            Ipld::String(text) => Ok(text),
            other => Err(format!("`aud` is {other:?}, not text")),
        })
        .transpose()?;
    let subject = text(&mut map, "sub")?;
    let command = text(&mut map, "cmd")?;
    let arguments = encode(&map.remove("args").ok_or("an invocation has `args`")?)?;
    let proofs = match map.remove("prf") {
        Some(Ipld::List(links)) => links
            .into_iter()
            .map(|cid| link(cid, "prf"))
            .collect::<Result<_, _>>()?,
        other => return Err(format!("`prf` is {other:?}, not a list")),
    };
    let cause = optional(&mut map, "cause")
        .map(|cid| link(cid, "cause"))
        .transpose()?;
    let issued_at = optional_integer(&mut map, "iat")?;
    let expiration = optional_integer(&mut map, "exp")?;
    let meta = optional_cbor(&mut map, "meta")?;
    let nonce = bytes_of(&mut map, "nonce")?;
    nothing_else(map)?;
    Ok(Invocation {
        audience,
        subject,
        command,
        arguments,
        proofs,
        cause,
        issued_at,
        expiration,
        meta,
        nonce,
    })
}

fn read_delegation(mut map: BTreeMap<String, Ipld>, issuer: &str) -> Result<Delegation, String> {
    issued_by(&mut map, issuer)?;
    let audience = text(&mut map, "aud")?;
    let subject = match map.remove("sub") {
        Some(Ipld::String(subject)) => Some(subject),
        Some(Ipld::Null) => None,
        other => return Err(format!("`sub` is {other:?}, not text or null")),
    };
    let command = text(&mut map, "cmd")?;
    let policy = encode(&map.remove("pol").ok_or("a delegation has `pol`")?)?;
    let expiration = optional_integer(&mut map, "exp")?;
    let not_before = optional_integer(&mut map, "nbf")?;
    let meta = optional_cbor(&mut map, "meta")?;
    let nonce = bytes_of(&mut map, "nonce")?;
    nothing_else(map)?;
    Ok(Delegation {
        audience,
        subject,
        command,
        policy,
        expiration,
        not_before,
        meta,
        nonce,
    })
}

fn write_ucan(tag: &str, payload: BTreeMap<String, Ipld>) -> Result<Vec<u8>, String> {
    let envelope = BTreeMap::from([
        ("h".to_owned(), Ipld::Bytes(ED25519_DAG_CBOR.to_vec())),
        (tag.to_owned(), Ipld::Map(payload)),
    ]);
    encode(&Ipld::Map(envelope))
}

fn some_integer(value: Option<u64>) -> Ipld {
    value.map_or(Ipld::Null, |value| Ipld::Integer(value.into()))
}

fn cid(bytes: &[u8]) -> Result<Ipld, String> {
    Cid::try_from(bytes)
        .map(Ipld::Link)
        .map_err(|error| error.to_string())
}

fn invocation_map(invocation: &Invocation, issuer: &str) -> Result<BTreeMap<String, Ipld>, String> {
    let mut map = BTreeMap::from([
        ("iss".to_owned(), Ipld::String(issuer.to_owned())),
        ("sub".to_owned(), Ipld::String(invocation.subject.clone())),
        ("cmd".to_owned(), Ipld::String(invocation.command.clone())),
        ("args".to_owned(), cbor(&invocation.arguments)?),
        (
            "prf".to_owned(),
            Ipld::List(
                invocation
                    .proofs
                    .iter()
                    .map(|proof| cid(proof))
                    .collect::<Result<_, _>>()?,
            ),
        ),
        ("exp".to_owned(), some_integer(invocation.expiration)),
        ("nonce".to_owned(), Ipld::Bytes(invocation.nonce.clone())),
    ]);
    if let Some(audience) = &invocation.audience {
        map.insert("aud".to_owned(), Ipld::String(audience.clone()));
    }
    if let Some(cause) = &invocation.cause {
        map.insert("cause".to_owned(), cid(cause)?);
    }
    if let Some(issued_at) = invocation.issued_at {
        map.insert("iat".to_owned(), Ipld::Integer(issued_at.into()));
    }
    if let Some(meta) = &invocation.meta {
        map.insert("meta".to_owned(), cbor(meta)?);
    }
    Ok(map)
}

fn delegation_map(delegation: &Delegation, issuer: &str) -> Result<BTreeMap<String, Ipld>, String> {
    let mut map = BTreeMap::from([
        ("iss".to_owned(), Ipld::String(issuer.to_owned())),
        ("aud".to_owned(), Ipld::String(delegation.audience.clone())),
        (
            "sub".to_owned(),
            delegation.subject.clone().map_or(Ipld::Null, Ipld::String),
        ),
        ("cmd".to_owned(), Ipld::String(delegation.command.clone())),
        ("pol".to_owned(), cbor(&delegation.policy)?),
        ("exp".to_owned(), some_integer(delegation.expiration)),
        ("nonce".to_owned(), Ipld::Bytes(delegation.nonce.clone())),
    ]);
    if let Some(not_before) = delegation.not_before {
        map.insert("nbf".to_owned(), Ipld::Integer(not_before.into()));
    }
    if let Some(meta) = &delegation.meta {
        map.insert("meta".to_owned(), cbor(meta)?);
    }
    Ok(map)
}

/// Reads the fields of a head after its tag: each of branch and issuer as
/// an 8-byte length and its UTF-8, the tree's 32 bytes, the edition as 8
/// bytes, and with a context, `0x01`, an 8-byte count, and each entry as an
/// origin's 32 bytes, its edition and its count, 8 bytes each.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let (taken, rest) = self
            .0
            .split_at_checked(count)
            .ok_or("the head ends early")?;
        self.0 = rest;
        Ok(taken)
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn array(&mut self) -> Result<[u8; 32], String> {
        Ok(self.take(32)?.try_into().expect("32 bytes"))
    }

    fn text(&mut self) -> Result<String, String> {
        let length = usize::try_from(self.u64()?).map_err(|_| "a head's text is too long")?;
        String::from_utf8(self.take(length)?.to_vec()).map_err(|error| error.to_string())
    }
}

fn read_head(bytes: &[u8], issuer: &str) -> Result<Head, String> {
    let mut cursor = Cursor(bytes);
    let branch = cursor.text()?;
    let named = cursor.text()?;
    if named != issuer {
        return Err(format!("the head is issued by {named}, not {issuer}"));
    }
    let tree = cursor.array()?;
    let edition = cursor.u64()?;
    let context = match cursor.0 {
        [] => None,
        [0x01, ..] => {
            cursor.take(1)?;
            let count = cursor.u64()?;
            let watermarks = (0..count)
                .map(|_| {
                    Ok(Watermark {
                        origin: cursor.array()?,
                        edition: cursor.u64()?,
                        count: cursor.u64()?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Some(watermarks)
        }
        _ => return Err("the head has an unknown marker after its edition".into()),
    };
    if !cursor.0.is_empty() {
        return Err("the head has bytes after its context".into());
    }
    Ok(Head {
        branch,
        tree,
        edition,
        context,
    })
}

fn write_head(head: &Head, issuer: &str) -> Vec<u8> {
    let mut bytes = HEAD_TAG.to_vec();
    for field in [head.branch.as_str(), issuer] {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes.extend_from_slice(&head.tree);
    bytes.extend_from_slice(&head.edition.to_be_bytes());
    if let Some(context) = &head.context {
        bytes.push(0x01);
        bytes.extend_from_slice(&(context.len() as u64).to_be_bytes());
        for watermark in context {
            bytes.extend_from_slice(&watermark.origin);
            bytes.extend_from_slice(&watermark.edition.to_be_bytes());
            bytes.extend_from_slice(&watermark.count.to_be_bytes());
        }
    }
    bytes
}

fn read_record(bytes: &[u8], issuer: &str) -> Result<RevisionRecord, String> {
    let Ipld::Map(mut map) = cbor(bytes)? else {
        return Err("the revision record is not a map".into());
    };
    issued_by_field(&mut map, "issuer", issuer)?;
    match map.remove("signature") {
        Some(Ipld::Bytes(signature)) if signature.is_empty() => {}
        _ => return Err("a revision record is signed with its signature empty".into()),
    }
    Ok(RevisionRecord {
        body: encode(&Ipld::Map(map))?,
    })
}

fn issued_by_field(
    map: &mut BTreeMap<String, Ipld>,
    key: &str,
    issuer: &str,
) -> Result<(), String> {
    let named = text(map, key)?;
    if named == issuer {
        Ok(())
    } else {
        Err(format!("the record is issued by {named}, not {issuer}"))
    }
}

fn write_record(record: &RevisionRecord, issuer: &str) -> Result<Vec<u8>, String> {
    let Ipld::Map(mut map) = cbor(&record.body)? else {
        return Err("a revision record's body is a map".into());
    };
    map.insert("issuer".to_owned(), Ipld::String(issuer.to_owned()));
    map.insert("signature".to_owned(), Ipld::Bytes(Vec::new()));
    let mut bytes = RECORD_TAG.to_vec();
    bytes.extend(encode(&Ipld::Map(map))?);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use dialog_credentials::ed25519::external::{ExternalSign, ExternalSigning, SigningKey};
    use dialog_credentials::{Ed25519Signer, Ed25519SigningKey, Signer};
    use dialog_ucan_core::promise::Promised;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, InvocationBuilder};
    use dialog_varsig::Principal as _;
    use ed25519_dalek::Signer as _;

    use super::*;

    /// Signs with a key of its own, and keeps each payload it is handed, as
    /// a guest kernel's signer is handed them.
    #[derive(Clone)]
    struct Recording {
        key: ed25519_dalek::SigningKey,
        seen: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl ExternalSign for Recording {
        fn sign(&self, payload: &[u8]) -> ExternalSigning {
            self.seen.lock().unwrap().push(payload.to_vec());
            let signature = self.key.sign(payload).to_bytes();
            Box::pin(async move { Ok(signature) })
        }
    }

    fn recording(seed: u8) -> (Signer, Recording) {
        let recording = Recording {
            key: ed25519_dalek::SigningKey::from_bytes(&[seed; 32]),
            seen: Arc::default(),
        };
        let key = SigningKey::new(recording.key.verifying_key(), recording.clone());
        let signer = Signer::from(Ed25519Signer::from(Ed25519SigningKey::from(key)));
        (signer, recording)
    }

    /// Reads what Dialog DB handed the signer, and writes it back the same.
    fn round_trip(recording: &Recording, issuer: &str) -> Payload {
        let seen = recording.seen.lock().unwrap();
        let bytes = seen.last().expect("Dialog DB asked for a signature");
        let payload = read(bytes, issuer).unwrap();
        assert_eq!(&write(&payload, issuer).unwrap(), bytes);
        payload
    }

    #[dialog_common::test]
    async fn it_writes_a_delegation_as_dialog_signs_it() {
        let (signer, recording) = recording(1);
        let issuer = signer.did().to_string();
        let audience = Ed25519Signer::import(&[2; 32]).await.unwrap().did();
        let subject = Ed25519Signer::import(&[3; 32]).await.unwrap().did();

        DelegationBuilder::new()
            .issuer(signer.clone())
            .audience(&audience)
            .subject(Subject::Specific(subject.clone()))
            .command(vec!["memory".into(), "read".into()])
            .try_build()
            .await
            .unwrap();
        let Payload::Delegation(delegation) = round_trip(&recording, &issuer) else {
            panic!("a delegation reads as one");
        };
        assert_eq!(delegation.audience, audience.to_string());
        assert_eq!(delegation.subject, Some(subject.to_string()));
        assert_eq!(delegation.command, "/memory/read");

        DelegationBuilder::new()
            .issuer(signer)
            .audience(&audience)
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let Payload::Delegation(powerline) = round_trip(&recording, &issuer) else {
            panic!("a delegation reads as one");
        };
        assert_eq!(powerline.subject, None);
    }

    #[dialog_common::test]
    async fn it_writes_an_invocation_as_dialog_signs_it() {
        let (signer, recording) = recording(1);
        let issuer = signer.did().to_string();
        let subject = Ed25519Signer::import(&[3; 32]).await.unwrap().did();

        InvocationBuilder::new()
            .issuer(signer)
            .audience(&subject)
            .subject(&subject)
            .command(vec!["archive".into(), "get".into()])
            .arguments(BTreeMap::from([(
                "key".to_owned(),
                Promised::String("value".into()),
            )]))
            .proofs(vec![])
            .try_build()
            .await
            .unwrap();
        let Payload::Invocation(invocation) = round_trip(&recording, &issuer) else {
            panic!("an invocation reads as one");
        };
        assert_eq!(invocation.subject, subject.to_string());
        assert_eq!(invocation.command, "/archive/get");
    }

    #[dialog_common::test]
    async fn it_reads_a_head_and_a_record_as_dialog_writes_them() {
        use dialog_artifacts::history::RevisionRecord as DialogRecord;
        use dialog_capability::history::{Context, Edition, Origin, Version};
        use dialog_capability::identity::{Entity, Revision, TreeReference};

        let did = Ed25519Signer::import(&[1; 32]).await.unwrap().did();
        let issuer = did.to_string();
        let same = |bytes: &[u8]| write(&read(bytes, &issuer).unwrap(), &issuer).unwrap();
        let branch: Entity = "branch:2b3c".parse().unwrap();

        let mut head = Revision::new(TreeReference::from([7; 32]), branch.clone(), did);
        let bare = head.payload();
        assert_eq!(same(&bare), bare);

        let mut context = Context::new();
        context.record(Version {
            origin: Origin([9; 32]),
            edition: Edition::new(4),
        });
        head.context = Some(context);
        let full = head.payload();
        let Payload::Head(read_head) = read(&full, &issuer).unwrap() else {
            panic!("a head reads as one");
        };
        assert_eq!(read_head.context.as_ref().map(Vec::len), Some(1));
        assert_eq!(same(&full), full);

        let record = DialogRecord {
            format: 0,
            branch,
            issuer: issuer.clone(),
            authority: issuer.clone(),
            parents: vec![Version {
                origin: Origin([3; 32]),
                edition: Edition::new(2),
            }],
            skips: vec![],
            signature: vec![],
        };
        let bytes = record.payload().unwrap();
        assert_eq!(same(&bytes), bytes);
    }

    #[test]
    fn it_writes_a_head_as_dialog_signs_it() {
        let issuer = "did:key:z6Mkissuer";
        let head = Head {
            branch: "branch:abc".into(),
            tree: [7; 32],
            edition: 42,
            context: Some(vec![Watermark {
                origin: [9; 32],
                edition: 41,
                count: 3,
            }]),
        };
        let bytes = write(&Payload::Head(head.clone()), issuer).unwrap();
        assert_eq!(read(&bytes, issuer).unwrap(), Payload::Head(head));
        assert!(read(&bytes, "did:key:z6Mkother").is_err());
    }

    #[test]
    fn it_refuses_bytes_of_no_kind() {
        assert!(read(b"anything at all", "did:key:z6Mkissuer").is_err());
    }
}
