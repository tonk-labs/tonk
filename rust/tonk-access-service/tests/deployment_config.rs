#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use tonk_access_service::helpers::{AccessServiceSettings, access_service};
use tonk_worker_api::DeploymentConfig;

#[dialog_common::test]
async fn it_serves_deployment_config_when_configured() -> anyhow::Result<()> {
    let expected = DeploymentConfig {
        account_service_url: "http://127.0.0.1:4100".parse()?,
        revocation_relay_url: "http://127.0.0.1:4100/revocations".parse()?,
    };
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
    assert_eq!(actual, expected);

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
