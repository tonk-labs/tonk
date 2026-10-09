mod common;

use anyhow::Result;
use serde_json::json;
use tonk_cli::{mcp_runtime::Runtime, site::TonkSite};

struct AccountProcess {
    child: std::process::Child,
    input: std::process::ChildStdin,
    output: std::io::BufReader<std::process::ChildStdout>,
}

impl AccountProcess {
    fn start(root: &std::path::Path) -> Result<(Self, serde_json::Value)> {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_tonk-mcp-runtime"));
        command.arg("--account-data").arg(root);
        Self::from_command(command)
    }

    fn from_command(mut command: std::process::Command) -> Result<(Self, serde_json::Value)> {
        use std::process::Stdio;
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut process = Self {
            input: child.stdin.take().unwrap(),
            output: std::io::BufReader::new(child.stdout.take().unwrap()),
            child,
        };
        let greeting = process.receive()?;
        Ok((process, greeting))
    }

    fn receive(&mut self) -> Result<serde_json::Value> {
        use std::io::BufRead;
        let mut line = String::new();
        self.output.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }

    fn call(&mut self, name: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
        use std::io::Write;
        writeln!(
            self.input,
            "{}",
            json!({"name": name, "arguments": arguments})
        )?;
        self.input.flush()?;
        self.receive()
    }
}

#[dialog_common::test]
#[ignore = "Requires Node and npm ci in mcp; run explicitly with --ignored"]
async fn oauth_callback_accepts_real_native_grant_and_issues_tenant_bound_token() -> Result<()> {
    use std::io::Write;
    let root = tempfile::tempdir()?;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../mcp/fixtures/oauth-native-flow.mjs");
    let mut command = std::process::Command::new("node");
    command
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_tonk-mcp-runtime"))
        .arg(root.path());
    let (mut process, greeting) = AccountProcess::from_command(command)?;
    let device = greeting["deviceDid"].as_str().unwrap();
    let approved = tonk_identity::ceremony::authorize_device(
        dialog_credentials::Ed25519Signer::generate().await?,
        device.parse()?,
        "https://accounts.example/ucan/",
    )
    .await?;
    writeln!(
        process.input,
        "{}",
        json!({
            "delegationHex": approved.delegation_hex,
            "credentialId": "oauth-native-fixture",
            "attachmentId": "oauth-native-generation"
        })
    )?;
    process.input.flush()?;
    let accepted = process.receive()?;
    assert_eq!(
        accepted,
        json!({"deviceDid": device, "rootDid": approved.root_did})
    );
    assert!(process.child.wait()?.success());
    Ok(())
}

impl Drop for AccountProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[dialog_common::test]
async fn private_account_process_accepts_a_grant_and_recovers_after_restart() -> Result<()> {
    let root = tempfile::tempdir()?;
    let tenant = root.path().join("tenant");
    let (mut process, greeting) = AccountProcess::start(&tenant)?;
    assert_eq!(greeting["capabilities"], json!([]));
    let device = greeting["deviceDid"].as_str().unwrap();
    let status = process.call("account_status", json!({}))?;
    assert_eq!(
        status["result"],
        json!({"deviceDid": device, "rootDid": null})
    );
    let approved = tonk_identity::ceremony::authorize_device(
        dialog_credentials::Ed25519Signer::generate().await?,
        device.parse()?,
        "https://accounts.example/ucan/",
    )
    .await?;
    let arguments = json!({"authorization": {
        "delegationHex": approved.delegation_hex,
        "credentialId": "hosted-process-test",
        "attachmentId": "hosted-process-generation"
    }, "expectedAccount": approved.root_did});
    let mut override_target = arguments.clone();
    override_target["profile"] = json!("another-tenant");
    assert!(process.call("account_authorize", override_target)?["error"].is_string());
    let accepted = process.call("account_authorize", arguments)?;
    assert_eq!(
        accepted["result"],
        json!({"deviceDid": device, "rootDid": approved.root_did})
    );
    // An attached account is not yet a selected, hydrated space.
    assert!(process.call("tonk_query", json!({"document": "concept:\n"}))?["error"].is_string());
    drop(process);
    let (mut reopened, next_greeting) = AccountProcess::start(&tenant)?;
    assert_eq!(greeting, next_greeting);
    assert_eq!(reopened.call("account_status", json!({}))?, accepted);
    assert!(!tenant.join("site").exists());
    Ok(())
}

