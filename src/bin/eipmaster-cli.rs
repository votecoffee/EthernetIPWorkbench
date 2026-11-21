use assembly_scanner::{
    capture::{Capture, FieldSummary, MAX_CAPTURE_SECONDS, Options, Slot},
    discovery, eds,
    explicit::{self, ReadPath, Route, SavedReadMode},
    integration::{self, Integration},
    model::{ConnectionTimeout, Settings, parse_hex},
    preflight,
    scanner::{
        CloseOutcome, CyclicIo, Diagnostics, ExpectedIdentity as ScannerIdentity, Identity,
        IdentityKey, IdentityPin, InputRunOptions, ReceiveTimeout, Schedule, SendTiming, Spacing,
        StartupTiming, Status, read_identity, run_input,
    },
    settings_file::{self, Preferences},
};
use serde::Serialize;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::Write,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::Ordering::Relaxed,
    time::Instant,
};

#[path = "eipmaster-cli/interrupt.rs"]
mod interrupt;
#[path = "eipmaster-cli/qualification.rs"]
mod qualification;
#[path = "eipmaster-cli/reads.rs"]
mod reads;
#[path = "eipmaster-cli/serve.rs"]
mod serve;
#[path = "eipmaster-cli/writes.rs"]
mod writes;

const USAGE: &str = "\
Usage:
  eipmaster-cli eds <vendor.eds>
  eipmaster-cli compare-eds <settings.toml> <updated.eds>
  eipmaster-cli validate <settings.toml> [<vendor.eds> <ConnectionN>]
  eipmaster-cli decode-input <settings.toml> \"<assembly hex bytes>\"
  eipmaster-cli identity <adapter-or-bridge-ipv4> <local-ipv4> [--route <port/link/...>] [--eds <vendor.eds>]
  eipmaster-cli discover <local-ipv4> [<destination-ipv4>]
  eipmaster-cli read <adapter-or-bridge-ipv4> <local-ipv4> <class> <instance> <attribute|all> [--route <port/link/...>]
  eipmaster-cli class-info <adapter-or-bridge-ipv4> <local-ipv4> <class> [--route <port/link/...>]
  eipmaster-cli port-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <port/link/...>]
  eipmaster-cli tcpip-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <port/link/...>]
  eipmaster-cli link-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <port/link/...>]
  eipmaster-cli manager-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <port/link/...>]
  eipmaster-cli qos-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <port/link/...>]
  eipmaster-cli write-attribute <adapter-or-bridge-ipv4> <local-ipv4> <class> <instance> <attribute> \"<value>\" [--type <Kind>] [--endian <little|big>] [--swap <none|bytes_in_words|word_order>] [--text-bytes <count>] [--route <port/link/...>] [--expect-current \"<hex bytes>\"] [--readback] [--connected [--connection-size <bytes>]] [--save-draft <new.toml> | --apply <vendor:type:product:serial>]
  eipmaster-cli write-draft <draft.toml> [--verify | --inspect | --apply <vendor:type:product:serial>]
  eipmaster-cli write-receipt <receipt-or-check.json> [--check-current]
  eipmaster-cli read-saved <settings.toml> <name>
  eipmaster-cli read-saved-all <settings.toml> [--multiple-service-packet] [--connected]
  eipmaster-cli capture <settings.toml> <new-output-dir> <seconds> <vendor:type:product[:serial]>
  eipmaster-cli serve <settings.toml> <vendor:type:product[:serial]> [<seconds>]
  eipmaster-cli qualify <settings.toml> <new-output-dir> <direct|switched|loopback> <vendor:type:product:serial> [<loopback-seconds>]

Capture and serve require a read-only heartbeat profile (input-only or listen-only). The scheduler defaults to deadline.
Write-attribute and write-draft preview offline by default. --apply requires a reviewed full device identity.
Serve runs the enabled software feeds without CSV until a console stop or an optional wall-clock duration limit.
[app].auto_reconnect opts in to input-only recovery. Listen-only sessions use manual reconnect.
The expected Identity Object values are decimal and checked before Forward_Open.
Capture seconds must be an integer from 1 to 3600.
Qualification runs 15 seconds at 8 ms and three 60-second trials at 1 ms.
Only loopback qualification accepts a duration override from 1 to 60 seconds.
On Windows, Ctrl+C or Ctrl+Break closes I/O and flushes pending capture samples.";

