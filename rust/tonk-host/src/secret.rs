//! Sealing a command's secret fields before a claim leaves the page.
//!
//! An attribute declared `as: secret` holds bytes sealed to the worker
//! (see `tonk_schema::secret`). Page JS and YAML views know nothing of
//! that: an event fills such a field with plain text, the way it fills
//! any other. Every claim a page makes, whether a view's event or
//! `window.tonk.transact` in a portal, reaches the worker through this
//! host's `tonk-claim` handler, so this is where the text is sealed.
//!
//! A field is sealed when the claim's descriptor types it as bytes, its
//! value is text, and its attribute is recorded as sealed on the branch
//! (`tonk.dialog.attribute/sealed`). It is sealed to the recipient the
//! worker publishes on the overlay of the profile's meta branch. Anything
//! that cannot be sealed is sent as it is, and the worker refuses it:
//! plain text where a secret belongs never runs.

use std::collections::BTreeSet;

use serde_json::Value;
use tonk_identity::sealed::RecipientKey;

/// The attribute recording an attribute as sealed.
pub const SEALED_MARKER: &str = "tonk.dialog.attribute/sealed";

/// The attribute the worker publishes its recipient under.
pub const RECIPIENT: &str = "xyz.tonk.command-secret/recipient";

/// The entity the recipient is published on.
pub const RECIPIENT_ENTITY: &str = "tonk:command-secret";

/// Whether `as` names the bytes type, in any spelling a descriptor uses.
fn is_bytes(kind: &str) -> bool {
    matches!(kind, "bytes:" | "Bytes" | "bytes")
}

/// A field of one claim that may need sealing: typed as bytes, bound to
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The claim's index in the request.
    pub claim: usize,
    /// The field name in the claim's parameters.
    pub field: String,
    /// The attribute the field is stored under.
    pub relation: String,
}

/// The fields of `request` (a `TransactRequest` as JSON) that may need
/// sealing. Empty for almost every claim, so a page pays nothing for
/// sealing unless it sends a secret.
pub fn candidates(request: &Value) -> Vec<Candidate> {
    let Some(claims) = request.get("claims").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (index, claim) in claims.iter().enumerate() {
        let application = &claim["application"];
        let parameters = &application["parameters"];
        let concept = &application["predicate"]["concept"];
        for fields in [&concept["with"], &concept["maybe"]] {
            let Some(fields) = fields.as_object() else {
                continue;
            };
            for (field, attribute) in fields {
                let typed_bytes = attribute
                    .get("as")
                    .and_then(Value::as_str)
                    .is_some_and(is_bytes);
                let text = parameters.get(field).is_some_and(Value::is_string);
                let relation = attribute.get("the").and_then(Value::as_str);
                if let (true, true, Some(relation)) = (typed_bytes, text, relation) {
                    found.push(Candidate {
                        claim: index,
                        field: field.clone(),
                        relation: relation.to_owned(),
                    });
                }
            }
        }
    }
    found
}

/// Seal each candidate of `request` whose attribute is in `sealed` to
/// `recipient`, replacing its text with the sealed bytes. Returns how
/// many were sealed; a candidate whose attribute is not sealed is left
/// as it is.
pub fn seal(
    request: &mut Value,
    candidates: &[Candidate],
    sealed: &BTreeSet<String>,
    recipient: &RecipientKey,
) -> Result<usize, String> {
    let mut count = 0;
    for candidate in candidates {
        if !sealed.contains(&candidate.relation) {
            continue;
        }
        let slot = &mut request["claims"][candidate.claim]["application"]["parameters"]
            [candidate.field.as_str()];
        let Some(text) = slot.as_str() else {
            continue;
        };
        let bytes = recipient
            .seal_secret(text.as_bytes(), &candidate.relation)
            .map_err(|error| format!("could not seal {}: {error}", candidate.field))?;
        // The wire shape a bytes value takes in a transact request: an
        // array of unsigned bytes, as `tonk-display` writes one.
        *slot = Value::Array(bytes.into_iter().map(Value::from).collect());
        count += 1;
    }
    Ok(count)
}

/// The relations a `/query` answer over [`sealed_query`] names.
pub fn sealed_relations(rows: &Value) -> BTreeSet<String> {
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let fields = row.get("fields").unwrap_or(row);
            let sealed = fields.get("sealed").and_then(Value::as_bool) == Some(true);
            let id = fields.get("id").and_then(Value::as_str)?;
            sealed.then(|| id.to_owned())
        })
        .collect()
}

/// The `/query` body listing every attribute recorded as sealed, by
/// relation.
pub fn sealed_query() -> Value {
    serde_json::json!({
        "predicate": { "with": {
            "id": { "the": "db.attribute/id", "as": "Text", "cardinality": "one" },
            "sealed": { "the": SEALED_MARKER, "as": "Boolean", "cardinality": "one" }
        } },
        "terms": {
            "this": { "?": { "name": "this" } },
            "id": { "?": { "name": "id" } },
            "sealed": { "?": { "name": "sealed" } }
        }
    })
}

/// The `/query` body reading the published recipient.
pub fn recipient_query() -> Value {
    serde_json::json!({
        "predicate": { "with": {
            "recipient": { "the": RECIPIENT, "as": "Text", "cardinality": "one" }
        } },
        "terms": {
            "this": RECIPIENT_ENTITY,
            "recipient": { "?": { "name": "recipient" } }
        }
    })
}