#[dialog_common::test]
async fn hosted_authorization_is_isolated_validated_and_durable() -> Result<()> {
    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use tonk_cli::{account, site::open_profile, space::SpaceStore};

    let root = tempfile::tempdir()?;
    let directory = Directory::At(root.path().join("profile").to_string_lossy().into_owned());
    let profile = open_profile("hosted", directory.clone(), true).await?;
    let store = SpaceStore::at(root.path().join("account"));
    let other_root = tempfile::tempdir()?;
    let other = open_profile(
        "hosted",
        Directory::At(
            other_root
                .path()
                .join("profile")
                .to_string_lossy()
                .into_owned(),
        ),
        true,
    )
    .await?;
    let other_store = SpaceStore::at(other_root.path().join("account"));
    assert_ne!(profile.did(), other.did());

    // A correctly signed but expired grant cannot authenticate a new OAuth link.
    let signer = Ed25519Signer::generate().await?;
    let expired = dialog_ucan_core::DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(signer))
        .audience(&profile.did())
        .subject(dialog_ucan_core::subject::Subject::Any)
        .command(vec![])
        .expiration(dialog_ucan_core::time::Timestamp::try_from(1_i128)?)
        .try_build()
        .await?;
    let expired = dialog_ucan_core::DelegationChain::new(expired);
    let invalid = serde_json::to_vec(&json!({
        "delegationHex": hex::encode(expired.to_bytes()?),
        "attachmentId": "expired-generation",
        "remote": "https://accounts.example/ucan/"
    }))?;
    let error = account::accept_authorization_in(&profile, &store, &invalid, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not valid at the current time"));
    assert!(account::active_in(&profile, &store).await?.is_none());

    let approved = tonk_identity::ceremony::authorize_device(
        Ed25519Signer::generate().await?,
        profile.did(),
        "https://accounts.example/ucan/",
    )
    .await?;
    let payload = json!({
        "delegationHex": approved.delegation_hex,
        "credentialId": "hosted-test",
        "attachmentId": "service-generation-1",
        // Signed metadata must win over this untrusted callback field.
        "remote": "https://wrong.example/ucan/"
    });
    let bytes = serde_json::to_vec(&payload)?;

    let error = account::accept_authorization_in(&other, &other_store, &bytes, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not this profile"));
    assert!(account::active_in(&other, &other_store).await?.is_none());

    let mut missing_generation = payload.clone();
    missing_generation
        .as_object_mut()
        .unwrap()
        .remove("attachmentId");
    for invalid in [
        b"invalid JSON".to_vec(),
        serde_json::to_vec(&missing_generation)?,
    ] {
        account::accept_authorization_in(&profile, &store, &invalid, None)
            .await
            .unwrap_err();
        assert!(account::active_in(&profile, &store).await?.is_none());
    }
    account::accept_authorization_in(&profile, &store, &bytes, Some(&other.did()))
        .await
        .unwrap_err();
    assert!(account::active_in(&profile, &store).await?.is_none());

    let accepted = account::accept_authorization_in(&profile, &store, &bytes, None).await?;
    assert_eq!(accepted.root_did, approved.root_did);
    assert_eq!(accepted.device_did, profile.did().to_string());
    assert_eq!(
        serde_json::to_value(&accepted)?.as_object().unwrap().len(),
        2
    );
    drop(profile);
    let profile = open_profile("hosted", directory, false).await?;
    let active = account::active_in(&profile, &store).await?.unwrap();
    assert_eq!(active.root_did, accepted.root_did);
    assert_eq!(active.attachment_id, "service-generation-1");
    assert_eq!(
        active.remote.as_deref(),
        Some("https://accounts.example/ucan/")
    );

    // Duplicate delivery cannot replace the active generation or erase state.
    account::accept_authorization_in(&profile, &store, &bytes, None)
        .await
        .unwrap_err();
    assert_eq!(account::active_in(&profile, &store).await?, Some(active));
    assert!(account::active_in(&other, &other_store).await?.is_none());
    Ok(())
}

#[dialog_common::test]
async fn preview_apply_conflict_and_reopen_share_the_desktop_contract() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    let path = fixture.site.root.clone();
    let config = fixture.config.clone();
    let mut runtime = Runtime::new(fixture.site);
    let document = format!(
        "{}\n{}\ntask!: &book\n  title: \"Read a book\"\n  done: false\n",
        common::ATTRIBUTE_DECL,
        common::CONCEPT_DECL
    );
    let preview = runtime
        .call("tonk_preview", json!({"document": document}))
        .await?;
    assert_eq!(preview["committed"], false);
    let applied = runtime
        .call(
            "tonk_apply",
            json!({"document": document, "expectedRevision": preview["revision"]}),
        )
        .await?;
    assert_eq!(applied["accepted"], true);
    assert_eq!(applied["renderingConfirmed"], false);
    let stale = runtime
        .call(
            "tonk_apply",
            json!({"document": document, "expectedRevision": preview["revision"]}),
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("changed since preview"));
    let query = runtime
        .call("tonk_query", json!({"document": "task:\n"}))
        .await?;
    assert_eq!(query["matches"][0]["results"].as_array().unwrap().len(), 1);
    let entity = query["matches"][0]["results"][0]["this"].as_str().unwrap();
    let update = format!("task!:\n  this: {entity}\n  done: true\n");
    let preview = runtime
        .call("tonk_preview", json!({"document": update}))
        .await?;
    let before = runtime
        .call("tonk_query", json!({"document": "task:\n"}))
        .await?;
    assert_eq!(before["matches"][0]["results"][0]["fields"]["done"], false);
    runtime
        .call(
            "tonk_apply",
            json!({"document": update, "expectedRevision": preview["revision"]}),
        )
        .await?;
    drop(runtime);
    let mut reopened = Runtime::new(TonkSite::open_with(&path, config).await?);
    let persisted = reopened
        .call("tonk_query", json!({"document": "task:\n"}))
        .await?;
    assert_eq!(
        persisted["matches"][0]["results"][0]["fields"]["done"],
        true
    );
    Ok(())
}