fn main() -> ExitCode {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|arg| arg == "capture" || arg == "qualify" || arg == "serve")
        && let Err(error) = interrupt::install()
    {
        eprintln!("Console stop handler unavailable: {error}");
    }
    let code = match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    };
    if interrupt::requested() {
        ExitCode::from(130)
    } else {
        code
    }
}

fn run(args: &[OsString]) -> Result<bool, String> {
    match args {
        [command, local, target @ ..] if command == "discover" => {
            let destination = match target {
                [] => std::net::Ipv4Addr::BROADCAST,
                [address] => address
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "Invalid discovery destination IPv4 address")?,
                _ => return Err(USAGE.into()),
            };
            let local = local
                .to_string_lossy()
                .parse()
                .map_err(|_| "Invalid local IPv4 address")?;
            let report = discovery::discover(local, destination).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        [
            command,
            adapter,
            local,
            class,
            instance,
            attribute,
            options @ ..,
        ] if command == "read" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let path = ReadPath {
                class: explicit::parse_id(&class.to_string_lossy())?,
                instance: explicit::parse_id(&instance.to_string_lossy())?,
                attribute: if attribute == "all" {
                    None
                } else {
                    Some(explicit::parse_id(&attribute.to_string_lossy())?)
                },
            };
            reads::single(adapter, local, path, route.as_ref(), None)
        }
        [command, adapter, local, class, options @ ..] if command == "class-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_class_info(
                adapter,
                local,
                explicit::parse_id(&class.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [command, adapter, local, instance, options @ ..] if command == "port-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_port_info(
                adapter,
                local,
                explicit::parse_id(&instance.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [command, adapter, local, instance, options @ ..] if command == "tcpip-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_tcpip_info(
                adapter,
                local,
                explicit::parse_id(&instance.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [command, adapter, local, instance, options @ ..] if command == "link-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_link_info(
                adapter,
                local,
                explicit::parse_id(&instance.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [command, adapter, local, instance, options @ ..] if command == "manager-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_manager_info(
                adapter,
                local,
                explicit::parse_id(&instance.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [command, adapter, local, instance, options @ ..] if command == "qos-info" => {
            let (adapter, local, route) = explicit_endpoint(adapter, local, options)?;
            let report = explicit::read_qos_info(
                adapter,
                local,
                explicit::parse_id(&instance.to_string_lossy())?,
                route,
            )?;
            println!("{}", report.report_json()?);
            Ok(report.is_usable())
        }
        [
            command,
            adapter,
            local,
            class,
            instance,
            attribute,
            bytes,
            options @ ..,
        ] if command == "write-attribute" => {
            let path = ReadPath {
                class: explicit::parse_id(&class.to_string_lossy())?,
                instance: explicit::parse_id(&instance.to_string_lossy())?,
                attribute: Some(explicit::parse_id(&attribute.to_string_lossy())?),
            };
            writes::attribute(adapter, local, path, bytes, options)
        }
        [command, path, options @ ..] if command == "write-draft" => {
            writes::file(Path::new(path), options)
        }
        [command, path, options @ ..] if command == "write-receipt" => {
            writes::receipt(Path::new(path), options)
        }
        [command, profile, name] if command == "read-saved" => {
            let (settings, app) = settings_file::load_profile(Path::new(profile))?;
            let reads = app.unwrap_or_default().cip_reads;
            let read = reads
                .iter()
                .find(|read| read.name == name.to_string_lossy())
                .ok_or_else(|| format!("No saved CIP read named {:?}", name))?;
            reads::single(
                settings.device_ip,
                settings.local_ip,
                read.path,
                read.route.as_ref(),
                read.selection,
            )
        }
        [command, profile, options @ ..] if command == "read-saved-all" => {
            let mode = match options {
                [] => SavedReadMode::Sequential,
                [option] if option == "--multiple-service-packet" => SavedReadMode::Packet,
                [option] if option == "--connected" => SavedReadMode::Connected,
                [first, second]
                    if first == "--connected" && second == "--multiple-service-packet"
                        || first == "--multiple-service-packet" && second == "--connected" =>
                {
                    SavedReadMode::ConnectedPacket
                }
                _ => return Err(USAGE.into()),
            };
            let (settings, app) = settings_file::load_profile(Path::new(profile))?;
            let app = app.unwrap_or_default();
            reads::all(
                settings.device_ip,
                settings.local_ip,
                app.cip_reads,
                mode,
                app.cip_class3_size,
            )
        }
        [command, path] if command == "eds" => {
            let profiles = eds::load(Path::new(path))?;
            let result = EdsReport {
                device: profiles.first().and_then(|profile| profile.device.clone()),
                profiles: profiles
                    .into_iter()
                    .map(|p| {
                        let assembly_definition_sizes = ["O->T size", "T->O size"]
                            .into_iter()
                            .filter(|field| p.size_from_assembly_definition(field))
                            .collect();
                        EdsProfile {
                            id: p.id,
                            name: p.name,
                            supported: p.unsupported.is_empty(),
                            unsupported: p.unsupported,
                            assembly_definition_sizes,
                            input_layout: p.input_layout,
                            output_layout: p.output_layout,
                            configuration_layout: p.configuration_layout,
                            output_rpi_range: p.output_rpi_range,
                            input_rpi_range: p.input_rpi_range,
                            fields: p.fields,
                        }
                    })
                    .collect(),
            };
            print!(
                "{}",
                toml::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        [command, path, comparison @ ..] if command == "validate" => {
            let saved = settings_file::load_named_profile(Path::new(path))?;
            let profile = match comparison {
                [] => saved.eds_profile,
                [eds_path, id] => Some(
                    eds::load(Path::new(eds_path))?
                        .into_iter()
                        .find(|profile| profile.id.eq_ignore_ascii_case(&id.to_string_lossy()))
                        .ok_or_else(|| format!("EDS profile {:?} was not found", id))?,
                ),
                _ => return Err(USAGE.into()),
            };
            let report = preflight::inspect(
                &saved.settings,
                &saved.preferences.unwrap_or_default(),
                profile.as_ref(),
            );
            print!(
                "{}",
                toml::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            Ok(report.connection_valid)
        }
        [command, path, eds_path] if command == "compare-eds" => {
            let saved = settings_file::load_named_profile(Path::new(path))?;
            let prior = saved.eds_profile.ok_or(
                "The settings file has no saved EDS profile. Save a profile with an EDS connection selected.",
            )?;
            let report = prior.compare_revision(&eds::load(Path::new(eds_path))?)?;
            print!("{}", report_text(&report)?);
            Ok(true)
        }
        [command, profile, payload] if command == "decode-input" => {
            let (settings, _) = settings_file::load_profile(Path::new(profile))?;
            let bytes = parse_hex(&payload.to_string_lossy())?;
            print!("{}", report_text(&settings.preview_input(&bytes)?)?);
            Ok(true)
        }
        [command, adapter, local, options @ ..] if command == "identity" => {
            let mut io_route = String::new();
            let mut eds_device = None;
            for pair in options.chunks(2) {
                match pair {
                    [option, route] if option == "--route" && io_route.is_empty() => {
                        io_route = route.to_string_lossy().parse::<Route>()?.to_string();
                    }
                    [option, path] if option == "--eds" && eds_device.is_none() => {
                        eds_device =
                            Some(eds::load_device_identity(Path::new(path))?.unwrap_or_default());
                    }
                    _ => return Err(USAGE.into()),
                }
            }
            let settings = Settings {
                device_ip: adapter
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "Invalid adapter IPv4 address")?,
                local_ip: local
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "Invalid local IPv4 address")?,
                io_route,
                ..Settings::default()
            };
            settings.validate()?;
            let identity = read_identity(&settings)?;
            let result = IdentityReport {
                adapter_ip: settings.device_ip,
                local_ip: settings.local_ip,
                io_route: settings.io_route.clone(),
                eds_comparison: eds_device.as_ref().map(|device| device.compare(&identity)),
                eds_device,
                identity,
            };
            print!(
                "{}",
                toml::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        [command, profile, directory, seconds, expected] if command == "capture" => {
            let seconds = seconds
                .to_string_lossy()
                .parse::<u64>()
                .ok()
                .filter(|n| (1..=MAX_CAPTURE_SECONDS).contains(n))
                .ok_or("Seconds must be an integer from 1 to 3600")?;
            let expected = parse_expected_identity(&expected.to_string_lossy())?;
            let (settings, preferences) = load_capture_profile(Path::new(profile))?;
            let report = capture(
                &settings,
                &preferences,
                Path::new(directory),
                seconds,
                expected,
            )?;
            println!("{}", report_text(&report)?);
            Ok(report.completed())
        }
        [command, profile, directory, path, expected, seconds @ ..] if command == "qualify" => {
            qualification::run(
                Path::new(profile),
                Path::new(directory),
                &path.to_string_lossy(),
                parse_expected_identity(&expected.to_string_lossy())?,
                seconds,
            )
        }
        [command, profile, expected, seconds @ ..] if command == "serve" => serve::run(
            Path::new(profile),
            parse_expected_identity(&expected.to_string_lossy())?,
            seconds,
        ),
        [] => {
            println!("{USAGE}");
            Ok(true)
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            Ok(true)
        }
        _ => Err(USAGE.into()),
    }
}

#[derive(Clone, Copy)]
struct ExpectedIdentity {
    key: IdentityKey,
    serial: Option<u32>,
}

impl From<ExpectedIdentity> for ScannerIdentity {
    fn from(expected: ExpectedIdentity) -> Self {
        match expected.serial {
            Some(serial) => Self::Device(IdentityPin {
                key: expected.key,
                serial,
            }),
            None => Self::Model(expected.key),
        }
    }
}

fn explicit_endpoint(
    adapter: &OsStr,
    local: &OsStr,
    options: &[OsString],
) -> Result<(Ipv4Addr, Ipv4Addr, Option<Route>), String> {
    let route = match options {
        [] => None,
        [option, route] if option == "--route" => Some(route.to_string_lossy().parse()?),
        _ => return Err(USAGE.into()),
    };
    let adapter = adapter
        .to_string_lossy()
        .parse()
        .map_err(|_| "Invalid adapter IPv4 address")?;
    let local = local
        .to_string_lossy()
        .parse()
        .map_err(|_| "Invalid local IPv4 address")?;
    Ok((adapter, local, route))
}

fn parse_expected_identity(value: &str) -> Result<ExpectedIdentity, String> {
    let error = "Expected decimal vendor:type:product or vendor:type:product:serial";
    let mut parts = value.split(':');
    let mut model_part = || {
        parts
            .next()
            .and_then(|part| part.parse::<u16>().ok())
            .ok_or(error)
    };
    let key = (model_part()?, model_part()?, model_part()?);
    let serial = parts
        .next()
        .map(str::parse::<u32>)
        .transpose()
        .map_err(|_| error)?;
    if parts.next().is_some() {
        return Err(error.into());
    }
    Ok(ExpectedIdentity { key, serial })
}

fn report_text(report: &impl Serialize) -> Result<String, String> {
    toml::to_string_pretty(report).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct EdsReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<eds::DeviceIdentity>,
    profiles: Vec<EdsProfile>,
}

#[derive(Serialize)]
struct EdsProfile {
    id: String,
    name: String,
    supported: bool,
    unsupported: Vec<String>,
    assembly_definition_sizes: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_layout: Option<eds::InputLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_layout: Option<eds::AssemblyLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    configuration_layout: Option<eds::AssemblyLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_rpi_range: Option<eds::RpiRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_rpi_range: Option<eds::RpiRange>,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
struct IdentityReport {
    adapter_ip: std::net::Ipv4Addr,
    local_ip: std::net::Ipv4Addr,
    #[serde(skip_serializing_if = "String::is_empty")]
    io_route: String,
    identity: Identity,
    #[serde(skip_serializing_if = "Option::is_none")]
    eds_device: Option<eds::DeviceIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eds_comparison: Option<eds::IdentityComparison>,
}

#[derive(Serialize)]
struct Report {
    integration: Option<integration::Counters>,
    state: &'static str,
    adapter_ip: std::net::Ipv4Addr,
    local_ip: std::net::Ipv4Addr,
    #[serde(skip_serializing_if = "String::is_empty")]
    io_route: String,
    input_assembly: u16,
    input_size: u16,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    input_multicast: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    default_unicast_port: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    listen_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_min_size: Option<u16>,
    duration_seconds: u64,
    duration_reached: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    interrupted: bool,
    schedule: Schedule,
    requested_ot_us: u32,
    requested_to_us: u32,
    actual_ot_us: Option<u32>,
    actual_to_us: Option<u32>,
    connection_timeout: ConnectionTimeout,
    accepted: u64,
    sequence_gaps: u64,
    duplicates: u64,
    late: u64,
    invalid: u64,
    timeouts: u64,
    receive_timeout: Option<ReceiveTimeout>,
    forward_close: Option<CloseOutcome>,
    sent: u64,
    skipped_deadlines: u64,
    seen: u64,
    after_capture_deadline: u64,
    unchanged: u64,
    written: u64,
    buffer_overruns: u64,
    last_value: Option<String>,
    last_input_hex: Option<String>,
    csv_prefix: String,
    capture_start_utc: String,
    csv_parts: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    field_summaries: Vec<FieldSummary>,
    identity_error: Option<String>,
    cip_error: Option<String>,
    network_error: Option<String>,
    disk_error: Option<String>,
    identity: Option<Identity>,
    cyclic_io: Option<CyclicIo>,
    startup_timing: Option<StartupTiming>,
    receive_spacing: Spacing,
    send_spacing: Spacing,
    send_timing: SendTiming,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attempts: Vec<AttemptReport>,
}

impl Report {
    fn completed(&self) -> bool {
        // The cutoff prevents another attempt from starting after the final window closes.
        let final_accepted = self
            .attempts
            .last()
            .map_or(self.accepted, |attempt| attempt.accepted);
        self.state == "Closed"
            && self
                .forward_close
                .as_ref()
                .is_some_and(CloseOutcome::is_confirmed)
            && self.duration_reached
            && !self.interrupted
            && final_accepted > self.after_capture_deadline
            && self.written != 0
            && self.seen + self.after_capture_deadline == self.accepted
            && self.written + self.unchanged == self.seen
            && self.disk_error.is_none()
    }
}

#[derive(Serialize)]
struct AttemptReport {
    generation: usize,
    state: &'static str,
    actual_ot_us: Option<u32>,
    actual_to_us: Option<u32>,
    connection_timeout: ConnectionTimeout,
    accepted: u64,
    sequence_gaps: u64,
    timeouts: u64,
    receive_timeout: Option<ReceiveTimeout>,
    forward_close: Option<CloseOutcome>,
    identity_error: Option<String>,
    cip_error: Option<String>,
    network_error: Option<String>,
    cyclic_io: Option<CyclicIo>,
    startup_timing: Option<StartupTiming>,
    receive_spacing: Spacing,
    send_spacing: Spacing,
    send_timing: SendTiming,
}

impl AttemptReport {
    fn from_diagnostics(generation: usize, settings: &Settings, diagnostics: &Diagnostics) -> Self {
        let status = &diagnostics.status;
        let (actual_ot_us, actual_to_us) = status.actual.unzip();
        Self {
            generation,
            state: status.state,
            actual_ot_us,
            actual_to_us,
            connection_timeout: settings.connection_timeout(actual_to_us),
            accepted: status.valid,
            sequence_gaps: status.gaps,
            timeouts: status.timeouts,
            receive_timeout: status.receive_timeout,
            forward_close: status.forward_close.clone(),
            identity_error: status.identity_error.clone(),
            cip_error: status.cip_error.clone(),
            network_error: status.network_error.clone(),
            cyclic_io: status.cyclic_io,
            startup_timing: status.startup_timing(diagnostics.send_timing),
            receive_spacing: status.spacing,
            send_spacing: diagnostics.send_spacing,
            send_timing: diagnostics.send_timing,
        }
    }
}

fn load_input_profile(profile: &Path, command: &str) -> Result<(Settings, Preferences), String> {
    let (settings, preferences) = settings_file::load_profile(profile)?;
    let preferences = preferences.unwrap_or_default();
    settings.validate()?;
    if !settings.input_only_without_writes() {
        return Err(format!(
            "{command} requires an input-only or listen-only heartbeat with zero configuration bytes and O->T Run/Idle disabled"
        ));
    }
    Ok((settings, preferences))
}

fn load_capture_profile(profile: &Path) -> Result<(Settings, Preferences), String> {
    let (settings, preferences) = load_input_profile(profile, "Capture")?;
    settings.csv_fields()?;
    Ok((settings, preferences))
}

fn capture(
    settings: &Settings,
    preferences: &Preferences,
    directory: &Path,
    seconds: u64,
    expected: ExpectedIdentity,
) -> Result<Report, String> {
    let integration = Integration::start(&preferences.integration, settings)?;
    fs::create_dir(directory).map_err(|e| {
        format!(
            "Cannot create new capture directory {}: {e}",
            directory.display()
        )
    })?;
    let mut report_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("capture.toml"))
        .map_err(|e| e.to_string())?;
    let slot = Slot::default();
    let options = Options {
        duration_seconds: seconds,
        ..preferences.capture_options(PathBuf::from(directory))
    };
    let mut capture = Capture::start(&slot, options, settings).map_err(|e| e.to_string())?;
    let sink = capture.sink.clone();
    let auto_reconnect = preferences.auto_reconnect && settings.supports_auto_reconnect();
    let mut duration_reached = false;
    let attempts = run_input(
        settings,
        InputRunOptions {
            expected: expected.into(),
            schedule: preferences.schedule,
            auto_reconnect,
        },
        &slot,
        integration.as_ref().map(Integration::publisher),
        || {
            if interrupt::requested() || capture.finished() {
                return true;
            }
            duration_reached |= sink.deadline().is_some_and(|end| Instant::now() >= end);
            duration_reached
        },
    );
    capture.stop(&slot);
    let attempts = attempts?;
    let last = attempts.last();
    let canceled = Status {
        state: "Canceled",
        ..Status::default()
    };
    let status = last.map_or(&canceled, |run| &run.status);
    let last_input = attempts
        .iter()
        .rev()
        .find(|run| run.status.valid != 0)
        .map(|run| &run.status);
    let (actual_ot_us, actual_to_us) = status.actual.unzip();
    let send_timing = last.map_or_else(SendTiming::default, |run| run.send_timing);
    let sum = |field: fn(&Diagnostics) -> u64| attempts.iter().map(field).sum();
    let report = Report {
        integration: integration.as_ref().map(Integration::counters),
        state: status.state,
        adapter_ip: settings.device_ip,
        local_ip: settings.local_ip,
        io_route: settings.io_route.clone(),
        input_assembly: settings.input_instance,
        input_size: settings.input_size,
        input_multicast: settings.input_multicast,
        default_unicast_port: settings.default_unicast_port,
        listen_only: settings.listen_only,
        input_min_size: settings.input_min_size,
        duration_seconds: seconds,
        duration_reached,
        interrupted: interrupt::requested(),
        schedule: preferences.schedule,
        requested_ot_us: settings.output_rpi_us,
        requested_to_us: settings.input_rpi_us,
        actual_ot_us,
        actual_to_us,
        connection_timeout: settings.connection_timeout(actual_to_us),
        accepted: sum(|run| run.status.valid),
        sequence_gaps: sum(|run| run.status.gaps),
        duplicates: sum(|run| run.status.duplicates),
        late: sum(|run| run.status.late),
        invalid: sum(|run| run.status.invalid),
        timeouts: sum(|run| run.status.timeouts),
        receive_timeout: status.receive_timeout,
        forward_close: status.forward_close.clone(),
        sent: sum(|run| run.sent),
        skipped_deadlines: sum(|run| run.skipped),
        seen: sink.seen.load(Relaxed),
        after_capture_deadline: sink.after_capture_deadline.load(Relaxed),
        unchanged: sink.unchanged.load(Relaxed),
        written: sink.written.load(Relaxed),
        buffer_overruns: sink.overrun.load(Relaxed),
        last_value: last_input
            .and_then(|run| run.value.as_ref())
            .map(ToString::to_string),
        last_input_hex: last_input.map(|run| assembly_scanner::model::hex(&run.input)),
        csv_prefix: sink.prefix.display().to_string(),
        capture_start_utc: sink.start_utc().to_string(),
        csv_parts: sink.part_count(),
        field_summaries: sink.field_summaries(),
        identity_error: status.identity_error.clone(),
        cip_error: status.cip_error.clone(),
        network_error: status.network_error.clone(),
        disk_error: sink.error.lock().unwrap().clone(),
        identity: status.identity.clone(),
        cyclic_io: status.cyclic_io,
        startup_timing: status.startup_timing(send_timing),
        receive_spacing: status.spacing,
        send_spacing: last.map_or_else(Spacing::default, |run| run.send_spacing),
        send_timing,
        attempts: if auto_reconnect {
            attempts
                .iter()
                .enumerate()
                .map(|(index, run)| AttemptReport::from_diagnostics(index + 1, settings, run))
                .collect()
        } else {
            Vec::new()
        },
    };
    let text = report_text(&report)?;
    report_file
        .write_all(text.as_bytes())
        .and_then(|_| report_file.flush())
        .map_err(|e| e.to_string())?;
    Ok(report)
}