/// The recipient a `/query` answer over [`recipient_query`] names.
pub fn recipient(rows: &Value) -> Option<RecipientKey> {
    let row = rows.as_array()?.first()?;
    let fields = row.get("fields").unwrap_or(row);
    let did: dialog_varsig::Did = fields.get("recipient")?.as_str()?.parse().ok()?;
    RecipientKey::try_from(&did).ok()
}

/// Seal the secret fields of a claim headed for `query_url`'s branch.
///
/// Reads which attributes are sealed off that branch and off the
/// profile's meta branch (where the profile library declares its
/// commands), and the recipient off meta. When either read fails, the
/// request goes as it is and the worker refuses its plain text.
#[cfg(target_arch = "wasm32")]
pub async fn seal_claim(mut request: Value, query_url: &str) -> Value {
    let found = candidates(&request);
    if found.is_empty() {
        return request;
    }
    const META: &str = "/api/repository/profile:tonk/branch/meta/query";
    let read = |url: String, body: Value| async move {
        let text = crate::http::post_json(&url, &body.to_string()).await.ok()?;
        serde_json::from_str::<Value>(&text).ok()
    };
    let Some(recipient) = read(META.into(), recipient_query())
        .await
        .as_ref()
        .and_then(recipient)
    else {
        tonk_common::log!("tonk-claim: no recipient to seal a secret to; sending it as given");
        return request;
    };
    let mut sealed = BTreeSet::new();
    for url in [query_url, META] {
        if let Some(rows) = read(url.to_owned(), sealed_query()).await {
            sealed.extend(sealed_relations(&rows));
        }
    }
    if let Err(error) = seal(&mut request, &found, &sealed, &recipient) {
        tonk_common::log!("tonk-claim: {error}");
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const SEALED_URL: &str = "xyz.tonk.command.join/sealed-url";

    /// A claim as a view's `on:join` event builds it: the url read off
    /// the event in plain text, into a field the descriptor types as
    /// bytes because it is declared `as: secret`.
    fn join_claim(url: &str) -> Value {
        serde_json::json!({ "claims": [{ "op": "assert", "application": {
            "predicate": { "kind": "transient", "concept": { "with": {
                "url": { "the": SEALED_URL, "as": "bytes:" }
            } } },
            "parameters": { "url": url }
        } }] })
    }

    fn recipient_of(seed: u8) -> RecipientKey {
        // A recipient is a public key; any 32 bytes clamp to one.
        let did: dialog_varsig::Did = {
            let mut bytes = vec![0xec, 0x01];
            bytes.extend_from_slice(&[seed; 32]);
            format!("did:key:z{}", bs58::encode(bytes).into_string())
                .parse()
                .unwrap()
        };
        RecipientKey::try_from(&did).unwrap()
    }

    #[dialog_common::test]
    fn it_seals_text_bound_to_a_sealed_attribute() {
        let mut request = join_claim("https://tonk.test/join#secret-seed");
        let found = candidates(&request);
        assert_eq!(
            found,
            vec![Candidate {
                claim: 0,
                field: "url".into(),
                relation: SEALED_URL.into()
            }]
        );
        let sealed = BTreeSet::from([SEALED_URL.to_owned()]);
        assert_eq!(seal(&mut request, &found, &sealed, &recipient_of(3)), Ok(1));
        let value = &request["claims"][0]["application"]["parameters"]["url"];
        let bytes: Vec<u8> = value
            .as_array()
            .expect("sealed bytes")
            .iter()
            .map(|byte| byte.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(
            RecipientKey::of_sealed(&bytes).unwrap(),
            recipient_of(3),
            "sealed to the published recipient"
        );
        assert!(!request.to_string().contains("secret-seed"));
    }

    #[dialog_common::test]
    fn it_leaves_text_alone_when_the_attribute_is_not_sealed() {
        let mut request = join_claim("not a secret");
        let found = candidates(&request);
        assert_eq!(
            seal(&mut request, &found, &BTreeSet::new(), &recipient_of(3)),
            Ok(0)
        );
        assert_eq!(
            request["claims"][0]["application"]["parameters"]["url"], "not a secret",
            "sent as given, for the worker to refuse"
        );
    }

    #[dialog_common::test]
    fn ordinary_claims_have_no_candidates() {
        let request = serde_json::json!({ "claims": [{ "op": "assert", "application": {
            "predicate": { "kind": "transient", "concept": { "with": {
                "name": { "the": "xyz.tonk.command.profile-rename/name", "as": "Text" },
                "blob": { "the": "xyz.tonk.thing/blob", "as": "Bytes" }
            } } },
            "parameters": { "name": "Ada", "blob": [1, 2, 3] }
        } }] });
        assert!(candidates(&request).is_empty());
    }

    #[dialog_common::test]
    fn it_reads_the_sealed_relations_and_the_recipient_off_query_rows() {
        let rows = serde_json::json!([
            { "this": "a", "fields": { "id": SEALED_URL, "sealed": true } },
            { "this": "b", "fields": { "id": "xyz.tonk.other/x", "sealed": false } }
        ]);
        assert_eq!(
            sealed_relations(&rows),
            BTreeSet::from([SEALED_URL.to_owned()])
        );
        let did = dialog_varsig::Principal::did(&recipient_of(4)).to_string();
        let rows =
            serde_json::json!([{ "this": RECIPIENT_ENTITY, "fields": { "recipient": did } }]);
        assert_eq!(recipient(&rows), Some(recipient_of(4)));
    }
}