#[dialog_common::test]
async fn runtime_rejects_target_overrides_and_file_includes_before_evaluation() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    let mut runtime = Runtime::new(fixture.site);
    for arguments in [
        json!({"document": "concept:\n", "space": "other"}),
        json!({"document": "!include /etc/passwd"}),
        json!({"document": "concept:\n", "expectedRevision": null}),
    ] {
        assert!(runtime.call("tonk_query", arguments).await.is_err());
    }
    assert!(
        runtime
            .call("tonk_apply", json!({"document": "concept:\n"}))
            .await
            .is_err()
    );
    assert!(
        runtime
            .call(
                "tonk_apply",
                json!({"document": "concept:\n", "expectedRevision": "bad"})
            )
            .await
            .is_err()
    );
    Ok(())
}

#[dialog_common::test]
async fn conditional_runtime_rejects_transient_commands_without_committing() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    fixture.eval_inline("concept!: &command-example\n  transient:\n  with:\n    title:\n      the: example.command/title\n      as: text\n      cardinality: one\n      description: Command title\n").await?;
    let mut runtime = Runtime::new(fixture.site);
    let document = "command-example!:\n  this: urn:command:test\n  title: Do something\n";
    let preview = runtime
        .call("tonk_preview", json!({"document": document}))
        .await?;
    let error = runtime
        .call(
            "tonk_apply",
            json!({"document": document, "expectedRevision": preview["revision"]}),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("transient commands are unavailable")
    );
    let after = runtime
        .call("tonk_query", json!({"document": "concept:\n"}))
        .await?;
    assert_eq!(after["revision"], preview["revision"]);
    Ok(())
}

#[dialog_common::test]
async fn bundled_notes_install_with_preview_provenance_and_no_replay() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    let path = fixture.site.root.clone();
    let config = fixture.config.clone();
    let mut runtime = Runtime::new(fixture.site);
    let initial = runtime
        .call("tonk_install_library", json!({"component":"prose"}))
        .await?;
    assert_eq!(initial["committed"], false);
    assert!(initial["claims"].as_u64().unwrap() > 0);
    assert_eq!(
        runtime
            .call("tonk_query", json!({"document":"seed/install:\n"}))
            .await?["matches"][0]["results"],
        json!([])
    );
    let installed = runtime
        .call(
            "tonk_install_library",
            json!({"component":"prose", "expectedRevision":initial["revision"]}),
        )
        .await?;
    assert_eq!(installed["committed"], true);
    let stale = runtime
        .call(
            "tonk_install_library",
            json!({"component":"notebook", "expectedRevision":initial["revision"]}),
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("changed since preview"));
    let repeated = runtime
        .call(
            "tonk_install_library",
            json!({"component":"prose", "expectedRevision":installed["revision"]}),
        )
        .await?;
    assert_eq!(repeated["alreadyInstalled"], true);
    assert_eq!(repeated["committed"], false);
    assert_eq!(repeated["revision"], installed["revision"]);
    let document = "prose!:\n  this: urn:test:note\n  content: '# ChatGPT integration test\\nCreated with the bundled prose model.'\n";
    let preview = runtime
        .call("tonk_preview", json!({"document":document}))
        .await?;
    runtime
        .call(
            "tonk_apply",
            json!({"document":document,"expectedRevision":preview["revision"]}),
        )
        .await?;
    let preview = runtime
        .call("tonk_install_library", json!({"component":"notebook"}))
        .await?;
    runtime
        .call(
            "tonk_install_library",
            json!({"component":"notebook","expectedRevision":preview["revision"]}),
        )
        .await?;
    let document = "notebook!:\n  this: urn:test:notebook\n  title: ChatGPT notebook\n  block: {N1: urn:test:notebook:block}\nnotebook/block!:\n  this: urn:test:notebook:block\n  notebook: urn:test:notebook\n  source: Hello from ChatGPT\n";
    let preview = runtime
        .call("tonk_preview", json!({"document":document}))
        .await?;
    runtime
        .call(
            "tonk_apply",
            json!({"document":document,"expectedRevision":preview["revision"]}),
        )
        .await?;
    drop(runtime);
    let mut runtime = Runtime::new(TonkSite::open_with(&path, config).await?);
    let notes = runtime
        .call(
            "tonk_query",
            json!({"document":"prose:\n  this: urn:test:note\n"}),
        )
        .await?;
    assert_eq!(notes["matches"][0]["results"][0]["this"], "urn:test:note");
    let notebooks = runtime
        .call(
            "tonk_query",
            json!({"document":"notebook:\n  this: urn:test:notebook\n"}),
        )
        .await?;
    assert_eq!(
        notebooks["matches"][0]["results"][0]["fields"]["title"],
        "ChatGPT notebook"
    );
    let records = runtime
        .call("tonk_query", json!({"document":"seed/install:\n"}))
        .await?;
    assert_eq!(
        records["matches"][0]["results"].as_array().unwrap().len(),
        2
    );
    for component in ["../core", "file:///etc/passwd", "table"] {
        assert!(
            runtime
                .call("tonk_install_library", json!({"component":component}))
                .await
                .is_err()
        );
    }
    Ok(())
}

