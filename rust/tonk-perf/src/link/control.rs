//! The control server a harness changes its links through, over plain
//! HTTP, so a shell script or a browser test can switch the network
//! mid-run.
//!
//! - `GET /__gate?mode=open|latency|stall|offline[&ms=N]`: as
//!   `bench/trace/gate.mjs` takes it, applied to the link's base shape
//!   (its starting profile, or the last `/profile`), so `open` returns to
//!   the run's path rather than to no shaping.
//! - `GET /profile?name=<profile>` and any of `mode`, `latency_ms`,
//!   `jitter_ms`, `kbps`, `stall_rate`, `stall_ms`, `reset_rate`,
//!   `idle_cut_ms`, `seed`: change the shape, and make it the base.
//! - `GET /reset`: reset every connection, as a network change does.
//! - `GET /stats`: every link's [`Stats`](super::Stats) as JSON.
//! - `GET /profiles`: the named profiles.
//!
//! Each change applies to every link, or to the one named by `link=`.

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::{Handle, Mode, PROFILES, Shape};

/// Serve the control of `links` on `listener` until the task is dropped.
pub async fn serve(listener: TcpListener, links: Vec<Handle>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let links = links.clone();
        tokio::spawn(async move {
            let _ = answer(stream, &links).await;
        });
    }
}

async fn answer(mut stream: TcpStream, links: &[Handle]) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..n]);
    }
    let line = String::from_utf8_lossy(&head);
    let target = line.split(' ').nth(1).unwrap_or("/");
    let (status, body) = match route(target, links) {
        Ok(body) => ("200 OK", body),
        Err(error) => ("400 Bad Request", format!("{error}\n")),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\n\
         access-control-allow-origin: *\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

fn route(target: &str, links: &[Handle]) -> Result<String, String> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), decode(value))
        })
        .collect();
    let chosen: Vec<&Handle> = match pairs.iter().find(|(key, _)| key == "link") {
        Some((_, name)) => {
            let named: Vec<&Handle> = links.iter().filter(|link| link.name() == name).collect();
            if named.is_empty() {
                return Err(format!("no link {name:?}"));
            }
            named
        }
        None => links.iter().collect(),
    };
    match path {
        "/__gate" | "/profile" => {
            let mut said = String::new();
            for link in &chosen {
                let shape = if path == "/__gate" {
                    // The gate switches the network around the path the
                    // run chose: `open` goes back to it, `ms` overrides
                    // its round trip.
                    let mut pairs = pairs.clone();
                    if !pairs.iter().any(|(key, _)| key == "mode") {
                        pairs.push(("mode".into(), "open".into()));
                    }
                    link.base().with(&pairs)?
                } else {
                    let shape = link.shape().with(&pairs)?;
                    link.set_base(Shape {
                        mode: Mode::Open,
                        ..shape.clone()
                    });
                    shape
                };
                said.push_str(&format!(
                    "{} {:?} {} ms ±{} ms {} kbps\n",
                    link.name(),
                    shape.mode,
                    shape.latency.as_millis(),
                    shape.jitter.as_millis(),
                    shape.kbps
                ));
                link.set(shape);
            }
            Ok(said)
        }
        "/reset" => {
            for link in &chosen {
                link.reset();
            }
            Ok("reset\n".into())
        }
        "/stats" => {
            let stats: Vec<_> = chosen.iter().map(|link| link.stats()).collect();
            serde_json::to_string_pretty(&stats).map_err(|error| error.to_string())
        }
        "/profiles" => Ok(PROFILES
            .iter()
            .map(|(name, what)| format!("{name}\t{what}\n"))
            .collect()),
        other => Err(format!("no control at {other}")),
    }
}

/// Percent-decode a query component.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
