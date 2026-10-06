//! Tonk in a native window.
//!
//! The browser build runs the worker as a service worker over IndexedDB,
//! and the top page is the shell that loads the profile and, inside it,
//! a space. This binary keeps that page and moves the worker out of the
//! browser: the worker runs in this process over filesystem storage, a
//! loopback server answers the page's `/api/...` fetches with it, and a
//! native window shows the page in the platform webview.
//!
//! The page learns it is hosted natively from `globalThis.tonkNativeHost`,
//! defined before any of its scripts run, and skips registering a
//! service worker.

mod server;
mod window;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use dialog_effects::storage::Directory;
use tonk_worker::native::{NativeWorker, set_service_origin};

use crate::server::Server;

/// Tonk in a native window.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The built UI: `trunk build` output of `rust/tonk-ui`.
    #[arg(long, env = "TONK_DESKTOP_DIST")]
    dist: PathBuf,

    /// Where profiles and spaces are kept. Defaults to `tonk-desktop`
    /// under the platform data directory, apart from the `tonk` CLI's
    /// store.
    #[arg(long, env = "TONK_DESKTOP_DATA")]
    data: Option<PathBuf>,

    /// Origin of the account and access services, such as
    /// `https://tonk.network`. Without one the app is local-only.
    #[arg(long, env = "TONK_DESKTOP_SERVICE")]
    service: Option<String>,

    /// Loopback port to serve on. Defaults to one the system picks.
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// Serve without opening a window, and log the launch URL. For
    /// driving the page from another browser in tests.
    #[arg(long)]
    serve_only: bool,

    /// With `--serve-only`, write the script the window would run before
    /// the page here, for the other browser to run in its place.
    #[arg(long, requires = "serve_only")]
    host_script: Option<PathBuf>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();

    let dist = args
        .dist
        .canonicalize()
        .with_context(|| format!("no built UI at {}", args.dist.display()))?;
    anyhow::ensure!(
        dist.join("index.html").is_file(),
        "{} has no index.html; build rust/tonk-ui with trunk first",
        dist.display()
    );

    let data = match args.data {
        Some(data) => data,
        None => dirs::data_dir()
            .context("no platform data directory; pass --data")?
            .join("tonk-desktop"),
    };
    let profiles = data.join("profiles");
    let spaces = data.join("spaces");
    std::fs::create_dir_all(&profiles)?;
    std::fs::create_dir_all(&spaces)?;
    // The worker keeps spaces under `Directory::Current`, which natively
    // is the working directory (see `tonk_worker::native`).
    std::env::set_current_dir(&spaces)
        .with_context(|| format!("cannot enter {}", spaces.display()))?;

    if let Some(service) = args.service {
        set_service_origin(service);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let profiles = profiles
        .to_str()
        .context("the data directory path is not UTF-8")?
        .to_owned();
    let server = runtime.block_on(async {
        let worker = NativeWorker::open(Directory::At(profiles))
            .await
            .context("failed to open the worker")?;
        let listener =
            tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, args.port)))
                .await
                .context("failed to bind the loopback port")?;
        let authority = listener.local_addr()?.to_string();
        let server = Server::new(authority, dist, worker.router, worker.state);
        let app = server.clone().app();
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("server stopped: {error}");
            }
        });
        anyhow::Ok(server)
    })?;

    if args.serve_only {
        // Logged to stderr: the worker logs to stdout, and a test reading
        // the URL must not have to pick it out of those.
        eprintln!("launch: {}", server.launch_url());
        // No window to load in: a test drives the page itself, and reads
        // where the worker sent it from here.
        tonk_worker::native::set_navigator(|href, replace| {
            eprintln!("navigate: {href} replace={replace}");
        });
        if let Some(path) = args.host_script {
            std::fs::write(&path, window::native_host_script(&server.origin()))?;
        }
        runtime.block_on(tokio::signal::ctrl_c())?;
        return Ok(());
    }

    window::run(runtime, server.launch_url(), server.origin())
}
