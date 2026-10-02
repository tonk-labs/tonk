// Opt-in regression against an actual released browser runtime.
mod legacy_worker {
    use super::*;
    /// Optional released-runtime regression. The historical browser binaries
    /// are not published as immutable release assets and must not be fetched
    /// from a moving production URL by CI. Supply both verified artifact trees:
    /// TONK_UI_TEST_ARTIFACT=<stable ea560cf2de60a1d2>
    /// TONK_UI_UPGRADE_ARTIFACT=<candidate> TONK_E2E_LOOPBACK_ONLY=1
    /// cargo test -p tonk-ui --features integration-tests --lib
    /// released_worker_keeps_rendering_during_a_mixed_version_upgrade -- --ignored
    /// The ordinary worker suite covers the same missing site-field contract
    /// without external artifacts. This exercises real old/new Wasm, sync,
    /// two devices sharing an account, and persisted browser reopening.
    #[dialog_common::test]
    #[ignore = "requires verified historical and candidate browser artifact trees"]
    async fn released_worker_keeps_rendering_during_a_mixed_version_upgrade() -> Result<()> {
        use dialog_common::helpers::Provisionable;
        anyhow::ensure!(
            std::env::var_os("TONK_E2E_LOOPBACK_ONLY").is_some(),
            "set TONK_E2E_LOOPBACK_ONLY=1 for this synthetic regression"
        );
        // Provision explicitly: the integration macro puts #[ignore] on its
        // logic function rather than the generated test wrapper.
        let service = TestEnvironment::start(Default::default()).await?;
        let result = tokio::spawn(mixed_version_upgrade(service.address.clone())).await;
        service.stop().await?;
        result?
    }

