use super::{AttemptReport, ExpectedIdentity, USAGE, interrupt, load_input_profile, report_text};
use assembly_scanner::{
    capture::Slot,
    integration::{Counters, Integration},
    scanner::{Diagnostics, Identity, InputRunOptions, run_input},
};
use serde::Serialize;
use std::{
    ffi::OsString,
    net::Ipv4Addr,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Serialize)]
struct Report {
    mode: &'static str,
    state: &'static str,
    adapter_ip: Ipv4Addr,
    local_ip: Ipv4Addr,
    #[serde(skip_serializing_if = "String::is_empty")]
    io_route: String,
    duration_seconds: Option<u64>,
    duration_reached: bool,
    interrupted: bool,
    requested_ot_us: u32,
    requested_to_us: u32,
    accepted: u64,
    sequence_gaps: u64,
    duplicates: u64,
    late: u64,
    invalid: u64,
    timeouts: u64,
    sent: u64,
    skipped_deadlines: u64,
    identity: Option<Identity>,
    integration: Counters,
    attempts: Vec<AttemptReport>,
}

pub(super) fn run(
    profile: &Path,
    expected: ExpectedIdentity,
    args: &[OsString],
) -> Result<bool, String> {
    let seconds = match args {
        [] => None,
        [seconds] => Some(
            seconds
                .to_string_lossy()
                .parse::<i64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .ok_or("Serve duration must be a positive whole number of seconds")?
                as u64,
        ),
        _ => return Err(USAGE.into()),
    };
    let (settings, preferences) = load_input_profile(profile, "Serve")?;
    let integration = Integration::start(&preferences.integration, &settings)?.ok_or(
        "Serve requires at least one HTTP, MQTT, or shared-memory feed in [app.integration]",
    )?;
    let started = Instant::now();
    let duration = seconds.map(Duration::from_secs);
    let mut duration_reached = false;
    let attempts = run_input(
        &settings,
        InputRunOptions {
            expected: expected.into(),
            schedule: preferences.schedule,
            auto_reconnect: preferences.auto_reconnect,
        },
        &Slot::default(),
        Some(integration.publisher()),
        || {
            if interrupt::requested() {
                return true;
            }
            duration_reached |= duration.is_some_and(|limit| started.elapsed() >= limit);
            duration_reached
        },
    )?;
    let counters = integration.shutdown();
    let final_status = attempts.last().map(|run| &run.status);
    let success = duration_reached
        && final_status.is_some_and(|status| {
            status.state == "Closed"
                && status.valid != 0
                && status
                    .forward_close
                    .as_ref()
                    .is_some_and(|close| close.is_confirmed())
        });
    let sum = |field: fn(&Diagnostics) -> u64| attempts.iter().map(field).sum();
    let report = Report {
        mode: "serve",
        state: final_status.map_or("Canceled", |status| status.state),
        adapter_ip: settings.device_ip,
        local_ip: settings.local_ip,
        io_route: settings.io_route.clone(),
        duration_seconds: seconds,
        duration_reached,
        interrupted: interrupt::requested(),
        requested_ot_us: settings.output_rpi_us,
        requested_to_us: settings.input_rpi_us,
        accepted: sum(|run| run.status.valid),
        sequence_gaps: sum(|run| run.status.gaps),
        duplicates: sum(|run| run.status.duplicates),
        late: sum(|run| run.status.late),
        invalid: sum(|run| run.status.invalid),
        timeouts: sum(|run| run.status.timeouts),
        sent: sum(|run| run.sent),
        skipped_deadlines: sum(|run| run.skipped),
        identity: final_status.and_then(|status| status.identity.clone()),
        integration: counters,
        attempts: attempts
            .iter()
            .enumerate()
            .map(|(index, run)| AttemptReport::from_diagnostics(index + 1, &settings, run))
            .collect(),
    };
    println!("{}", report_text(&report)?);
    Ok(success)
}
