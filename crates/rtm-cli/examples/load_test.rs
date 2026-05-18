#[path = "../tests/common/mod.rs"]
mod common;
#[path = "support/spawn.rs"]
mod spawn_support;

use std::process::Command;

use anyhow::{Result, ensure};
use clap::Parser;
use lilo_rm_core::{Lifecycle, RuntimeKind, RuntimeResponse, SpawnTarget};

const DEFAULT_SESSIONS: usize = 50;
const APP_FOOTPRINT_LIMIT_KIB: u64 = 90 * 1024;
const NON_APP_FOOTPRINT_CATEGORIES: &[&str] =
    &["page table", "stack", "unused dyld shared cache area"];

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value_t = DEFAULT_SESSIONS)]
    sessions: usize,
    #[arg(long, value_name = "headless")]
    target: SpawnTarget,
}

struct FootprintSample {
    total_kib: u64,
    app_kib: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        matches!(args.target, SpawnTarget::Headless(_)),
        "load_test requires --target headless"
    );
    let harness = common::RtmHarness::start();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut lifecycles = Vec::with_capacity(args.sessions);

    for _ in 0..args.sessions {
        lifecycles.push(spawn_one(&runtime, &harness, args.target.clone()));
    }

    let pids = substrate_pids(harness.daemon_pid(), &lifecycles);
    let rss_kib = combined_rss_kib(&pids);
    let footprints = footprint_samples(&pids);
    let footprint_kib = footprints
        .iter()
        .map(|sample| sample.total_kib)
        .sum::<u64>();
    let app_footprint_kib = footprints.iter().map(|sample| sample.app_kib).sum::<u64>();
    println!("sessions={}", args.sessions);
    println!("combined_rss_kib={rss_kib}");
    println!("combined_rss_mib={:.2}", rss_kib as f64 / 1024.0);
    println!("combined_footprint_kib={footprint_kib}");
    println!(
        "combined_footprint_mib={:.2}",
        footprint_kib as f64 / 1024.0
    );
    println!("combined_app_footprint_kib={app_footprint_kib}");
    println!(
        "combined_app_footprint_mib={:.2}",
        app_footprint_kib as f64 / 1024.0
    );
    assert!(
        app_footprint_kib < APP_FOOTPRINT_LIMIT_KIB,
        "combined app footprint {app_footprint_kib} KiB exceeded {APP_FOOTPRINT_LIMIT_KIB} KiB"
    );
    Ok(())
}

fn spawn_one(
    runtime: &tokio::runtime::Runtime,
    harness: &common::RtmHarness,
    target: SpawnTarget,
) -> Lifecycle {
    let response = runtime
        .block_on(spawn_support::spawn_runtime(
            harness.socket_path(),
            uuid::Uuid::now_v7(),
            RuntimeKind::Claude,
            target,
        ))
        .expect("spawn rpc");
    match response {
        RuntimeResponse::Spawned { lifecycle, .. } => lifecycle,
        other => panic!("unexpected spawn response: {other:?}"),
    }
}

fn substrate_pids(daemon_pid: u32, lifecycles: &[Lifecycle]) -> Vec<u32> {
    std::iter::once(daemon_pid)
        .chain(lifecycles.iter().filter_map(|row| row.shim_pid))
        .collect()
}

fn combined_rss_kib(pids: &[u32]) -> u64 {
    pids.iter().copied().map(rss_kib).sum()
}

fn footprint_samples(pids: &[u32]) -> Vec<FootprintSample> {
    pids.iter().copied().map(footprint_sample).collect()
}

fn rss_kib(pid: u32) -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps rss");
    assert!(output.status.success(), "ps failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("rss stdout")
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("invalid rss for pid {pid}: {error}"))
}

fn footprint_sample(pid: u32) -> FootprintSample {
    let output = Command::new("footprint")
        .args(["-pid", &pid.to_string(), "-summary"])
        .output()
        .expect("footprint summary");
    assert!(output.status.success(), "footprint failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("footprint stdout");
    parse_footprint_sample(&stdout).unwrap_or_else(|| panic!("invalid footprint for pid {pid}"))
}

fn parse_footprint_sample(stdout: &str) -> Option<FootprintSample> {
    let line = stdout.lines().find(|line| line.contains("Footprint:"))?;
    let (_, metric) = line.split_once("Footprint:")?;
    let total_kib = parse_kib(metric)?;
    let non_app_kib = NON_APP_FOOTPRINT_CATEGORIES
        .iter()
        .filter_map(|category| parse_category_kib(stdout, category))
        .sum::<u64>();
    Some(FootprintSample {
        total_kib,
        app_kib: total_kib.saturating_sub(non_app_kib),
    })
}

fn parse_category_kib(stdout: &str, category: &str) -> Option<u64> {
    let line = stdout
        .lines()
        .find(|line| line.trim_end().ends_with(category))?;
    parse_kib(line)
}

fn parse_kib(text: &str) -> Option<u64> {
    let mut fields = text.split_whitespace();
    let amount = fields.next()?.parse::<f64>().ok()?;
    let unit = fields.next()?;
    match unit {
        "B" => (amount / 1024.0).ceil() as u64,
        "KB" => amount.ceil() as u64,
        "MB" => (amount * 1024.0).ceil() as u64,
        "GB" => (amount * 1024.0 * 1024.0).ceil() as u64,
        _ => return None,
    }
    .into()
}
