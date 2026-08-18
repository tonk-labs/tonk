// Runs the test access service standalone for local development/benchmarking.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tonk_access_service::helpers::{AccessServiceSettings, access_service};
    use tonk_worker_api::DeploymentConfig;

    // Serve `/.well-known/tonk` when the account service's URLs are in
    // the environment. Without it the endpoint 404s, the browser reads
    // that as "deployment configuration is invalid", and the account
    // panel dead-ends — so a dev stack that starts an account service
    // must pass its URLs through.
    let deployment = match (
        std::env::var("ACCOUNT_SERVICE_URL"),
        std::env::var("REVOCATION_RELAY_URL"),
    ) {
        (Ok(account), Ok(revocations)) => Some(DeploymentConfig {
            account_service_url: account.parse()?,
            revocation_relay_url: revocations.parse()?,
            // Filled in by the server with its own generated identity.
            service_did: None,
        }),
        _ => None,
    };

    let service = access_service(AccessServiceSettings {
        deployment,
        // Behind a dev proxy the activation links must open on the page
        // origin, not this server's own port.
        public_origin: std::env::var("ACCESS_PUBLIC_ORIGIN").ok(),
        ..Default::default()
    })
    .await?;
    let url = &service.address.access_service_url;
    println!("ACCESS_SERVICE_URL={url}");
    // Activation emails are captured rather than sent; a human completing
    // registration against this server reads the links back from here.
    println!("ACCESS_SERVICE_DID={}", service.address.service_did);
    println!("ACCESS_ACTIVATION_EMAILS={url}/_test/emails");
    std::io::Write::flush(&mut std::io::stdout())?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
