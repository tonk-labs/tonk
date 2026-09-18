use super::*;

fn registry(profile: &TempDir) -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(&std::fs::read(
        profile.path().join("spaces/spaces.json"),
    )?)?)
}

fn aliases_for(registry: &serde_json::Value, subject: &str) -> Vec<String> {
    registry["spaces"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, entry)| entry["connection"]["subject"] == subject)
        .map(|(name, _)| name.clone())
        .collect()
}

async fn groups(browser: &WebDriver, request: &str) -> Result<Vec<serde_json::Value>> {
    browser.enter_default_frame().await?;
    let reply = get_json(browser, "/api/account/terminal-links").await?;
    let rows = successful_body("terminal management records", &reply);
    Ok(rows
        .as_array()
        .context("terminal rows")?
        .iter()
        .find(|row| row["requestId"] == request)
        .context("terminal record")?["spaces"]
        .as_array()
        .context("terminal groups")?
        .clone())
}

async fn open_management(browser: &WebDriver, env: &TestEnvironment, request: &str) -> Result<()> {
    browser.enter_default_frame().await?;
    goto(browser, env.tonk_web.join("settings")?.as_str()).await?;
    enter_hub(browser).await?;
    wait_for_displayed(browser, "[data-terminal-management-refresh]").await?;
    click(browser, "[data-terminal-management-refresh]").await?;
    wait_for_displayed(browser, &format!("[data-terminal-record='{request}']")).await?;
    Ok(())
}

async fn add_space(browser: &WebDriver, request: &str, subject: &str) -> Result<()> {
    let row = format!("[data-terminal-record='{request}']");
    click(browser, &format!("{row} [data-terminal-add-open]")).await?;
    let choice = format!("{row} [data-terminal-add-subject='{subject}']:not([disabled])");
    wait_for_displayed(browser, &choice).await?;
    click(browser, &choice).await?;
    click(browser, &format!("{row} [data-terminal-add-submit]")).await?;
    wait_for_text_containing(
        browser,
        "[data-terminal-management-result]",
        "access was sent.",
    )
    .await?;
    Ok(())
}

async fn capture_terminal_management(browser: &WebDriver, request: &str) -> Result<()> {
    for (name, width, height, dark) in [
        ("desktop", 1200, 900, false),
        ("narrow", 390, 844, false),
        ("short-dark", 390, 540, true),
    ] {
        browser.enter_default_frame().await?;
        browser.set_window_rect(0, 0, width, height).await?;
        ChromeDevTools::new(browser.handle.clone())
            .execute_cdp_with_params(
                "Emulation.setDeviceMetricsOverride",
                serde_json::json!({
                    "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false,
                }),
            )
            .await?;
        ChromeDevTools::new(browser.handle.clone())
            .execute_cdp_with_params(
                "Emulation.setEmulatedMedia",
                serde_json::json!({ "features": [
                { "name": "prefers-reduced-motion", "value": "reduce" },
                { "name": "prefers-color-scheme", "value": if dark { "dark" } else { "light" } },
            ] }),
            )
            .await?;
        let outer = browser.execute("return {width:document.documentElement.clientWidth, scroll:document.documentElement.scrollWidth}", vec![]).await?;
        assert_eq!(outer.json()["width"], serde_json::json!(width));
        assert!(outer.json()["scroll"].as_u64().unwrap() <= u64::from(width));
        enter_hub(browser).await?;
        browser
            .execute(
                "document.querySelector('[data-terminal-management-refresh]').focus()",
                vec![],
            )
            .await?;
        browser
            .find(By::Css("[data-terminal-management-refresh]"))
            .await?
            .send_keys(Key::Tab)
            .await?;
        assert_eq!(
            browser.execute("return document.activeElement.matches('details > summary')", vec![]).await?.json(),
            &serde_json::json!(true),
            "keyboard must reach terminal details before add spaces"
        );
        browser.active_element().await?.send_keys(Key::Tab).await?;
        let focus = browser.execute(r#"const node=document.activeElement, style=getComputedStyle(node);return {
            add:node.matches('[data-terminal-add-open]'), visible:node.matches(':focus-visible'),
            ring:style.outlineStyle!=='none'||style.boxShadow!=='none', height:node.getBoundingClientRect().height,
            animation:style.animationName, reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,
            width:document.documentElement.clientWidth, scroll:document.documentElement.scrollWidth
        }"#, vec![]).await?;
        let state = focus.json();
        assert_eq!(
            state["add"], true,
            "keyboard did not reach add spaces: {state}"
        );
        assert_eq!(state["visible"], true, "keyboard focus invisible: {state}");
        assert_eq!(state["ring"], true, "keyboard focus ring absent: {state}");
        assert!(
            state["height"].as_f64().unwrap() >= 44.0,
            "small management target: {state}"
        );
        assert_eq!(state["animation"], "none");
        assert!(
            state["scroll"].as_u64().unwrap() <= state["width"].as_u64().unwrap(),
            "terminal management overflows: {state}"
        );
        element(
            browser,
            &format!("[data-terminal-record='{request}'] [data-terminal-add-open]"),
        )
        .await?
        .scroll_into_view()
        .await?;
        capture_handoff_page(browser, &format!("terminal-management-{name}")).await?;
    }
    Ok(())
}

