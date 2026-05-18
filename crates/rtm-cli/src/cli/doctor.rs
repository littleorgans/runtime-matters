use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use lilo_rm_core::{
    DoctorResponse, LifecycleCounts, RuntimeCapability, RuntimeResponse, RuntimeRpc,
};

pub async fn run() -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(&socket_path, RuntimeRpc::Doctor).await?;
    match response {
        RuntimeResponse::Doctor { doctor } => {
            print_doctor(&doctor);
            Ok(())
        }
        other => bail!("unexpected doctor response: {other:?}"),
    }
}

fn print_doctor(doctor: &DoctorResponse) {
    println!("rtmd");
    println!(
        "  version             {} (git: {})",
        doctor.version.version, doctor.version.git_sha
    );
    println!("  protocol            {}", doctor.version.protocol_version);
    println!(
        "  capabilities        {}",
        format_capabilities(&doctor.version.capabilities)
    );
    println!("  socket              {}", doctor.socket_path);
    println!(
        "  uptime              {}",
        format_duration(doctor.uptime_secs)
    );
    println!("sqlite");
    println!(
        "  applied migrations  {} of {} ({})",
        doctor.sqlite.applied,
        doctor.sqlite.total,
        format_migrations(&doctor.sqlite.applied_descriptions)
    );
    if !doctor.sqlite.pending_descriptions.is_empty() {
        println!(
            "  pending migrations  {}",
            format_migrations(&doctor.sqlite.pending_descriptions)
        );
    }
    print_lifecycle_counts(&doctor.lifecycles);
    println!("kqueue watchers       {}", doctor.watchers.kqueue_watchers);
    println!("shim sockets          {}", doctor.watchers.shim_sockets);
    println!("launchers");
    for launcher in &doctor.launchers {
        let value = launcher
            .command
            .as_deref()
            .or(launcher.error.as_deref())
            .unwrap_or("unavailable");
        println!("  {:<18} {}", launcher.runtime, value);
    }
    println!("tmux                  {}", format_tmux(doctor));
    println!(
        "last probe sweep      {}",
        format_optional_time(doctor.last_probe_sweep)
    );
    print_recent_lost(doctor);
}

fn print_lifecycle_counts(counts: &LifecycleCounts) {
    println!("lifecycles");
    println!("  forking             {}", counts.forking);
    println!("  running             {}", counts.running);
    println!("  exited              {}", counts.exited);
    println!("  lost                {}", counts.lost);
}

fn print_recent_lost(doctor: &DoctorResponse) {
    if doctor.recent_lost.is_empty() {
        println!("recent lost           (none in last 24h)");
        return;
    }
    println!("recent lost");
    for event in &doctor.recent_lost {
        println!(
            "  {} {} {}",
            event.session_id,
            event.evidence,
            event.occurred_at.to_rfc3339()
        );
    }
}

fn format_migrations(values: &[String]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values.join(", ")
}

fn format_capabilities(values: &[RuntimeCapability]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_tmux(doctor: &DoctorResponse) -> String {
    if doctor.tmux.available {
        let version = doctor.tmux.version.as_deref().unwrap_or("version unknown");
        return format!("available ({version})");
    }
    match doctor.tmux.error.as_deref() {
        Some(error) => format!("unavailable ({error})"),
        None => "unavailable".to_owned(),
    }
}

fn format_optional_time(value: Option<DateTime<Utc>>) -> String {
    match value {
        Some(time) => format!("{} ({} ago)", time.to_rfc3339(), format_age(time)),
        None => "never".to_owned(),
    }
}

fn format_age(time: DateTime<Utc>) -> String {
    let seconds = (Utc::now() - time).num_seconds().max(0) as u64;
    format_duration(seconds)
}

fn format_duration(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
