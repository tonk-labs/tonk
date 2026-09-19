//! Digest a perf run: proxy JSONL + patched ACCESS_UCAN log -> report.
//!
//! Answers, with numbers: how many remote round trips a phase cost by
//! kind; which objects (blocks) were redeemed more than once (missing
//! single-flight); how serialized the traffic was; and what the load
//! would cost at a modelled RTT.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct Request {
    t0: u64,
    t1: u64,
    dur_ms: u64,
    method: String,
    path: String,
    resp_bytes: u64,
    resp_sha: String,
}

struct Redeem {
    t: u64,
    command: String,
    subject: String,
    ok: bool,
    object: String,
}

/// `ACCESS_UCAN t=<ms> command=<c> subject=<s> authorized=<bool> object=<METHOD /path>`
fn parse_redeem(line: &str) -> Option<Redeem> {
    let rest = line.trim().strip_prefix("ACCESS_UCAN t=")?;
    let (t, rest) = rest.split_once(" command=")?;
    let (command, rest) = rest.split_once(" subject=")?;
    let (subject, rest) = rest.split_once(" authorized=")?;
    let (ok, object) = rest.split_once(" object=")?;
    Some(Redeem {
        t: t.parse().ok()?,
        command: command.to_string(),
        subject: subject.to_string(),
        ok: ok == "true",
        object: object.trim().to_string(),
    })
}

fn classify(path: &str) -> &'static str {
    if path.starts_with("/ucan") {
        "ucan-redeem"
    } else if path.starts_with("/api/") {
        "api"
    } else if path.starts_with("/.well-known/trunk/ws") {
        // hot-swap.js retrying trunk's live-reload websocket against a
        // server that answers 200: dev-only noise, kept out of the
        // asset numbers.
        "dev-noise"
    } else {
        "asset"
    }
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b}B"),
        b if b < 1024 * 1024 => format!("{}KB", b / 1024),
        b => format!("{:.1}MB", b as f64 / (1024.0 * 1024.0)),
    }
}

