//! `tonk log`: the branch's committed history.
//!
//! Dialog records two things about every commit inside the branch's
//! own tree: a signed revision record (the DAG edge to the revisions
//! it built on) and, in the history region, every claim the commit
//! asserted or retracted. Both ride the tree, so a pulled space
//! carries its history with it and nothing here is reconstructed
//! from the archive. The revision walk is [`Branch::log`]; the claim
//! rows are read through [`Branch::history`].
//!
//! [`Branch::log`]: dialog_repository::Branch::log
//! [`Branch::history`]: dialog_repository::Branch::history

use std::collections::{BTreeSet, HashMap};

use dialog_artifacts::Entity;
use dialog_artifacts::history::{
    Edition, HistorySelector, Origin, Record, RevisionRecord, Version,
};
use futures_util::StreamExt as _;
use serde::Serialize;

use crate::data_ops::{self, DataOpError};
use crate::site::TonkSite;
use crate::{Coded, ExitCode, schema, views};

/// One row of `tonk log`: a revision as its signed record describes it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRow {
    /// `<edition>@<origin>` — the form `--revision` takes back.
    pub version: String,
    /// Causal depth: one more than the deepest parent.
    pub edition: u64,
    /// Operator DID that minted the revision.
    pub issuer: String,
    /// Profile DID the issuer acted for.
    pub authority: String,
    /// The versions this revision built on; empty for the first.
    pub parents: Vec<String>,
}

impl RevisionRow {
    fn new(version: Version, record: &RevisionRecord) -> Self {
        Self {
            version: version.to_string(),
            edition: version.edition.value(),
            issuer: record.issuer.clone(),
            authority: record.authority.clone(),
            parents: record.parents.iter().map(ToString::to_string).collect(),
        }
    }
}

/// One row of `tonk log <concept>`: a claim as a revision recorded it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRow {
    /// The revision that recorded the claim.
    pub version: String,
    /// That revision's edition.
    pub edition: u64,
    /// `assert` or `retract`.
    pub change: &'static str,
    /// The entity the claim is about.
    pub entity: String,
    /// The entity's bookmark, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The attribute URI.
    pub attribute: String,
    /// The value, as JSON.
    pub value: serde_json::Value,
    /// Notation form of the value, for the text listing.
    #[serde(skip)]
    pub value_text: String,
    /// The earlier claims on the same entity and attribute this one
    /// replaced or withdrew.
    pub supersedes: Vec<String>,
}

