// Runs the test access service standalone for local development/benchmarking.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tonk_access_service::helpers::{AccessServiceSettings, access_service};

    let service = access_service(AccessServiceSettings::default()).await?;
    let url = &service.address.access_service_url;
    println!("ACCESS_SERVICE_URL={url}");
    std::io::Write::flush(&mut std::io::stdout())?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
