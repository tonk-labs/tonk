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
