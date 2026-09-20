// Review-only full application test: two isolated browser profiles, two
// service workers, and the repository's real local HTTP access service.
#[dialog_common::test]
async fn review_automerge_two_browsers_with_access_service(env: TestEnvironment) -> Result<()> {
    let first = driver_with_prf(&env).await?;
    let second = driver_with_prf(&env).await?;
    let result = async {
        sign_up(&first, &env, "automerge-owner@example.com").await?;
        let repo = create_space_awaiting_remote(&first, "Automerge review", true).await?;
        first.enter_default_frame().await?;
        let text_path = format!("/api/repository/{repo}/branch/main/document/id%3Areview%2Ftext");
        let table_path = format!("/api/repository/{repo}/branch/main/document/id%3Areview%2Ftable");
        let initialized = post_json(&first, &text_path, serde_json::json!({"format":"automerge/text@1", "edits":[{"edit":"set-text", "text":"base"}]})).await?;
        successful_body("create prose", &initialized);
        let initialized = post_json(&first, &table_path, serde_json::json!({"format":"automerge/table@1", "edits":[{"edit":"put", "path":"sheets/s/name", "value":"Sheet 1"}]})).await?;
        successful_body("create workbook", &initialized);
        // `/api/sync` only pokes the debounced sweep; it does not wait
        // for publication. Establish a real remote branch before inviting.
        let pushed = post_json(&first, &format!("/api/repository/{repo}/branch/main/sync/push"), serde_json::json!({})).await?;
        successful_body("publish initial branch", &pushed);
        let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let invited = post_json(&first, &format!("/api/repository/{repo}/invite"), serde_json::json!({"baseUrl":env.tonk_web.join("join")?})).await?;
        let invite_url = successful_body("invite second browser", &invited)["url"].as_str().context("invite URL")?.to_string();
        sign_up(&second, &env, "automerge-editor@example.com").await?;
        second.enter_default_frame().await?;
        let joined = post_json(&second, "/api/profile/join", serde_json::json!({"url":invite_url})).await?;
        if joined["status"] != 200 {
            dump_browser_log(&first, &env).await;
            dump_browser_log(&second, &env).await;
        }
        successful_body("join second browser", &joined);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let base = loop {
            let read = get_json(&second, &text_path).await?;
            if read["status"] == 200 && read["body"]["text"] == "base" { break read["body"].clone(); }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "second browser never received initial bytes: {read}");
            let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        };
        eprintln!("AUTOMERGE REVIEW: second browser opened the initial remote document");
        // Both edits name the same base, even if background sync runs.
        let left = post_json(&first, &text_path, serde_json::json!({"heads":base["heads"],"edits":[{"edit":"set-text","text":"left base"}]})).await?;
        successful_body("first concurrent edit", &left);
        let right = post_json(&second, &text_path, serde_json::json!({"heads":base["heads"],"edits":[{"edit":"set-text","text":"base right"}]})).await?;
        successful_body("second concurrent edit", &right);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = get_json(&first, &text_path).await?;
            let right = get_json(&second, &text_path).await?;
            if left["body"]["text"] == "left base right" && right["body"]["text"] == "left base right" { break; }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "prose did not converge: left={left}, right={right}");
            let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        eprintln!("AUTOMERGE REVIEW: two-browser concurrent prose converged through HTTP access service");
        // Exercise the actual element + host event route in both profiles,
        // not just the JSON route. Both editors open before either edits.
        for browser in [&first, &second] {
            browser.execute(r#"return (async () => {
                await import('/tonk-prose/tonk-prose.js');
                const el = document.createElement('tonk-prose');
                el.setAttribute('with', 'main@' + arguments[0]);
                el.setAttribute('subject', 'id:review/text');
                window.__reviewProse = el;
                document.body.append(el);
                for (let n=0; n<300 && (!el.editor || el.value !== 'left base right'); n++)
                    await new Promise(r => setTimeout(r, 100));
                if (!el.editor || el.value !== 'left base right') throw Error('document element did not open: ' + el.value);
                return true;
            })()"#, vec![serde_json::json!(repo)]).await?;
        }
        let (left, right) = tokio::join!(
            first.execute("const e = window.__reviewProse.editor; e.view.dispatch(e.view.state.tr.insertText('element ', 1)); return true", vec![]),
            second.execute("const e = window.__reviewProse.editor; e.view.dispatch(e.view.state.tr.insertText(' tail', e.view.state.doc.content.size - 1)); return true", vec![]),
        );
        left?;
        right?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = first.execute("return window.__reviewProse.value", vec![]).await?;
            let right = second.execute("return window.__reviewProse.value", vec![]).await?;
            if left.json() == "element left base right tail" && right.json() == left.json() { break; }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "prose elements did not converge: left={left:?}, right={right:?}");
            let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        eprintln!("AUTOMERGE REVIEW: real prose elements converged across two browser profiles");
        let table_base = get_json(&first, &table_path).await?;
        successful_body("read workbook", &table_base);
        // Request the second document before sweeping: this also exercises late bytes.
        let _ = get_json(&second, &table_path).await?;
        for _ in 0..3 {
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        for (browser, value) in [(&first, "one"), (&second, "two")] {
            let reply = post_json(browser, &table_path, serde_json::json!({"heads":table_base["body"]["heads"],"edits":[{"edit":"put","path":"sheets/s/cells/B2","value":value}]})).await?;
            successful_body("concurrent table cell", &reply);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = get_json(&first, &table_path).await?;
            let right = get_json(&second, &table_path).await?;
            if left["body"]["table"] == right["body"]["table"] && left["body"]["table"]["sheets"][0]["conflicts"] == serde_json::json!(["B2"]) { break; }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "table did not converge with conflict: left={left}, right={right}");
            let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        eprintln!("AUTOMERGE REVIEW: two-browser workbook converged with one B2 and conflict");

        // Persist an offline edit, close its tab, stop the worker, and
        // reopen the same profile. The in-memory dirty tracker is gone.
        let devtools = ChromeDevTools::new(first.handle.clone());
        devtools.execute_cdp("Network.enable").await?;
        devtools.execute_cdp_with_params("Network.emulateNetworkConditions", serde_json::json!({
            "offline": true, "latency": 0, "downloadThroughput": 0, "uploadThroughput": 0,
        })).await?;
        let current = get_json(&first, &text_path).await?;
        let offline = post_json(&first, &text_path, serde_json::json!({
            "heads": current["body"]["heads"],
            "request": {"id":"offline-restart", "time":123},
            "edits":[{"edit":"set-text", "text":"element left base right tail offline"}],
        })).await?;
        successful_body("offline document save", &offline);
        devtools.execute_cdp("ServiceWorker.enable").await?;
        devtools.execute_cdp("ServiceWorker.stopAllWorkers").await?;
        let old_tab = first.window().await?;
        let reopened = first.new_tab().await?;
        first.switch_to_window(old_tab).await?;
        first.close_window().await?;
        first.switch_to_window(reopened).await?;
        let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
        let not_yet = get_json(&second, &text_path).await?;
        anyhow::ensure!(not_yet["body"]["text"] == "element left base right tail", "offline edit reached the other profile before reconnection: {not_yet}");
        let devtools = ChromeDevTools::new(first.handle.clone());
        devtools.execute_cdp_with_params("Network.emulateNetworkConditions", serde_json::json!({
            "offline": false, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1,
        })).await?;
        goto(&first, env.tonk_web.as_str()).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = get_json(&first, &text_path).await?;
            let right = get_json(&second, &text_path).await?;
            if left["body"]["text"] == "element left base right tail offline" && right["body"]["text"] == left["body"]["text"] { break; }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "offline edit did not survive tab/worker restart and sync: left={left}, right={right}");
            let _ = post_json(&first, "/api/sync", serde_json::json!({})).await?;
            let _ = post_json(&second, "/api/sync", serde_json::json!({})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        eprintln!("AUTOMERGE REVIEW: offline edit survived tab close, worker restart, and remote reconnection");
        Ok(())
    }.await;
    if result.is_err() {
        dump_browser_log(&first, &env).await;
        dump_browser_log(&second, &env).await;
    }
    let _ = first.quit().await;
    let _ = second.quit().await;
    result
}
