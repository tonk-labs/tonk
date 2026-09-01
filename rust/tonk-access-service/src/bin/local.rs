// Runs the test access service standalone for local development/benchmarking.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tonk_access_service::helpers::{AccessServiceSettings, access_service};
    use tonk_worker_api::DeploymentConfig;

    // Always serve `/.well-known/tonk`: the browser reads a 404 there
    // as "deployment configuration is invalid" and the account panel
    // dead-ends. The identity is filled in by the server itself.
    let deployment = Some(DeploymentConfig::default());

    let service = access_service(AccessServiceSettings {
        deployment,
        // Behind a dev proxy the activation links must open on the page
        // origin, not this server's own port.
        public_origin: std::env::var("ACCESS_PUBLIC_ORIGIN").ok(),
        // Persist customers, the service key, and a blob snapshot, so a
        // restarted dev service stops wiping the state clients hold
        // credentials against.
        state_dir: std::env::var("ACCESS_STATE_DIR")
            .ok()
            .map(std::path::PathBuf::from),
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
