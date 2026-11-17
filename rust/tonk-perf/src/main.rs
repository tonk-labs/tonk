//! Cold-load replication perf harness.
//!
//! `proxy` fronts the whole dev stack: it serves the trunk-built dist
//! (SPA fallback) and reverse-proxies the access-service paths, adding
//! a configurable RTT and bandwidth cap to EVERY request — page and
//! service-worker fetches alike, which Chrome DevTools throttling does
//! not shape — and logs each request as JSONL with timing, size,
//! in-flight depth, and a response hash for duplicate detection.
//!
//! The one leg it cannot see is the presigned S3 GET/PUT, which goes
//! straight to LocalS3's own port; the patched tonk-access-local logs
//! those 1:1 via the redeem permits (`ACCESS_UCAN ... object=...`).
//!
//! `link` runs shaped links (`tonk_perf::link`) in front of each service
//! a run reaches, with a control server to change them mid-run. Unlike
//! `proxy` it works below HTTP, so it also shapes the live-update socket
//! and presigned object reads and writes.
//!
//! `analyze` digests a run directory (requests.jsonl + access.log +
//! phases.json) into the report bench/perf/run.sh prints.

mod analyze;
mod proxy;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "tonk perf harness: shaping front proxy and run analyzer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the dist and proxy the access service, shaping and logging
    /// every request.
    Proxy {
        /// Port to listen on (127.0.0.1)
        #[arg(long)]
        listen: u16,
        /// Access service base URL to proxy /ucan/ (and friends) to
        #[arg(long)]
        ucan: String,
        /// tonk-ui dist directory to serve
        #[arg(long)]
        root: PathBuf,
        /// Full round trip added per request, in milliseconds
        #[arg(long, default_value_t = 0)]
        latency_ms: u64,
        /// Response bandwidth cap; 0 is unlimited
        #[arg(long, default_value_t = 0)]
        bandwidth_kbps: u64,
        /// JSONL request log path (appended)
        #[arg(long)]
        log: PathBuf,
    },
    /// Run shaped links to the services a run reaches, and a control
    /// server to change them while it runs.
    ///
    /// Prints `LINK_READY <name> <url>` for each link, then
    /// `LINK_CONTROL <url>`.
    Link {
        /// A link, as `name=listen:upstream`, such as
        /// `ucan=8801:127.0.0.1:41234`; listen 0 picks a port. Repeat for
        /// each service.
        #[arg(long = "link", required = true)]
        links: Vec<String>,
        /// The profile every link starts with (`tonk-perf link --profiles`
        /// lists them)
        #[arg(long, default_value = "lan")]
        profile: String,
        /// Port of the control server; 0 picks one
        #[arg(long, default_value_t = 0)]
        control: u16,
        /// JSONL log of every HTTP exchange the links carry
        #[arg(long)]
        log: Option<PathBuf>,
    },
    /// List the link profiles.
    Profiles,
    /// Digest a run directory into a report.
    Analyze {
        run_dir: PathBuf,
        /// Phase from phases.json to window on, or "all"
        #[arg(long, default_value = "all")]
        phase: String,
        /// Modelled 3G round trip for the cost estimate
        #[arg(long, default_value_t = 400)]
        rtt_ms: u64,
    },
    /// Print the current unix time in milliseconds.
    Now,
    /// Record a phase window into RUN_DIR/phases.json.
    Phase {
        run_dir: PathBuf,
        name: String,
        t0: u64,
        t1: u64,
    },
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as u64)
        .unwrap_or(0)
}

fn record_phase(run_dir: &std::path::Path, name: &str, t0: u64, t1: u64) -> anyhow::Result<()> {
    let path = run_dir.join("phases.json");
    let mut phases: serde_json::Map<String, serde_json::Value> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(_) => Default::default(),
    };
    phases.insert(name.to_string(), serde_json::json!([t0, t1]));
    std::fs::write(&path, serde_json::to_vec(&phases)?)?;
    Ok(())
}

fn run_links(
    specs: &[String],
    profile: &str,
    control: u16,
    log: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    use tonk_perf::link::{Link, Options, Shape, check_upstream, serve_control};

    let shape =
        Shape::profile(profile).ok_or_else(|| anyhow::anyhow!("no link profile {profile:?}"))?;
    let mut options = Vec::new();
    for spec in specs {
        let (name, rest) = spec
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--link {spec}: expected name=listen:upstream"))?;
        let (listen, upstream) = rest
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("--link {spec}: expected name=listen:upstream"))?;
        check_upstream(upstream).map_err(|error| anyhow::anyhow!("--link {spec}: {error}"))?;
        options.push(Options {
            name: name.to_string(),
            listen: listen.parse()?,
            upstream: upstream.to_string(),
            shape: shape.clone(),
        });
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let mut links = Vec::new();
        for options in options {
            let name = options.name.clone();
            let link = Link::start_with(options, log).await?;
            println!("LINK_READY {name} {}", link.url());
            links.push(link);
        }
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", control)).await?;
        println!("LINK_CONTROL http://{}", listener.local_addr()?);
        std::io::Write::flush(&mut std::io::stdout())?;
        let handles = links.iter().map(Link::handle).collect();
        tokio::select! {
            _ = serve_control(listener, handles) => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        drop(links);
        Ok(())
    })
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Proxy {
            listen,
            ucan,
            root,
            latency_ms,
            bandwidth_kbps,
            log,
        } => proxy::run(listen, ucan, root, latency_ms, bandwidth_kbps, log),
        Command::Link {
            links,
            profile,
            control,
            log,
        } => run_links(&links, &profile, control, log.as_deref()),
        Command::Profiles => {
            for (name, what) in tonk_perf::link::PROFILES {
                println!("{name}\t{what}");
            }
            Ok(())
        }
        Command::Analyze {
            run_dir,
            phase,
            rtt_ms,
        } => analyze::run(&run_dir, &phase, rtt_ms),
        Command::Now => {
            println!("{}", now_ms());
            Ok(())
        }
        Command::Phase {
            run_dir,
            name,
            t0,
            t1,
        } => record_phase(&run_dir, &name, t0, t1),
    }
}
