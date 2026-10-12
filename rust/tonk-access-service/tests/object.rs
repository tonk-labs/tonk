//! The retired object endpoint must never redeem residual bearer URLs.
#![cfg(feature = "integration-tests")]
use tonk_access_service::helpers::AccessServiceAddress;
#[dialog_common::test]
async fn old_object_urls_require_original_ucan(env: AccessServiceAddress) {
    let client = reqwest::Client::new();
    for method in [
        reqwest::Method::GET,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
    ] {
        let response = client
            .request(
                method,
                format!(
                    "{}/object/arbitrary?permit=legacy&signature=legacy",
                    env.access_service_url
                ),
            )
            .body("untrusted")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["kind"],
            "ReauthorizationRequired"
        );
    }
}

#[dialog_common::test]
async fn legacy_transport_has_actionable_guidance_and_versions_do_not_authorize(
    env: AccessServiceAddress,
) {
    let client = reqwest::Client::new();
    for version in [None, Some("0.6.15"), Some("999.0.0")] {
        let mut request = client
            .post(env.ucan_endpoint())
            .header("Content-Type", "application/cbor")
            .body(vec![0xa0]);
        if let Some(version) = version {
            request = request.header("X-Tonk-Version", version);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 426);
        let value = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(value["code"], "UpgradeRequired");
        let rejection: dialog_effects::Rejection = serde_json::from_value(value).unwrap();
        assert!(
            rejection
                .to_string()
                .contains("npm install -g @tonk/cli@0.6.17")
        );
    }
    let response = client
        .post(env.ucan_endpoint())
        .header("Authorization", "UCAN invalid")
        .header("X-Tonk-Version", "999.0.0")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        400,
        "claimed version never bypasses UCAN verification"
    );
}
