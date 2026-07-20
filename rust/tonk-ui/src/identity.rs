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

    #[dialog_common::test]
    async fn it_creates_a_passkey_and_derives_a_stable_root_did(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;

        // A CTAP2.1 platform authenticator with PRF (hmac-secret), user
        // verification, and automatic presence — the shape of a real
        // passkey provider.
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

        // Wait for the ceremony hook: it installs as soon as the UI
        // wasm main runs, independent of service-worker readiness.
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
}
