// Runs the account service standalone for local development/benchmarking.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tonk_account_service::helpers::AccountServer;

    let server = AccountServer::start().await;
    let url = &server.endpoint;
    println!("ACCOUNT_SERVICE_URL={url}");
    std::io::Write::flush(&mut std::io::stdout())?;

    // This service sends no mail. Signing up locally needs the access
    // service's activation link, which its own binary prints the inbox
    // for (`ACCESS_ACTIVATION_EMAILS`); there used to be a verification
    // code drained here, and the ceremony that asked for one is gone.

    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