impl ChangeRow {
    fn new(version: Version, record: &Record, names: &HashMap<Entity, String>) -> Self {
        let claim = record.claim();
        Self {
            version: version.to_string(),
            edition: version.edition.value(),
            change: if record.is_assertion() {
                "assert"
            } else {
                "retract"
            },
            entity: claim.of.to_string(),
            name: names.get(&claim.of).cloned(),
            attribute: claim.the.to_string(),
            value: serde_json::to_value(claim.is.untagged()).unwrap_or(serde_json::Value::Null),
            value_text: schema::value_to_notation(&claim.is),
            supersedes: claim
                .cause
                .versions()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

/// Which recorded claims `tonk log <concept>` lists.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    /// Only what this one revision recorded.
    pub revision: Option<Version>,
    /// Only claims on these relations — a concept's fields, each as
    /// `domain/name`, or `domain/` for a keyed collection.
    pub attributes: Option<BTreeSet<String>>,
    /// Only claims on this entity.
    pub entity: Option<Entity>,
}

/// Why a log could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// The concept argument names nothing on the branch.
    #[error(transparent)]
    Concept(#[from] DataOpError),
    /// The entity argument is neither a bookmark nor an entity URI.
    #[error("no entity named '{0}'; pass a bookmark or a `did:key:…` URI")]
    UnknownEntity(String),
    /// The revision argument is not in the form `tonk log` prints.
    #[error("invalid revision '{0}': expected <edition>@<origin> as `tonk log` prints it")]
    InvalidVersion(String),
    /// Anything else — dialog I/O, a failed read.
    #[error("{0}")]
    Io(String),
}

impl Coded for LogError {
    fn exit_code(&self) -> ExitCode {
        match self {
            LogError::Concept(error) => error.exit_code(),
            LogError::UnknownEntity(_) | LogError::InvalidVersion(_) => ExitCode::ParseError,
            LogError::Io(_) => ExitCode::IoError,
        }
    }
}

fn io(error: impl std::fmt::Display) -> LogError {
    LogError::Io(error.to_string())
}

/// The newest `limit` revisions of `main`, newest first, each before
/// any of its ancestors.
pub async fn revisions(site: &TonkSite, limit: usize) -> Result<Vec<RevisionRow>, LogError> {
    let session = site.branch().await.map_err(io)?;
    let entries = session
        .handle()
        .log(&site.operator, limit)
        .await
        .map_err(io)?;
    Ok(entries
        .into_iter()
        .map(|(version, record)| RevisionRow::new(version, &record))
        .collect())
}

/// Parse a version back from the `<edition>@<origin>` form it prints as.
pub fn parse_version(text: &str) -> Result<Version, LogError> {
    use base58::FromBase58 as _;

    let invalid = || LogError::InvalidVersion(text.to_owned());
    let (edition, origin) = text.split_once('@').ok_or_else(invalid)?;
    let edition: u64 = edition.parse().map_err(|_| invalid())?;
    let bytes: [u8; 32] = origin
        .from_base58()
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    Ok(Version::new(Origin::from(bytes), Edition::new(edition)))
}

/// Resolve the command's arguments into a [`Filter`]: the concept's
/// field attributes, the entity behind a bookmark or URI, the parsed
/// revision.
pub async fn filter_for(
    site: &TonkSite,
    concept: Option<&str>,
    entity: Option<&str>,
    revision: Option<&str>,
) -> Result<Filter, LogError> {
    let attributes = match concept {
        Some(name) => {
            let info = data_ops::require_concept(site, name).await?;
            // A field names one relation, `domain/name`; a keyed
            // collection names a whole domain, kept as the `domain/`
            // prefix every entry's relation starts with.
            Some(
                info.descriptor
                    .with()
                    .iter()
                    .map(|(_, field)| match field.name() {
                        Some(name) => format!("{}/{name}", field.domain()),
                        None => format!("{}/", field.domain()),
                    })
                    .collect(),
            )
        }
        None => None,
    };
    let entity = match entity {
        Some(reference) => Some(match reference.parse::<Entity>() {
            Ok(entity) => entity,
            Err(_) => views::entity_for_name(site, reference)
                .await
                .map_err(io)?
                .ok_or_else(|| LogError::UnknownEntity(reference.to_owned()))?,
        }),
        None => None,
    };
    let revision = revision.map(parse_version).transpose()?;
    Ok(Filter {
        revision,
        attributes,
        entity,
    })
}

/// Whether a concept's field set selects `relation`: an exact
/// `domain/name` entry, or a `domain/` entry for a keyed collection.
fn selects(attributes: &BTreeSet<String>, relation: &str) -> bool {
    attributes.contains(relation)
        || attributes
            .iter()
            .any(|pattern| pattern.ends_with('/') && relation.starts_with(pattern))
}

/// The claims matching `filter`, grouped by revision newest first and
/// within a revision by entity then attribute; at most `limit`
/// revisions' worth.
pub async fn changes(
    site: &TonkSite,
    filter: &Filter,
    limit: usize,
) -> Result<Vec<ChangeRow>, LogError> {
    let names = schema::name_claims_by_entity(site).await.map_err(io)?;
    let session = site.branch().await.map_err(io)?;
    let history = session.handle().history(&site.operator);
    let selector = match filter.revision {
        Some(version) => HistorySelector::At(version),
        None => HistorySelector::All,
    };
    let records = history.select(selector);
    tokio::pin!(records);

    let mut rows: Vec<(Version, ChangeRow)> = Vec::new();
    while let Some(entry) = records.next().await {
        let (version, record) = entry.map_err(io)?;
        let claim = record.claim();
        if let Some(attributes) = &filter.attributes
            && !selects(attributes, claim.the.as_str())
        {
            continue;
        }
        if let Some(entity) = &filter.entity
            && claim.of != *entity
        {
            continue;
        }
        rows.push((version, ChangeRow::new(version, &record, &names)));
    }
    rows.sort_by(|(a, left), (b, right)| {
        b.cmp(a)
            .then_with(|| left.entity.cmp(&right.entity))
            .then_with(|| left.attribute.cmp(&right.attribute))
    });

    let mut kept = Vec::with_capacity(rows.len());
    let mut last: Option<Version> = None;
    let mut revisions = 0usize;
    for (version, row) in rows {
        if last != Some(version) {
            revisions += 1;
            if revisions > limit {
                break;
            }
            last = Some(version);
        }
        kept.push(row);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_parses_the_form_a_version_prints_as() {
        let version = Version::new(Origin::from([7u8; 32]), Edition::new(12));
        let parsed = parse_version(&version.to_string()).unwrap();
        assert_eq!(parsed, version);
    }

    #[dialog_common::test]
    fn it_rejects_a_version_without_an_origin() {
        assert!(matches!(
            parse_version("12"),
            Err(LogError::InvalidVersion(text)) if text == "12"
        ));
    }

    #[dialog_common::test]
    fn it_selects_a_field_exactly_and_a_keyed_collection_by_domain() {
        let fields: BTreeSet<String> =
            ["xyz.tonk.task/title".to_owned(), "xyz.tonk.tag/".to_owned()]
                .into_iter()
                .collect();
        assert!(selects(&fields, "xyz.tonk.task/title"));
        assert!(!selects(&fields, "xyz.tonk.task/done"));
        assert!(selects(&fields, "xyz.tonk.tag/urgent"));
        assert!(!selects(&fields, "xyz.tonk.tagline/x"));
    }

    #[dialog_common::test]
    fn it_rejects_an_origin_of_the_wrong_length() {
        assert!(matches!(
            parse_version("12@abc"),
            Err(LogError::InvalidVersion(_))
        ));
    }
}