pub fn run(run_dir: &Path, phase: &str, rtt_ms: u64) -> anyhow::Result<()> {
    let window: Option<(u64, u64)> = if phase == "all" {
        None
    } else {
        let phases: HashMap<String, (u64, u64)> =
            serde_json::from_slice(&std::fs::read(run_dir.join("phases.json"))?)?;
        Some(*phases.get(phase).ok_or_else(|| {
            anyhow::anyhow!(
                "phase {phase:?} not in {:?}",
                phases.keys().collect::<Vec<_>>()
            )
        })?)
    };
    let in_window = |t: u64| window.is_none_or(|(t0, t1)| (t0..=t1).contains(&t));

    let requests: Vec<Request> = std::fs::read_to_string(run_dir.join("requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Request>(line).ok())
        .filter(|request| in_window(request.t0))
        .collect();
    let redeems: Vec<Redeem> = std::fs::read_to_string(run_dir.join("access.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(parse_redeem)
        .filter(|redeem| in_window(redeem.t))
        .collect();

    println!(
        "== perf report: {} phase={phase} ==",
        run_dir.file_name().and_then(|n| n.to_str()).unwrap_or("?")
    );
    if let Some((t0, t1)) = window {
        println!("   window: {} ms", t1 - t0);
    }

    if !requests.is_empty() {
        report_proxy(&requests);
    }
    if !redeems.is_empty() {
        report_redeems(&redeems, rtt_ms);
    }
    Ok(())
}

fn report_proxy(requests: &[Request]) {
    println!("\n-- proxy traffic (browser-side, everything but the S3 leg) --");
    let mut by_class: HashMap<&str, (u64, u64, u64)> = HashMap::new();
    for request in requests {
        let entry = by_class.entry(classify(&request.path)).or_default();
        entry.0 += 1;
        entry.1 += request.resp_bytes;
        entry.2 += request.dur_ms;
    }
    let mut classes: Vec<_> = by_class.into_iter().collect();
    classes.sort_by_key(|(_, (count, ..))| std::cmp::Reverse(*count));
    for (class, (count, bytes, duration)) in classes {
        println!(
            "   {class:12} {count:4} requests  {:>8}  {:6.0} ms avg",
            format_bytes(bytes),
            duration as f64 / count.max(1) as f64
        );
    }

    // Duplicate GETs: same path AND same content served more than once
    // (the hot-swap websocket probe is excluded as dev-only noise).
    let mut dupes: HashMap<(&str, &str), (u64, u64)> = HashMap::new();
    for request in requests {
        if request.method == "GET"
            && request.resp_bytes > 0
            && classify(&request.path) != "dev-noise"
        {
            let entry = dupes
                .entry((request.path.as_str(), request.resp_sha.as_str()))
                .or_insert((0, request.resp_bytes));
            entry.0 += 1;
        }
    }
    let wasted_requests: u64 = dupes.values().map(|(count, _)| count - 1).sum();
    let wasted_bytes: u64 = dupes.values().map(|(count, size)| (count - 1) * size).sum();
    println!(
        "   duplicate GETs (same path+content): {wasted_requests} wasted requests, {} wasted",
        format_bytes(wasted_bytes)
    );
    let mut repeats: Vec<_> = dupes.iter().filter(|(_, (count, _))| *count > 1).collect();
    repeats.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for ((path, _), (count, _)) in repeats.iter().take(8) {
        let path: String = path.chars().take(90).collect();
        println!("     {count}x {path}");
    }

    // Concurrency: how much busy wall time had exactly one request in
    // flight (the serialization signature) vs four or more.
    let mut events: Vec<(u64, i64)> = Vec::with_capacity(requests.len() * 2);
    for request in requests {
        events.push((request.t0, 1));
        events.push((request.t1, -1));
    }
    events.sort_unstable();
    let mut depth = 0i64;
    let mut last_t = None;
    let mut time_at: HashMap<i64, u64> = HashMap::new();
    for (t, delta) in events {
        if let Some(last) = last_t
            && depth > 0
        {
            *time_at.entry(depth.min(17)).or_default() += t - last;
        }
        depth += delta;
        last_t = Some(t);
    }
    let busy: u64 = time_at.values().sum();
    if busy > 0 {
        let serial = time_at.get(&1).copied().unwrap_or(0);
        let deep: u64 = time_at
            .iter()
            .filter(|(depth, _)| **depth >= 4)
            .map(|(_, ms)| ms)
            .sum();
        println!(
            "   busy time {busy} ms; single-request-in-flight {serial} ms ({:.0}% serialized)",
            100.0 * serial as f64 / busy as f64
        );
        println!(
            "   >=4 in flight: {deep} ms ({:.0}%)",
            100.0 * deep as f64 / busy as f64
        );
    }
}

fn report_redeems(redeems: &[Redeem], rtt_ms: u64) {
    println!("\n-- redeem log (1 redeem = 1 presigned S3 op; the block-fetch ground truth) --");
    let mut by_command: HashMap<&str, u64> = HashMap::new();
    for redeem in redeems {
        *by_command.entry(redeem.command.as_str()).or_default() += 1;
    }
    let mut commands: Vec<_> = by_command.into_iter().collect();
    commands.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (command, count) in commands {
        println!("   {command:28} {count}");
    }

    let gets: Vec<&Redeem> = redeems
        .iter()
        .filter(|redeem| redeem.ok && redeem.object.starts_with("GET"))
        .collect();
    let mut by_object: HashMap<&str, u64> = HashMap::new();
    for get in &gets {
        *by_object.entry(get.object.as_str()).or_default() += 1;
    }
    let wasted: u64 = by_object.values().map(|count| count - 1).sum();
    println!(
        "   GET permits: {} total, {} distinct objects, {wasted} repeat fetches ({:.0}% waste)",
        gets.len(),
        by_object.len(),
        100.0 * wasted as f64 / gets.len().max(1) as f64
    );
    let mut repeats: Vec<_> = by_object.iter().filter(|(_, count)| **count > 1).collect();
    repeats.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    for (object, count) in repeats.iter().take(10) {
        let object: String = object.chars().take(100).collect();
        println!("     {count}x {object}");
    }

    let mut by_subject: HashMap<String, u64> = HashMap::new();
    for redeem in redeems.iter().filter(|redeem| redeem.ok) {
        let tail: String = redeem
            .subject
            .chars()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        *by_subject.entry(tail).or_default() += 1;
    }
    println!("   by subject (last 8 chars): {by_subject:?}");

    if !gets.is_empty() {
        let mut times: Vec<u64> = redeems.iter().map(|redeem| redeem.t).collect();
        times.sort_unstable();
        let span = times.last().unwrap_or(&0) - times.first().unwrap_or(&0);
        println!("   redeem span: {span} ms for {} redeems", times.len());
        let serial_cost = 2.0 * rtt_ms as f64 * times.len() as f64 / 1000.0;
        println!(
            "   cost model @ {rtt_ms}ms RTT, fully serial, redeem+S3 (2 RTT/block): \
             {serial_cost:.0} s worst case; /16 if perfectly 16-wide: {:.0} s",
            serial_cost / 16.0
        );
    }
}
