//! Passkey ceremony tests against a CDP virtual authenticator.

#[allow(unused_imports)]
mod tests {
    use crate::helpers::TestEnvironment;
    use anyhow::Result;
    #[cfg(not(target_arch = "wasm32"))]
    use serde_json::json;
    #[cfg(not(target_arch = "wasm32"))]
    use thirtyfour::extensions::cdp::ChromeDevTools;
    #[cfg(not(target_arch = "wasm32"))]
    use thirtyfour::prelude::*;

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(feature = "integration-tests", feature = "web-integration-tests")
    ))]
    async fn driver_with_prf(env: TestEnvironment) -> Result<WebDriver> {
        let driver = env.driver().await?;
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools.execute_cdp("WebAuthn.enable").await?;
        devtools
            .execute_cdp_with_params(
                "WebAuthn.addVirtualAuthenticator",
                json!({
                    "options": {
                        "protocol": "ctap2",
                        "ctap2Version": "ctap2_1",
                        "transport": "internal",
                        "hasResidentKey": true,
                        "hasUserVerification": true,
                        "isUserVerified": true,
                        "hasPrf": true,
                        "automaticPresenceSimulation": true,
                    }
                }),
            )
            .await?;
        driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const wait = () =>
                    window.tonkIdentity ? done(true) : setTimeout(wait, 50);
                wait();
                "#,
                vec![],
            )
            .await?;
        Ok(driver)
    }

    #[dialog_common::test]
    async fn it_creates_a_passkey_and_derives_a_stable_root_did(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(env).await?;

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

    #[dialog_common::test]
    async fn it_builds_a_root_signed_account_creation_in_one_browser_ceremony(
        env: TestEnvironment,
    ) -> Result<()> {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::principal::Principal;
        use dialog_ucan_core::{DelegationChain, InvocationChain};

        let driver = driver_with_prf(env).await?;
        let device = Ed25519Signer::import(&[8u8; 32]).await?;
        let device_did = device.did().to_string();
        let output = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                window.tonkIdentity.createAccount({
                    email: "person@example.com",
                    code: "123456",
                    deviceDid: arguments[0],
                    deviceName: "test browser",
                })
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
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await?;
        assert_eq!(
            invocation.command().0,
            vec!["account".to_string(), "create".to_string()]
        );

        let delegation = hex::decode(ceremony["delegationHex"].as_str().unwrap())?;
        let delegation = DelegationChain::try_from(delegation.as_slice())?;
        assert_eq!(delegation.audience().to_string(), device_did);
        assert_eq!(delegation.issuer(), invocation.subject());

        driver.quit().await?;
        Ok(())
    }
}