#[dialog_common::test]
async fn library_install_leaves_untracked_models_and_other_versions_alone() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    fixture.eval_inline("concept!: &prose\n  description: Authored model\n  with:\n    title:\n      description: Authored title\n      the: example.authored/title\n      as: text\nprose!:\n  this: urn:authored:one\n  title: Keep me\n").await?;
    let branch = fixture.site.branch().await?;
    let before = branch.handle().revision();
    let mut runtime = Runtime::new(fixture.site);
    for extra in [json!({}), json!({"expectedRevision":before})] {
        let mut args = json!({"component":"prose"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let error = runtime
            .call("tonk_install_library", args)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
    }
    let read = runtime
        .call("tonk_query", json!({"document":"prose:\n"}))
        .await?;
    assert_eq!(read["revision"], serde_json::to_value(before)?);
    assert_eq!(
        read["matches"][0]["results"][0]["fields"]["title"],
        "Keep me"
    );

    let fixture = common::TestSite::new().await?;
    let branch = fixture.site.branch().await?;
    branch
        .handle()
        .transaction()
        .assert(tonk_schema::SeedAvailable {
            this: "seed:older".parse()?,
            source: tonk_schema::domain::seed::Source("/library/notebook.yaml".into()),
            replaces: tonk_schema::domain::seed::Replaces("seed:none".parse()?),
        })
        .commit()
        .publish()
        .perform(&fixture.site.operator)
        .await?;
    let before = branch.handle().revision();
    let mut runtime = Runtime::new(fixture.site);
    let error = runtime
        .call(
            "tonk_install_library",
            json!({"component":"notebook","expectedRevision":before}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Upgrades and repairs"));
    assert_eq!(branch.handle().revision(), before);
    Ok(())
}

#[dialog_common::test]
async fn ui_read_routes_in_ephemeral_overlay_and_rejects_authority_changes() -> Result<()> {
    let fixture = common::TestSite::new().await?;
    let branch = fixture.site.branch().await?;
    let mut runtime = Runtime::new(fixture.site);
    let preview = runtime
        .call("tonk_install_library", json!({"component":"notebook"}))
        .await?;
    runtime
        .call(
            "tonk_install_library",
            json!({"component":"notebook","expectedRevision":preview["revision"]}),
        )
        .await?;
    let before = branch.handle().revision();
    let query = json!({"predicate":{"with":{"entity":{"the":"xyz.tonk.site/entity","as":"Entity","cardinality":"one"}}},"terms":{"this":"site:chatgpt","entity":{"?":{"name":"entity"}}}});
    let result = runtime
        .call(
            "ui_read",
            json!({"path":"/notebook/urn:demo:one","query":query}),
        )
        .await?;
    assert_eq!(result["site"], "site:chatgpt");
    assert!(result["rows"].to_string().contains("urn:demo:one"));
    let result = runtime
        .call(
            "ui_read",
            json!({"path":"/notebook/urn:demo:two","query":query}),
        )
        .await?;
    assert!(result["rows"].to_string().contains("urn:demo:two"));
    assert!(!result["rows"].to_string().contains("urn:demo:one"));
    assert_eq!(branch.handle().revision(), before);
    for args in [
        json!({"path":"//other"}),
        json!({"path":"/notebook/x?y"}),
        json!({"path":"/","repository":"other"}),
        json!({"path":"/","branch":"other"}),
    ] {
        assert!(runtime.call("ui_read", args).await.is_err());
    }
    assert_eq!(branch.handle().revision(), before);
    Ok(())
}
