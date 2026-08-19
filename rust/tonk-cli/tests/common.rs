//! Shared fixtures for tonk's integration tests. Each test
//! lands its own tempdir-rooted site with a unique profile name
//! so parallel runs don't trip over the user's real
//! `~/Library/Application Support/dialog/` profile or each other.

#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;
use dialog_effects::storage::Directory;
use tempfile::TempDir;
use tonk_cli::eval::{self, Source};
use tonk_cli::site::{SiteConfig, TonkSite};

pub struct TestSite {
    pub site: TonkSite,
    pub config: SiteConfig,
    pub parent: PathBuf,
    pub tmp: TempDir,
}

impl TestSite {
    pub async fn new() -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = isolated_config(&parent)?;
        let site = TonkSite::init_with(&parent, config.clone()).await?;
        Ok(Self {
            site,
            config,
            parent,
            tmp,
        })
    }

    /// The base58 reference of this branch's current tree root.
    ///
    /// A resolver selects by content address, so `tree/*` needs this
    /// bound before it can run. Real documents reach it by joining
    /// through the branch revision; tests that are pinning resolver
    /// behavior rather than the join read it directly.
    pub async fn tree_root(&self) -> Result<String> {
        let session = self.site.branch().await?;
        let revision = session
            .handle()
            .revision()
            .ok_or_else(|| anyhow::anyhow!("branch has no revision yet"))?;
        // `TreeReference` displays as `#<base58>`; the resolver takes
        // the bare reference.
        Ok(revision.tree.to_string().trim_start_matches('#').to_owned())
    }

    pub async fn eval_inline(&self, doc: &str) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(
            &self.site,
            Source::Inline(doc.to_string()),
            eval::Options::default(),
        )
        .await
    }

    pub async fn eval_inline_with(
        &self,
        doc: &str,
        options: eval::Options,
    ) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(&self.site, Source::Inline(doc.to_string()), options).await
    }
}

/// Build a [`SiteConfig`] whose profile lives entirely inside
/// `parent` so the test never touches the user's data dir.
pub fn isolated_config(parent: &std::path::Path) -> Result<SiteConfig> {
    let profile_dir = parent.join("_profile");
    std::fs::create_dir_all(&profile_dir)?;
    let suffix: u64 = rand::random();
    Ok(SiteConfig {
        profile_name: format!("tonk-test-{suffix:x}"),
        profile_directory: Directory::At(profile_dir.to_string_lossy().into_owned()),
        require_account: false,
    })
}

#[cfg(feature = "integration-tests")]
pub struct AccountFixture {
    /// A site this profile created before the account existed, so its
    /// repository authority reaches no account root.
    pub pre_account_site: TonkSite,
    pub server: tonk_account_service::helpers::AccountServer,
    pub profile: dialog_operator::Profile,
    pub store: tonk_cli::spot::SpotStore,
    pub link: dialog_ucan_core::DelegationChain,
    pub config: SiteConfig,
    pub descriptor: Vec<u8>,
    root_prf: [u8; 32],
    pub tmp: TempDir,
}

#[cfg(feature = "integration-tests")]
impl AccountFixture {
    pub async fn new() -> Result<Self> {
        // A dead remote: fixtures that never sync the account repository do
        // not need one, and a live URL would make them depend on a service
        // they have no use for.
        Self::with_account_remote("http://127.0.0.1:9/ucan/").await
    }

    /// A fixture whose account repository points at a REAL remote, for tests
    /// that sync the account between devices.
    pub async fn with_account_remote(remote: &str) -> Result<Self> {
        let test = TestSite::new().await?;
        let profile = test.site.profile.clone();
        let store = tonk_cli::spot::SpotStore::at(test.parent.join("state"));
        let server = tonk_account_service::helpers::AccountServer::start().await;
        let root_prf = [77; 32];
        let root = tonk_identity::derive::derive_root_signer(&root_prf).await?;
        let link =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &profile.did()).await?;
        let email = "person@example.com".to_string();
        let ceremony = tonk_identity::ceremony::create_account(
            root,
            email,
            "fixture-credential".to_string(),
            profile.did(),
            "fixture-device".to_string(),
            hex::encode(link.to_bytes()?),
            remote.to_string(),
            None,
        )
        .await?;
        reqwest::Client::new()
            .post(format!("{}/accounts", server.endpoint))
            .body(hex::decode(&ceremony.invocation_hex)?)
            .send()
            .await?
            .error_for_status()?;
        let descriptor = hex::decode(
            ceremony
                .descriptor_hex
                .as_deref()
                .expect("creation establishes a descriptor"),
        )?;
        tonk_cli::account::attach_for_integration_test(
            &profile,
            &test.site.operator,
            test.config.clone(),
            &server.endpoint,
            "fixture-credential",
            link.clone(),
            &descriptor,
        )
        .await?;

        // Mark the descriptor trusted, so the fixture models a device that
        // has hydrated its account rather than one that has only linked it.
        // Without this the account reads as unhydrated and nothing will mount
        // its repository.
        let validated = tonk_account::AccountRepositoryDescriptorV1::validate(&descriptor).await?;
        profile
            .credential()
            .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
            .save(validated.content_hash().to_vec())
            .perform(&test.site.operator)
            .await?;

