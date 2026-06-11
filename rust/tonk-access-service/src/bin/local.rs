// Runs the test access service standalone for local development/benchmarking.

use tonk_access_service::helpers::{AccessServiceSettings, access_service};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let url = &service.address.access_service_url;
    println!("ACCESS_SERVICE_URL={url}");
    std::io::Write::flush(&mut std::io::stdout())?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}
