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
        // A deployment advertises the service that serves it and
        // nothing else: the account service it used to name is gone.
        assert!(result["body"]["accountServiceUrl"].is_null());
        assert!(
            result["body"]["serviceDid"].as_str().is_some(),
            "discovery names the access service's identity"
        );

        driver.quit().await?;
        Ok(())
    }
}