        Ok(Self {
            pre_account_site: test.site,
            server,
            profile,
            store,
            link,
            config: test.config,
            descriptor,
            root_prf,
            tmp: test.tmp,
        })
    }

    pub async fn backup(
        &self,
        space_seed: u8,
        name: Option<&str>,
        remote_url: Option<&str>,
    ) -> Result<(dialog_varsig::Did, tonk_account::backup::AccountSpotBackup)> {
        use dialog_ucan_core::subject::Subject;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain};
        use dialog_varsig::Principal as _;

        let root = tonk_identity::derive::derive_root_signer(&self.root_prf).await?;
        let space = dialog_credentials::Ed25519Signer::import(&[space_seed; 32]).await?;
        let subject = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(space)
            .audience(&root.did())
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await?;
        let chain = DelegationChain::new(delegation);
        let backup = tonk_account::backup::AccountSpotBackup {
            chain_hex: hex::encode(chain.to_bytes()?),
            remote_url: remote_url.map(str::to_string),
            revocation_url: None,
            name: name.map(str::to_string),
        };
        Ok((subject, backup))
    }

    pub async fn put(&self, backup: &tonk_account::backup::AccountSpotBackup) -> Result<String> {
        use dialog_ucan_core::promise::Promised;

        let bytes = serde_json::to_vec(backup)?;
        let body = tonk_identity::request::build_device_invocation(
            self.profile.signer().signer().clone(),
            &self.link,
            vec!["account".into(), "chain".into(), "put".into()],
            [("chain".to_string(), Promised::String(hex::encode(bytes)))]
                .into_iter()
                .collect(),
        )
        .await?;
        let response = reqwest::Client::new()
            .post(format!("{}/chains/put", self.server.endpoint))
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json::<serde_json::Value>().await?["key"]
            .as_str()
            .unwrap()
            .to_string())
    }
}

/// Notation declaration of the `task-title` and `task-done`
/// attributes — used by every test that needs a task schema.
pub const ATTRIBUTE_DECL: &str = r#"
attribute!: &task-title
  description: "task title"
  the:         xyz.tonk.task/title
  as:          text
  cardinality: one

attribute!: &task-done
  description: "task done flag"
  the:         xyz.tonk.task/done
  as:          boolean
  cardinality: one
"#;

/// Notation declaration of a `task` concept referencing the
/// attributes above.
pub const CONCEPT_DECL: &str = r#"
concept!: &task
  description: "a task"
  with:
    title: task-title
    done:  task-done
"#;

/// Attributes for the cardinality-many lock tests: a one-cardinality
/// `body` and a many-cardinality `tag`.
pub const NOTE_ATTRIBUTE_DECL: &str = r#"
attribute!: &note-body
  description: "note body"
  the:         xyz.tonk.note/body
  as:          text
  cardinality: one

attribute!: &note-tag
  description: "a tag on a note"
  the:         xyz.tonk.note/tag
  as:          text
  cardinality: many
"#;

/// A `note` concept referencing the attributes above — one required
/// one-cardinality field plus one required many-cardinality field.
pub const NOTE_CONCEPT_DECL: &str = r#"
concept!: &note
  description: "a tagged note"
  with:
    body: note-body
    tag:  note-tag
"#;

/// Seed schema for HTML views: a `text/html`-bound attribute and
/// a `view` concept that uses it. Pasted at the top of any test
/// that needs `view!` heads. Shared with [`tonk_cli::guide`] so the
/// agent-facing reference matches what the tests exercise.
pub const VIEW_DECL: &str = r#"
attribute!: &html-body
  description: "HTML body of a tonk-authored view"
  the:         "text/html"
  as:          text
  cardinality: many

concept!: &view
  description: "An HTML view, served via the host route"
  with:
    body: html-body
"#;

/// A stand-in for the browser authorization page.
///
/// The real page runs a passkey ceremony; this holds the account key
/// directly. Everything after the ceremony is identical — mint the
/// `account → device` grant, pair it with the descriptor, and POST it
/// base64-encoded to the `callback` the CLI passed. That contract is what
/// the CLI depends on, so exercising it here pins the command's whole
/// receiving half without a browser. The ceremony itself is covered by the
/// e2e suite, which drives a real virtual authenticator.
#[cfg(feature = "integration-tests")]
pub struct AuthorizingPage {
    /// URL to hand the CLI as its `--via` page.
    pub url: String,
    _handle: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "integration-tests")]
pub async fn authorizing_page(
    root: dialog_credentials::Ed25519Signer,
    remote: String,
) -> Result<AuthorizingPage> {
    use axum::extract::{Query, State};
    use axum::response::Html;
    use axum::routing::get;
    use base64::Engine as _;
    use std::collections::HashMap;

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    // Answering the GET is what the user approving in the browser amounts to.
    async fn approve(
        State((root, remote)): State<(dialog_credentials::Ed25519Signer, String)>,
        Query(params): Query<HashMap<String, String>>,
    ) -> Html<String> {
        let (Some(audience), Some(callback)) = (params.get("audience"), params.get("callback"))
        else {
            return Html("missing audience or callback".to_owned());
        };
        let Ok(device_did) = audience.parse() else {
            return Html("unparseable audience".to_owned());
        };
        let Ok(authorized) =
            tonk_identity::ceremony::authorize_device(root, device_did, &remote).await
        else {
            return Html("ceremony failed".to_owned());
        };
        let payload = serde_json::json!({
            "delegationHex": authorized.delegation_hex,
            "descriptorHex": authorized.descriptor_hex,
            "credentialId": authorized.root_did,
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
        let _ = reqwest::Client::new()
            .post(callback)
            .form(&[("authorize", encoded)])
            .send()
            .await;
        Html("approved".to_owned())
    }

    let app = axum::Router::new()
        .route("/", get(approve))
        .with_state((root, remote));
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(AuthorizingPage {
        url,
        _handle: handle,
    })
}