/// A stopped CLI receives an addressed addition later; standard leaf revocation
/// preserves local edits and sibling access, and re-add creates a fresh replica.
#[dialog_common::test]
async fn it_manages_offline_terminal_additions_and_revocations(env: TestEnvironment) -> Result<()> {
    let browser = driver_with_prf(&env).await?;
    sign_up(&browser, &env, "terminal-management@example.com").await?;
    let mut subjects = Vec::new();
    for name in ["Managed first", "Managed second"] {
        let repo = create_space_awaiting_remote(&browser, name, true).await?;
        let reply = post_json(
            &browser,
            &format!("/api/repository/{repo}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("publish management source", &reply);
    }
    let profile = tempfile::tempdir()?;
    let PendingTerminal {
        mut child,
        mut stdout,
        mut stderr,
        prefix,
        approval_url,
        request,
    } = start_terminal(&env, &profile, &[]).await?;
    goto(&browser, approval_url.as_str()).await?;
    enter_hub(&browser).await?;
    wait_for_displayed(&browser, "[data-terminal-subject]:not([disabled])").await?;
    for name in ["Managed first", "Managed second"] {
        let mut found = None;
        for row in browser.find_all(By::Css(".terminal-choice")).await? {
            if row.text().await?.contains(name) {
                let input = row.find(By::Css("[data-terminal-subject]")).await?;
                found = input.attr("data-terminal-subject").await?;
                if name == "Managed first" {
                    input.click().await?;
                }
                break;
            }
        }
        subjects.push(found.context("named management candidate")?);
    }
    click_terminal_decision(&browser, "[data-terminal-approve]").await?;
    await_terminal_decision(
        &browser,
        &request.id(),
        "spaces approved. return to your terminal to finish.",
    )
    .await?;
    let output = finish_link(&mut child, &mut stdout, &mut stderr, prefix).await?;
    anyhow::ensure!(
        output.status.success(),
        "initial management import: {}",
        output.stderr
    );
    // finish_link waits for process exit: no terminal process is online during addition.
    let initial = registry(&profile)?;
    let old_alias = aliases_for(&initial, &subjects[0])
        .pop()
        .context("first alias")?;
    assert_eq!(initial["spaces"].as_object().unwrap().len(), 1);
    open_management(&browser, &env, &request.id()).await?;
    add_space(&browser, &request.id(), &subjects[1]).await?;
    assert_eq!(
        registry(&profile)?,
        initial,
        "browser addition changed an offline CLI"
    );
    let added_groups = groups(&browser, &request.id()).await?;
    assert_eq!(added_groups.len(), 2);
    assert!(
        added_groups
            .iter()
            .all(|group| group["recipient"] == request.recipient().to_string())
    );
    let resumed = run_cli(
        &env,
        &profile,
        &[
            "link".into(),
            "--resume".into(),
            request.id(),
            "--no-open".into(),
        ],
    )
    .await?;
    anyhow::ensure!(
        resumed.status.success(),
        "receive offline addition: {}",
        resumed.stderr
    );
    let expanded = registry(&profile)?;
    let second_alias = aliases_for(&expanded, &subjects[1])
        .pop()
        .context("added alias")?;
    assert_eq!(expanded["spaces"].as_object().unwrap().len(), 2);
    assert_eq!(
        expanded["spaces"][&old_alias],
        initial["spaces"][&old_alias]
    );
    let edit_document = "attribute!: &retained-title\n  description: Retained terminal edit\n  the: test.terminal/title\n  as: text\n  cardinality: one\nconcept!: &retained-note\n  description: Retained terminal note\n  with:\n    title: retained-title\nretained-note!: &retained-one\n  title: Unsynced terminal edit survives revocation\n";
    let edit = run_cli(
        &env,
        &profile,
        &[
            "--space".into(),
            old_alias.clone(),
            "eval".into(),
            "--no-sync".into(),
            "-c".into(),
            edit_document.into(),
        ],
    )
    .await?;
    anyhow::ensure!(edit.status.success(), "offline edit: {}", edit.stderr);
    let old_group = added_groups
        .iter()
        .find(|g| g["subject"] == subjects[0])
        .context("old group")?;
    let old_id = old_group["id"].as_str().context("old group id")?;
    open_management(&browser, &env, &request.id()).await?;
    click(
        &browser,
        &format!(
            "[data-terminal-record='{}'] [data-connection-revoke='{old_id}']",
            request.id()
        ),
    )
    .await?;
    wait_for_text_containing(
        &browser,
        &format!("[data-connection-id='{old_id}']"),
        "access removal confirmed for 6 of 6 permissions",
    )
    .await?;
    let revoked = groups(&browser, &request.id()).await?;
    let old = revoked.iter().find(|g| g["id"] == old_id).unwrap();
    assert_eq!(old["targets"].as_array().unwrap().len(), 6);
    assert!(
        old["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["acknowledged"] == true)
    );
    let denied = run_cli(
        &env,
        &profile,
        &["--space".into(), old_alias.clone(), "push".into()],
    )
    .await?;
    assert!(
        !denied.status.success(),
        "removed grant pushed successfully"
    );
    assert!(denied.stderr.contains("revoked"), "{}", denied.stderr);
    assert!(!denied.stderr.contains("account login"));
    let sibling_edit = run_cli(
        &env,
        &profile,
        &[
            "--space".into(),
            second_alias.clone(),
            "eval".into(),
            "--no-sync".into(),
            "-c".into(),
            edit_document.replace(
                "Unsynced terminal edit survives revocation",
                "Independent remaining grant commits",
            ),
        ],
    )
    .await?;
    anyhow::ensure!(
        sibling_edit.status.success(),
        "remaining grant local edit: {}",
        sibling_edit.stderr
    );
    let sibling = run_cli(
        &env,
        &profile,
        &["--space".into(), second_alias, "push".into()],
    )
    .await?;
    anyhow::ensure!(
        sibling.status.success(),
        "remaining space push: {}",
        sibling.stderr
    );
    open_management(&browser, &env, &request.id()).await?;
    add_space(&browser, &request.id(), &subjects[0]).await?;
    let readded = groups(&browser, &request.id()).await?;
    let fresh = readded
        .iter()
        .find(|g| g["subject"] == subjects[0] && g["id"] != old_id)
        .context("fresh re-add group")?;
    assert_eq!(fresh["recipient"], old_group["recipient"]);
    assert!(fresh["targets"].as_array().unwrap().iter().all(|new| {
        !old_group["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|old| old["cid"] == new["cid"])
    }));
    // This delivery is revoked while the CLI is still offline. A later fresh
    // grant must remain receivable without installing the rejected delivery.
    let stale_id = fresh["id"]
        .as_str()
        .context("queued stale group")?
        .to_owned();
    open_management(&browser, &env, &request.id()).await?;
    click(
        &browser,
        &format!(
            "[data-terminal-record='{}'] [data-connection-revoke='{stale_id}']",
            request.id()
        ),
    )
    .await?;
    wait_for_text_containing(
        &browser,
        &format!("[data-connection-id='{stale_id}']"),
        "access removal confirmed for 6 of 6 permissions",
    )
    .await?;
    assert_eq!(
        registry(&profile)?,
        expanded,
        "offline revocation changed CLI registry"
    );
    open_management(&browser, &env, &request.id()).await?;
    add_space(&browser, &request.id(), &subjects[0]).await?;
    let newest = groups(&browser, &request.id()).await?;
    let fresh = newest
        .iter()
        .find(|g| g["subject"] == subjects[0] && g["id"] != old_id && g["id"] != stale_id)
        .context("newest queued re-add")?;
    assert_eq!(fresh["recipient"], request.recipient().to_string());
    assert!(fresh["targets"].as_array().unwrap().iter().all(|new| {
        readded
            .iter()
            .flat_map(|g| g["targets"].as_array().unwrap())
            .all(|old| old["cid"] != new["cid"])
    }));
    let resumed = run_cli(
        &env,
        &profile,
        &[
            "link".into(),
            "--resume".into(),
            request.id(),
            "--no-open".into(),
        ],
    )
    .await?;
    anyhow::ensure!(
        resumed.status.success(),
        "receive fresh re-add: {}",
        resumed.stderr
    );
    assert!(
        resumed.stderr.contains("was not installed: revoked"),
        "queued revocation was not reported: {}",
        resumed.stderr
    );
    let latest = registry(&profile)?;
    assert_eq!(latest["spaces"].as_object().unwrap().len(), 3);
    assert!(
        latest["spaces"]
            .as_object()
            .unwrap()
            .values()
            .all(|entry| entry["connection"]["id"] != stale_id),
        "revoked queued delivery published an alias"
    );
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(
        profile
            .path()
            .join("spaces/terminal-links")
            .join(request.id())
            .join("deliveries.json"),
    )?)?;
    assert_eq!(journal["cursor"], 3);
    assert_eq!(
        journal["rejected"]
            .as_object()
            .context("rejected delivery journal")?
            .len(),
        1
    );
    assert!(
        journal["rejected"]
            .as_object()
            .unwrap()
            .values()
            .all(|row| row["reason"] == "revoked")
    );

    let fresh_alias = aliases_for(&latest, &subjects[0])
        .into_iter()
        .find(|a| a != &old_alias)
        .context("fresh alias")?;
    assert_ne!(
        latest["spaces"][&fresh_alias]["site"],
        latest["spaces"][&old_alias]["site"]
    );
    let local = run_cli(
        &env,
        &profile,
        &[
            "--space".into(),
            old_alias.clone(),
            "eval".into(),
            "--no-sync".into(),
            "-c".into(),
            "retained-note:\n  title: ?title\n".into(),
        ],
    )
    .await?;
    anyhow::ensure!(
        local.status.success(),
        "retained local read: {}",
        local.stderr
    );
    assert!(
        local
            .stdout
            .contains("Unsynced terminal edit survives revocation")
    );
    let fresh_edit = run_cli(
        &env,
        &profile,
        &[
            "--space".into(),
            fresh_alias.clone(),
            "eval".into(),
            "--no-sync".into(),
            "-c".into(),
            edit_document.replace(
                "Unsynced terminal edit survives revocation",
                "Fresh re-add grant commits",
            ),
        ],
    )
    .await?;
    anyhow::ensure!(
        fresh_edit.status.success(),
        "fresh grant local edit: {}",
        fresh_edit.stderr
    );
    let fresh_push = run_cli(
        &env,
        &profile,
        &["--space".into(), fresh_alias, "push".into()],
    )
    .await?;
    anyhow::ensure!(
        fresh_push.status.success(),
        "fresh grant push: {}",
        fresh_push.stderr
    );
    open_management(&browser, &env, &request.id()).await?;
    capture_handoff_page(&browser, "terminal-management-readded").await?;
    click(
        &browser,
        &format!(
            "[data-terminal-record='{}'] [data-terminal-revoke-all]",
            request.id()
        ),
    )
    .await?;
    wait_for_text_containing(
        &browser,
        "[data-terminal-management-result]",
        "access removal confirmed for 24 of 24 permissions",
    )
    .await?;
    let final_groups = groups(&browser, &request.id()).await?;
    assert_eq!(final_groups.len(), 4);
    assert!(final_groups.iter().all(|g| {
        g["targets"].as_array().unwrap().len() == 6
            && g["targets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["acknowledged"] == true)
    }));
    assert_eq!(
        registry(&profile)?,
        latest,
        "revocation removed local state"
    );
    open_management(&browser, &env, &request.id()).await?;
    capture_terminal_management(&browser, &request.id()).await?;
    browser.quit().await?;
    Ok(())
}
