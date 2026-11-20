use crate::{
    capture::{self, Capture},
    eds::{self, Profile},
    integration::{Counters as IntegrationCounters, Integration, MqttOptions},
    model::{self, ByteSwap, CaptureField, CaptureMode, Kind, Selection, Settings},
    preflight,
    scanner::{
        self, Connection, ExpectedIdentity, Identity, IdentityPin, Reconnect, Schedule, SendTiming,
        Spacing, Status,
    },
    settings_file::{self, NamedDevice, Preferences, Session, Workspace},
};
use eframe::egui::{self, Color32, RichText, Ui};
use std::{
    fs,
    io::ErrorKind,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{atomic::Ordering::Relaxed, mpsc},
    time::{Duration, Instant},
};

mod app;
mod benchmark;
mod configuration;
mod device;
mod explore;
mod logging;
mod mapping;
mod monitor;
mod outputs;
mod run_report;
mod setup;
mod software;
use app::App;
use benchmark::Benchmark;
mod workspace;
use workspace::Tab;

const CYAN: Color32 = Color32::from_rgb(99, 226, 213);
const AMBER: Color32 = Color32::from_rgb(247, 191, 101);
const RED: Color32 = Color32::from_rgb(255, 119, 119);

pub fn run() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let benchmarks = if args.len() == 3 && args[0] == "--loopback-benchmark" && args[1] == "dual" {
        let root = PathBuf::from(&args[2]);
        [
            ("healthy", Ipv4Addr::LOCALHOST, 8_000),
            ("stalled", Ipv4Addr::new(127, 0, 0, 2), 16_000),
        ]
        .into_iter()
        .map(|(name, address, rpi)| Benchmark {
            schedule: Schedule::Deadline,
            directory: root.join(name),
            started: Instant::now(),
            stopping: false,
            owner: false,
            profile: None,
            reconnect: false,
            transitioned: false,
            target: Some((address, rpi)),
        })
        .collect()
    } else if matches!(args.len(), 3 | 4) && args[0] == "--loopback-benchmark" {
        let schedule = if args[1] == "tokio" {
            Schedule::Tokio
        } else {
            Schedule::Deadline
        };
        vec![Benchmark {
            schedule,
            directory: PathBuf::from(&args[2]),
            started: Instant::now(),
            stopping: false,
            owner: args.len() == 3 && args[1] == "owner",
            profile: args.get(3).map(PathBuf::from),
            reconnect: args[1] == "reconnect",
            transitioned: false,
            target: None,
        }]
    } else {
        Vec::new()
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 860.0])
            .with_min_inner_size([900.0, 580.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Assembly Scanner",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            let mut style = (*cc.egui_ctx.style()).clone();
            for font in style.text_styles.values_mut() {
                font.size += 1.0;
            }
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = Color32::from_rgb(22, 28, 37);
            style.visuals.window_fill = Color32::from_rgb(28, 36, 46);
            style.visuals.selection.bg_fill = Color32::from_rgb(22, 83, 87);
            style.spacing.item_spacing = egui::vec2(9.0, 7.0);
            cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
            Ok(Box::new(App::new(benchmarks, Workspace::path())))
        }),
    )
}

struct Device {
    id: u64,
    name: String,
    tab: Tab,
    profile_dialog: bool,
    connect_dialog: bool,
    explorer: explore::Explorer,
    settings: Settings,
    device: String,
    local: String,
    eds_path: String,
    toml_path: String,
    replace_profile: Option<PathBuf>,
    profiles: Vec<Profile>,
    selected: Option<usize>,
    bench_confirmed: bool,
    writes_confirmed: bool,
    output_entries: Vec<String>,
    configuration_entries: Vec<String>,
    preview_hex: String,
    connection: Option<Connection>,
    integration: Option<Integration>,
    integration_counters: Option<IntegrationCounters>,
    active: Option<Settings>,
    status: Status,
    generation: u64,
    tx: u64,
    skipped: u64,
    tx_spacing: Spacing,
    send_timing: SendTiming,
    capture: Option<Capture>,
    capture_run: run_report::CaptureRun,
    slot: capture::Slot,
    capturing: bool,
    preferences: Preferences,
    notice: Option<String>,
    error: Option<String>,
    identity_task: Option<mpsc::Receiver<Result<Identity, String>>>,
    verified_identity: Option<IdentityPin>,
    reconnect: Option<Reconnect>,
    bench: Option<Benchmark>,
}
fn title(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.heading(title);
}
fn number<T: egui::emath::Numeric>(ui: &mut Ui, label: &str, value: &mut T) {
    ui.label(label);
    ui.add(egui::DragValue::new(value).speed(0.25));
    ui.end_row();
}

#[cfg(test)]
mod tests;
