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
        provision_account_spaces: false,
        account_store: tonk_cli::space::SpaceStore::at(parent.join("_state")),
    })
}

#[cfg(feature = "integration-tests")]
pub struct AccountFixture {
    /// A site this profile created before the account existed, so its
    /// repository authority reaches no account root.
    pub pre_account_site: TonkSite,
    pub profile: dialog_operator::Profile,
    pub store: tonk_cli::space::SpaceStore,
    pub link: dialog_ucan_core::DelegationChain,
    pub config: SiteConfig,
    pub root_prf: [u8; 32],
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
        Self::build(remote, true).await
    }

    /// A fixture that has linked but never hydrated: the trusted-base
    /// marker is absent, exactly like a fresh device right after
    /// `tonk account login`.
    pub async fn unhydrated_with_account_remote(remote: &str) -> Result<Self> {
        Self::build(remote, false).await
    }

    async fn build(remote: &str, hydrated: bool) -> Result<Self> {
        let test = TestSite::new().await?;
        let profile = test.site.profile.clone();
        // One installation, one store: session state, the account repository,
        // and the space registry all live in the same place the site config
        // opens sites through.
        let store = test.config.account_store.clone();
        let root_prf = [77; 32];
        let root = dialog_credentials::Ed25519Signer::import(&root_prf).await?;
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
        tonk_cli::account::attach_for_integration_test(
            &profile,
            &test.site.operator,
            test.config.clone(),
            remote,
            "fixture-credential",
            link.clone(),
            remote,
        )
        .await?;

        // Save the root → device link through the account-store operator
        // too: hydration and the pull-side prover resolve authority
        // through it, and access certificates saved only through the
        // site operator are not in its reach.
        let account_operator =
            tonk_cli::account_state::credential_operator_for_store(&profile, &store).await?;
        profile
            .access()
            .save(dialog_ucan::UcanDelegation(link.clone()))
            .perform(&account_operator)
            .await?;

        if hydrated {
            // Mark the descriptor trusted, so the fixture models a device
            // that has hydrated its account rather than one that has only
            // linked it. Without this the account reads as unhydrated and
            // nothing will mount its repository.
            profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .save(link.issuer().as_str().as_bytes().to_vec())
                .perform(&test.site.operator)
                .await?;

            // Real accounts publish their encryption key: the ceremony
            // saves it with the root and the account sweep publishes the
            // fact. The fixture publishes directly, so account-backed
            // creates can seal their seeds into custody.
            use dialog_varsig::Principal as _;
            use tonk_schema::prelude::DidExt as _;
            let recipient = tonk_identity::envelope::AccountSecret::from_bytes(
                zeroize::Zeroizing::new(root_prf),
            )
            .secret()
            .did();
            let root_did: dialog_varsig::Did = ceremony.root_did.parse()?;
            if let Some(account) =
                tonk_cli::account_state::open_account_branch_in(&profile, &account_operator, &store)
                    .await?
            {
                account
                    .transaction()
                    .assert(tonk_schema::AccountSealedInbox::new(
                        root_did.this(),
                        recipient.this(),
                    ))
                    .commit()
                    .perform(&account_operator)
                    .await?;
            }
        }

        Ok(Self {
            pre_account_site: test.site,
            profile,
            store,
            link,
            config: test.config,
            root_prf,
            tmp: test.tmp,
        })
    }

    /// Enroll this fixture's account root with `access` and confirm its
    /// email, so the access service will serve and provision for it.
    ///
    /// The gate serves nothing for a customer that has not confirmed an
    /// email address, so any fixture that pushes, pulls, or provisions
    /// against a live access service comes through here first.
    pub async fn activate_with(
        &self,
        access: &tonk_access_service::helpers::AccessServiceAddress,
    ) -> Result<()> {
        let root = dialog_credentials::Ed25519Signer::import(&self.root_prf).await?;
        access.activate_customer(&root, "person@example.com").await
    }

    /// This fixture's account root signer.
    pub async fn root_signer(&self) -> Result<dialog_credentials::Ed25519Signer> {
        Ok(dialog_credentials::Ed25519Signer::import(&self.root_prf).await?)
    }

    /// Mint a `space → account-root` chain for a synthetic space.
    pub async fn space_chain(
        &self,
        space_seed: u8,
    ) -> Result<(dialog_varsig::Did, dialog_ucan_core::DelegationChain)> {
        use dialog_ucan_core::subject::Subject;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain};
        use dialog_varsig::Principal as _;

        let root = dialog_credentials::Ed25519Signer::import(&self.root_prf).await?;
        let space = dialog_credentials::Ed25519Signer::import(&[space_seed; 32]).await?;
        let subject = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(&root.did())
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await?;
        Ok((subject, DelegationChain::new(delegation)))
    }

    /// The account-store operator every spaces read/write in tests runs
    /// under.
    pub async fn operator(
        &self,
    ) -> Result<dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>> {
        tonk_cli::account_state::credential_operator_for_store(&self.profile, &self.store).await
    }

    /// The profile's account branch, which the fixture treats as
    /// already hydrated (see the trusted-marker write in the
    /// constructor).
    pub async fn account_branch(&self) -> Result<dialog_repository::Branch> {
        let operator = self.operator().await?;
        tonk_cli::account_state::open_account_branch_in(&self.profile, &operator, &self.store)
            .await?
            .ok_or_else(|| anyhow::anyhow!("fixture account branch did not open"))
    }

    /// Record a synthetic space in the account directory the way
    /// another device's `record_site` would have, and make its
    /// authority provable here by saving the `space → root` chain into
    /// the profile's access store (what adopting a synced account
    /// would have delivered).
    pub async fn record_directory_space(
        &self,
        space_seed: u8,
        name: Option<&str>,
        remote_url: Option<&str>,
    ) -> Result<dialog_varsig::Did> {
        use tonk_schema::directory::{MountBranch, MountRecord, MountRemote};

        let (subject, chain) = self.space_chain(space_seed).await?;
        let operator = self.operator().await?;
        self.profile
            .access()
            .save(dialog_ucan::UcanDelegation(chain))
            .perform(&operator)
            .await?;
        let account = self.account_branch().await?;
        let record = MountRecord {
            remotes: remote_url
                .map(|url| {
                    vec![MountRemote {
                        name: "origin".to_string(),
                        address: dialog_repository::SiteAddress::from(
                            dialog_remote_ucan_s3::UcanAddress::new(url),
                        ),
                        subject: subject.clone(),
                        revocation: None,
                    }]
                })
                .unwrap_or_default(),
            branches: remote_url
                .map(|_| {
                    vec![MountBranch {
                        name: "main".to_string(),
                        upstream: Some(("origin".to_string(), "main".to_string())),
                    }]
                })
                .unwrap_or_default(),
        };
        tonk_schema::directory::record(&account, &subject, name, &record, &operator).await?;
        Ok(subject)
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
            "remote": remote,
            "credentialId": authorized.root_did,
            "attachmentId": "0707070707070707070707070707070707070707070707070707070707070707",
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
