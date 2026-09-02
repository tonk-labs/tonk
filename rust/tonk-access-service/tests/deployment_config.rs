#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use tonk_access_service::helpers::{AccessServiceSettings, access_service};
use tonk_worker_api::DeploymentConfig;

#[dialog_common::test]
async fn it_serves_deployment_config_when_configured() -> anyhow::Result<()> {
    let expected = DeploymentConfig::default();
    let service = access_service(AccessServiceSettings {
        deployment: Some(expected.clone()),
        ..Default::default()
    })
    .await?;

    let actual: DeploymentConfig = reqwest::get(format!(
        "{}/.well-known/tonk",
        service.address.access_service_url
    ))
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(
        actual.account_service_url, None,
        "a deployment advertises no account service"
    );
    // The server fills discovery with its own generated identity.
    assert_eq!(
        actual.service_did.as_deref(),
        Some(&*service.address.service_did)
    );

    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
async fn it_returns_not_found_without_deployment_config() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let response = reqwest::get(format!(
        "{}/.well-known/tonk",
        service.address.access_service_url
    ))
    .await?;
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
async fn it_advertises_account_setup_lifecycle_without_account_state() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let response = reqwest::get(format!(
        "{}/capabilities",
        service.address.access_service_url
    ))
    .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(
        response.json::<serde_json::Value>().await?,
        serde_json::json!({
            "service": "tonk-access-service",
            "capabilities": { "accountSetupLifecycle": 1 },
        })
    );

    service.stop().await?;
    Ok(())
}