    async fn mixed_version_upgrade(env: TestEnvironment) -> Result<()> {
        use crate::service_worker_upgrade::tests::{
            copy_artifact_tree, promote_second_generation, wait_for_mounted_build,
        };
        let candidate = PathBuf::from(
            std::env::var("TONK_UI_UPGRADE_ARTIFACT")
                .context("set TONK_UI_UPGRADE_ARTIFACT to the candidate browser artifact")?,
        );
        let historical = PathBuf::from(std::env::var("TONK_UI_TEST_ARTIFACT")?);
        let stable: serde_json::Value =
            serde_json::from_slice(&std::fs::read(historical.join("version.json"))?)?;
        anyhow::ensure!(
            stable["build"] == "ea560cf2de60a1d2",
            "expected the verified released stable build: {stable}"
        );
        copy_artifact_tree(&candidate, &env.deployment_root.join("generation-b"))?;
        let version: serde_json::Value =
            serde_json::from_slice(&std::fs::read(candidate.join("version.json"))?)?;
        let build = version["build"]
            .as_str()
            .context("candidate build missing")?;
        let (browser, owner_authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&browser, &env, "migration-audit@example.com").await?;
        successful_body(
            "name account",
            &post_json(
                &browser,
                "/api/account/display-name",
                serde_json::json!({"name":"Migration Audit Owner"}),
            )
            .await?,
        );
        goto(&browser, env.tonk_web.as_str()).await?;
        enter_hub(&browser).await?;
        browser.execute("const input=document.querySelector('.snew-form input[name=name]'); input.value='Persisted Stable Garden'; input.dispatchEvent(new Event('input',{bubbles:true}));",vec![]).await?;
        click(&browser, ".snew-form button[type=submit]").await?;
        browser.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let key = loop {
            let current = browser.current_url().await?;
            if let Some(key) = current.path().strip_prefix("/space/") {
                break key.to_owned();
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "stable UI create did not navigate: {current}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let document = r#"
concept!: &audit-home
  this: audit:home
  description: Synthetic authored migration sentinel.
  with:
    subject:
      description: The authored space subject.
      the: dialog.replica/subject
      as: entity
view!:
  this: audit:home
  show:
    ui: |
      <h1 id="audit-preserved">Stable authored garden</h1>
      <a id="audit-link" href="/garden">Garden route</a>
      <tonk-display model="audit:note" view="ui"></tonk-display>
concept!: &note
  this: audit:note
  description: Independently authored migration data.
  with:
    text:
      description: The authored note text.
      the: audit.note/text
      as: text
view!:
  this: audit:note
  show:
    ui: '<p class="audit-note">{text}</p>'
note!:
  this: audit:original
  text: "Original stable data"
name!:
  this: id:tonk/space
  entity: audit:home
route!:
  this: audit:route
  path: /garden
  concept: audit:home
"#;
        let wrote = post_yaml(
            &browser,
            &format!("/api/repository/{key}/branch/main/evaluate"),
            document,
        )
        .await?;
        anyhow::ensure!(wrote["status"] == 200, "author stable content: {wrote}");
        let url = env.tonk_web.join(&format!("space/{key}"))?;
        goto(&browser, url.as_str()).await?;
        enter_space_view(&browser).await?;
        wait_for_text(&browser, "#audit-preserved", "Stable authored garden").await?;
        anyhow::ensure!(
            wait_for_displayed(&browser, "#audit-link")
                .await?
                .is_displayed()
                .await?
        );
        browser.enter_default_frame().await?;
        successful_body(
            "stable sync",
            &post_json(&browser, "/api/sync", serde_json::json!({})).await?,
        );
        let profile = std::fs::read_dir(&env.browser_profile_root)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("chrome-profile-")
            })
            .context("profile missing")?;
        let (old, _) =
            second_device_with_same_passkey(&env, &browser, &owner_authenticator).await?;
        old.add_cookie(Cookie::new("tonk-test-generation", "a"))
            .await?;
        let old_profile = std::fs::read_dir(&env.browser_profile_root)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("chrome-profile-")
                    && path != &profile
            })
            .context("old profile missing")?;
        raise_cluster_from_hub(&old, &env).await?;
        run_cluster_login_with_action(
            &old,
            "migration-audit@example.com",
            "log in with your passkey",
        )
        .await?;
        goto(&old, env.tonk_web.as_str()).await?;
        sync_pair(&browser, &old).await?;
        wait_for_profile_name(&old, "Migration Audit Owner").await?;
        render_notes(&old, url.as_str(), &["Original stable data"]).await?;
        let owner_account = get_json(&browser, "/api/account").await?;
        let linked_account = get_json(&old, "/api/account").await?;
        anyhow::ensure!(
            linked_account["body"]["accountState"] == "ready"
                && linked_account["body"]["rootDid"] == owner_account["body"]["rootDid"],
            "clients must share the same ready synthetic account"
        );
        promote_second_generation(&env)?;
        browser.refresh().await?;
        wait_for_mounted_build(&browser, build).await?;
        goto(&browser, url.as_str()).await?;
        enter_space_view(&browser).await?;
        wait_for_text(&browser, "#audit-preserved", "Stable authored garden").await?;
        anyhow::ensure!(
            wait_for_displayed(&browser, "#audit-link")
                .await?
                .is_displayed()
                .await?
        );
        browser.enter_default_frame().await?;
        goto(&browser, env.tonk_web.as_str()).await?;
        enter_hub(&browser).await?;
        wait_for_text_containing(&browser, "body", "Persisted Stable Garden").await?;
        browser.enter_default_frame().await?;
        wait_for_profile_name(&browser, "Migration Audit Owner").await?;
        let old_health = get_json(&old, "/api/health").await?;
        anyhow::ensure!(
            old_health["body"]["build"] == "ea560cf2de60a1d2",
            "old client not pinned: {old_health}"
        );
        let current_health = get_json(&browser, "/api/health").await?;
        anyhow::ensure!(
            current_health["body"]["build"] == build,
            "current client wrong generation: {current_health}"
        );
        let endpoint = format!("/api/repository/{key}/branch/main/evaluate");
        let old_write = post_yaml(
            &old,
            &endpoint,
            "note!:\n  this: audit:old-write\n  text: \"Written on stable\"\n",
        )
        .await?;
        anyhow::ensure!(old_write["status"] == 200, "old write failed: {old_write}");
        sync_pair(&old, &browser).await?;
        render_notes(
            &old,
            url.as_str(),
            &["Original stable data", "Written on stable"],
        )
        .await?;
        render_notes(
            &browser,
            url.as_str(),
            &["Original stable data", "Written on stable"],
        )
        .await?;
        let current_write = post_yaml(
            &browser,
            &endpoint,
            "note!:\n  this: audit:new-write\n  text: \"Written on candidate\"\n",
        )
        .await?;
        anyhow::ensure!(
            current_write["status"] == 200,
            "candidate write failed: {current_write}"
        );
        sync_pair(&browser, &old).await?;
        render_notes(
            &old,
            url.as_str(),
            &[
                "Original stable data",
                "Written on stable",
                "Written on candidate",
            ],
        )
        .await?;
        render_notes(
            &browser,
            url.as_str(),
            &[
                "Original stable data",
                "Written on stable",
                "Written on candidate",
            ],
        )
        .await?;
        anyhow::ensure!(
            get_json(&old, "/api/health").await?["body"]["build"] == "ea560cf2de60a1d2",
            "old client unexpectedly upgraded during sync"
        );
        old.delete_cookie("tonk-test-generation").await?;
        old.refresh().await?;
        wait_for_mounted_build(&old, build).await?;
        render_notes(
            &old,
            url.as_str(),
            &[
                "Original stable data",
                "Written on stable",
                "Written on candidate",
            ],
        )
        .await?;
        wait_for_profile_name(&old, "Migration Audit Owner").await?;
        old.quit().await?;
        let old_reopened = WebDriver::new(
            env.chromedriver.as_str(),
            env.chrome_capabilities_for_profile(&old_profile)?,
        )
        .await?;
        goto(&old_reopened, url.as_str()).await?;
        wait_for_mounted_build(&old_reopened, build).await?;
        render_notes(
            &old_reopened,
            url.as_str(),
            &[
                "Original stable data",
                "Written on stable",
                "Written on candidate",
            ],
        )
        .await?;
        wait_for_profile_name(&old_reopened, "Migration Audit Owner").await?;
        old_reopened.quit().await?;
        browser.quit().await?;
        let reopened = WebDriver::new(
            env.chromedriver.as_str(),
            env.chrome_capabilities_for_profile(&profile)?,
        )
        .await?;
        goto(&reopened, url.as_str()).await?;
        wait_for_service_worker(&reopened).await?;
        enter_space_view(&reopened).await?;
        wait_for_text(&reopened, "#audit-preserved", "Stable authored garden").await?;
        anyhow::ensure!(
            wait_for_displayed(&reopened, "#audit-link")
                .await?
                .is_displayed()
                .await?
        );
        reopened.enter_default_frame().await?;
        wait_for_profile_name(&reopened, "Migration Audit Owner").await?;
        reopened.quit().await?;
        Ok(())
    }

    async fn wait_for_profile_name(client: &WebDriver, expected: &str) -> Result<()> {
        client.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let profiles = get_json(client, "/api/profiles").await?;
            let matches = profiles["body"]["profiles"].as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["active"] == true && row["displayName"] == expected)
            });
            if matches {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "active profile name mismatch: {profiles}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn sync_pair(writer: &WebDriver, reader: &WebDriver) -> Result<()> {
        for client in [writer, reader, writer, reader] {
            client.enter_default_frame().await?;
            successful_body(
                "mixed synthetic sync",
                &post_json(client, "/api/sync", serde_json::json!({})).await?,
            );
        }
        Ok(())
    }

    async fn render_notes(client: &WebDriver, url: &str, notes: &[&str]) -> Result<()> {
        client.enter_default_frame().await?;
        goto(client, url).await?;
        enter_space_view(client).await?;
        wait_for_text(client, "#audit-preserved", "Stable authored garden").await?;
        anyhow::ensure!(
            wait_for_displayed(client, "#audit-link")
                .await?
                .is_displayed()
                .await?
        );
        for note in notes {
            wait_for_text_containing(client, "body", note).await?;
        }
        client.enter_default_frame().await?;
        Ok(())
    }
}
