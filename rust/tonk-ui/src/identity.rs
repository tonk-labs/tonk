//! Passkey ceremony tests against a CDP virtual authenticator.

#[allow(unused_imports)]
mod tests {
    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    use crate::helpers::{TestEnvironment, driver_with_prf};
    use anyhow::{Result, anyhow};
    #[cfg(not(target_arch = "wasm32"))]
    use thirtyfour::prelude::*;

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    #[dialog_common::test]
    async fn it_serves_deployment_config_on_the_page_origin(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                fetch("/.well-known/tonk")
                    .then(async response => done({ status: response.status, body: await response.json() }))
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        let result = result.json();
        assert_eq!(result["status"], 200);
        assert_eq!(
            result["body"]["accountServiceUrl"].as_str(),
            Some(env.account_service.as_str())
        );

        driver.quit().await?;
        Ok(())
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    #[dialog_common::test]
    async fn it_creates_a_passkey_and_derives_a_stable_root_did(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;

        let created = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                window.tonkIdentity.createPasskey("tester")
                    .then((result) => done({ ok: result }))
                    .catch((error) => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        let created = created.json().clone();
        assert!(
            created.get("error").is_none(),
            "createPasskey failed: {created:?}",
        );

        let mut dids = Vec::new();
        for _ in 0..2 {
            let derived = driver
                .execute_async(
                    r#"
                    const done = arguments[arguments.length - 1];
                    window.tonkIdentity.deriveRootDid()
                        .then((did) => done({ did }))
                        .catch((error) => done({ error: String(error) }));
                    "#,
                    vec![],
                )
                .await?;
            let derived = derived.json().clone();
            let did = derived
                .get("did")
                .and_then(|did| did.as_str())
                .unwrap_or_else(|| panic!("deriveRootDid failed: {derived:?}"))
                .to_owned();
            dids.push(did);
        }
        assert!(
            dids[0].starts_with("did:key:z6Mk"),
            "expected an ed25519 did:key, got {}",
            dids[0],
        );
        assert_eq!(
            dids[0], dids[1],
            "the root did must be stable across derivations"
        );

        driver.quit().await?;
        Ok(())
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    #[dialog_common::test]
    async fn it_builds_a_root_signed_account_creation_in_one_browser_ceremony(
        env: TestEnvironment,
    ) -> Result<()> {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::principal::Principal;
        use dialog_ucan_core::promise::Promised;
        use dialog_ucan_core::{DelegationChain, InvocationChain};

        let driver = driver_with_prf(&env).await?;
        let device = Ed25519Signer::import(&[8u8; 32]).await?;
        let device_did = device.did().to_string();
        let output = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                window.tonkIdentity.createRoot({
                    deviceDid: arguments[0],
                    label: "person@example.com",
                })
                    .then((root) => window.tonkIdentity.createAccount({
                        email: "person@example.com",
                        code: "123456",
                        deviceDid: arguments[0],
                        deviceName: "test browser",
                        rootDid: root.rootDid,
                        credentialId: root.credentialId,
                        delegationHex: root.delegationHex,
                        remote: "https://accounts.example/ucan/",
                    }))
                    .then((result) => done({ ok: result }))
                    .catch((error) => done({ error: String(error) }));
                "#,
                vec![serde_json::Value::String(device_did.clone())],
            )
            .await?;
        let output = output.json().clone();
        assert!(output.get("error").is_none(), "ceremony failed: {output:?}");
        let ceremony = &output["ok"];
        assert_eq!(ceremony["deviceDid"], device_did);

        let invocation = hex::decode(ceremony["invocationHex"].as_str().unwrap())?;
        let invocation = InvocationChain::try_from(invocation.as_slice())?;
        invocation
            .verify(&dialog_credentials::DidKeyResolver)
            .await?;
        assert_eq!(
            invocation.command().0,
            vec!["account".to_string(), "create".to_string()]
        );

        let delegation = hex::decode(ceremony["delegationHex"].as_str().unwrap())?;
        let delegation = DelegationChain::try_from(delegation.as_slice())?;
        assert_eq!(delegation.audience().to_string(), device_did);
        assert_eq!(delegation.issuer(), invocation.subject());
        let descriptor = hex::decode(ceremony["descriptorHex"].as_str().unwrap())?;
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(&descriptor).await?;
        assert_eq!(descriptor.account_subject(), delegation.issuer());
        assert_eq!(
            invocation.arguments().get("repositoryDescriptor"),
            Some(&Promised::String(
                ceremony["descriptorHex"].as_str().unwrap().to_string()
            ))
        );

        driver.quit().await?;
        Ok(())
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    #[dialog_common::test]
    async fn it_creates_an_account_against_the_local_service(env: TestEnvironment) -> Result<()> {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::DelegationChain;
        use dialog_ucan_core::principal::Principal;

        let client = reqwest::Client::new();
        let email = "person@example.com";
        let driver = driver_with_prf(&env).await?;
        let device = Ed25519Signer::import(&[8u8; 32]).await?;
        let device_did = device.did().to_string();
        let output = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                window.tonkIdentity.createRoot({
                    deviceDid: arguments[0],
                    label: arguments[1],
                })
                    .then(root => window.tonkIdentity.createAccount({
                        email: arguments[1],
                        deviceDid: arguments[0],
                        deviceName: "test browser",
                        rootDid: root.rootDid,
                        credentialId: root.credentialId,
                        delegationHex: root.delegationHex,
                        remote: arguments[2],
                    }))
                    .then(result => done({ ok: result }))
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![
                    serde_json::Value::String(device_did),
                    serde_json::Value::String(email.to_string()),
                    serde_json::Value::String(env.tonk_web.join("ucan/")?.to_string()),
                ],
            )
            .await?;
        let output = output.json();
        assert!(output.get("error").is_none(), "ceremony failed: {output:?}");
        let invocation = hex::decode(output["ok"]["invocationHex"].as_str().unwrap())?;
        let response = client
            .post(env.account_service.join("accounts")?)
            .header(reqwest::header::CONTENT_TYPE, "application/cbor")
            .body(invocation)
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);

        let root = dialog_credentials::Ed25519Signer::import(&[10u8; 32]).await?;
        let competing_device = Ed25519Signer::import(&[12u8; 32]).await?;
        let delegation = tonk_identity::delegation::mint_device_delegation(
            root.clone(),
            &competing_device.did(),
        )
        .await?;
        let ceremony = tonk_identity::ceremony::create_account(
            root,
            email.to_string(),
            "competing-credential".to_string(),
            competing_device.did(),
            "competing device".to_string(),
            hex::encode(delegation.to_bytes()?),
            env.tonk_web.join("ucan/")?.to_string(),
            None,
        )
        .await?;
        let response = client
            .post(env.account_service.join("accounts")?)
            .header(reqwest::header::CONTENT_TYPE, "application/cbor")
            .body(hex::decode(ceremony.invocation_hex)?)
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
        let error: serde_json::Value = response.json().await?;
        assert_eq!(error["error"]["code"], "CONFLICT");
        assert_eq!(
            error["error"]["message"],
            tonk_account_service::core::accounts::EMAIL_TAKEN
        );

        driver.quit().await?;
        Ok(())
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    #[dialog_common::test]
    async fn it_builds_a_root_signed_cli_handoff(env: TestEnvironment) -> Result<()> {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::principal::Principal;
        use dialog_ucan_core::promise::Promised;
        use dialog_ucan_core::{DelegationChain, InvocationChain};
        use tonk_account::handoff::CompleteLinkCeremony;

        let driver = driver_with_prf(&env).await?;
        let browser = Ed25519Signer::import(&[8u8; 32]).await?;
        let cli = Ed25519Signer::import(&[9u8; 32]).await?;
        let browser_did = browser.did().to_string();
        let cli_did = cli.did().to_string();
        let output = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                window.tonkIdentity.createRoot({ deviceDid: arguments[0] })
                    .then(() => window.tonkIdentity.completeLink({
                        tokenHash: "handoff-hash",
                        deviceDid: arguments[1],
                        deviceName: "test terminal",
                    }))
                    .then((result) => done({ ok: result }))
                    .catch((error) => done({ error: String(error) }));
                "#,
                vec![
                    serde_json::Value::String(browser_did),
                    serde_json::Value::String(cli_did.clone()),
                ],
            )
            .await?;
        let output = output.json().clone();
        assert!(output.get("error").is_none(), "handoff failed: {output:?}");
        let raw = output["ok"].as_object().expect("ceremony is an object");
        assert_eq!(
            raw.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["invocationHex"]
        );
        let ceremony: CompleteLinkCeremony = serde_json::from_value(output["ok"].clone())?;
        let invocation = hex::decode(ceremony.invocation_hex)?;
        let invocation = InvocationChain::try_from(invocation.as_slice())?;
        invocation
            .verify(&dialog_credentials::DidKeyResolver)
            .await?;
        assert_eq!(
            invocation.command().0,
            vec![
                "account".to_string(),
                "link".to_string(),
                "complete".to_string()
            ]
        );
        assert_eq!(
            invocation.arguments().get("tokenHash"),
            Some(&Promised::String("handoff-hash".into()))
        );
        let delegation_hex = match invocation.arguments().get("delegation") {
            Some(Promised::String(delegation_hex)) => delegation_hex,
            other => panic!("expected delegation argument, got {other:?}"),
        };
        let delegation = hex::decode(delegation_hex)?;
        let delegation = DelegationChain::try_from(delegation.as_slice())?;
        assert_eq!(delegation.audience().to_string(), cli_did);

        driver.quit().await?;
        Ok(())
    }
}
