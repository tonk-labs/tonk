// Runs the account service standalone for local development/benchmarking.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::time::Duration;
    use tonk_account_service::helpers::AccountServer;

    let server = AccountServer::start().await;
    let url = &server.endpoint;
    println!("ACCOUNT_SERVICE_URL={url}");
    std::io::Write::flush(&mut std::io::stdout())?;

    // Verification codes are captured rather than sent, so without this the
    // sign-up flow is unfinishable in a browser: the page asks for a code that
    // exists only inside this process's memory. Draining them to stdout is the
    // whole reason a human (or a driver script) can sign up against a local
    // service at all.
    let emails = server.emails.clone();
    tokio::spawn(async move {
        loop {
            let captured: Vec<(String, String)> = {
                let mut inbox = emails.0.lock().expect("captured email mutex poisoned");
                std::mem::take(&mut *inbox)
            };
            for (address, code) in captured {
                println!("ACCOUNT_VERIFICATION_CODE {address} {code}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
