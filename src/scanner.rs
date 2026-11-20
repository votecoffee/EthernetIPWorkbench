use crate::{
    capture::Slot,
    explicit::{Route, Session, identity},
    integration::Publisher,
    model::{self, Settings, Value},
};
use ethernetip_connections::forward_open::{
    ForwardCloseRequest, ForwardCloseResponse, ForwardOpenRequest, ForwardOpenResponse,
    NetworkConnectionParameters,
};
use ethernetip_core::{cpf, error::EipError};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    ops::RangeInclusive,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OT_SOCKET: u16 = 0x8000;
type Result<T> = std::result::Result<T, EipError>;
pub(crate) fn protocol(message: impl Into<String>) -> EipError {
    EipError::Protocol(message.into())
}

pub use crate::explicit::{Identity, IdentityKey, IdentityPin};

#[derive(Clone, Copy, Debug)]
pub enum ExpectedIdentity {
    Model(IdentityKey),
    Device(IdentityPin),
}

impl ExpectedIdentity {
    fn mismatch(&self, actual: &Identity) -> Option<String> {
        match *self {
            Self::Model((vendor, device_type, product))
                if (vendor, device_type, product) != actual.key() =>
            {
                Some(format!(
                    "Identity mismatch. Expected vendor:type:product {vendor}:{device_type}:{product}, got {}:{}:{}",
                    actual.vendor, actual.device_type, actual.code
                ))
            }
            Self::Device(pin) if pin != actual.pin() => Some(format!(
                "Identity mismatch. Expected {:?}, serial {:08X}; got {:?}, serial {:08X}",
                pin.key,
                pin.serial,
                actual.key(),
                actual.serial
            )),
            _ => None,
        }
    }
}

const RETRY_SECONDS: [u64; 3] = [1, 2, 4];

/// A retry budget for one explicitly started input-only session.
pub struct Reconnect {
    pin: IdentityPin,
    tried: usize,
    due: Option<Instant>,
}

impl Reconnect {
    pub fn new(pin: IdentityPin) -> Self {
        Self {
            pin,
            tried: 0,
            due: None,
        }
    }

    pub fn schedule(&mut self, now: Instant) -> bool {
        self.due = RETRY_SECONDS
            .get(self.tried)
            .map(|seconds| now + Duration::from_secs(*seconds));
        self.due.is_some()
    }

    pub fn pending(&self) -> Option<(usize, Instant)> {
        self.due.map(|due| (self.tried + 1, due))
    }

    pub fn take_ready(&mut self, now: Instant) -> Option<IdentityPin> {
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.tried += 1;
            Some(self.pin)
        } else {
            None
        }
    }
}

pub use crate::explicit::read_identity;
mod cyclic;
mod diagnostics;
mod supervision;
mod udp;
use cyclic::{produce, receive};
#[cfg(test)]
use diagnostics::Histogram;
pub use diagnostics::*;
pub use supervision::{InputRunOptions, run_input};

struct InputTimes {
    origin: Instant,
    first_us: AtomicU64,
    latest_us: AtomicU64,
}

impl InputTimes {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            first_us: AtomicU64::new(0),
            latest_us: AtomicU64::new(0),
        }
    }

    fn record(&self, stamp: Instant, first: bool) {
        // An offset of zero in either atomic means no input has arrived yet.
        let offset = (stamp - self.origin).as_micros() as u64 + 1;
        if first {
            self.first_us.store(offset, Relaxed);
        }
        self.latest_us.store(offset, Relaxed);
    }

    fn timestamp(&self, offset: &AtomicU64) -> Option<Instant> {
        let micros = offset.load(Relaxed).checked_sub(1)?;
        Some(self.origin + Duration::from_micros(micros))
    }

    fn age(&self) -> Option<Duration> {
        self.timestamp(&self.latest_us).map(|stamp| stamp.elapsed())
    }
}

/// Read-only references to the measurements already published by a connection.
pub(crate) struct DiagnosticsSource {
    requested: (u32, u32),
    status: Arc<Mutex<Status>>,
    tx: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    tx_spacing: Arc<Mutex<Spacing>>,
    send_timing: Arc<Mutex<SendTiming>>,
    input_times: Arc<InputTimes>,
}

impl DiagnosticsSource {
    pub(crate) fn diagnostics(&self) -> Diagnostics {
        snapshot(
            self.requested,
            &self.status,
            &self.tx,
            &self.skipped,
            &self.tx_spacing,
            &self.send_timing,
        )
    }

    pub(crate) fn input_age(&self) -> Option<Duration> {
        self.input_times.age()
    }
}

fn snapshot(
    requested: (u32, u32),
    status: &Mutex<Status>,
    tx: &AtomicU64,
    skipped: &AtomicU64,
    tx_spacing: &Mutex<Spacing>,
    send_timing: &Mutex<SendTiming>,
) -> Diagnostics {
    let status = status.lock().unwrap().clone();
    let send_spacing = *tx_spacing.lock().unwrap();
    let send_timing = *send_timing.lock().unwrap();
    Diagnostics {
        requested_ot_us: requested.0,
        requested_to_us: requested.1,
        status,
        sent: tx.load(Relaxed),
        skipped: skipped.load(Relaxed),
        send_spacing,
        send_timing,
    }
}

pub struct Connection {
    pub status: Arc<Mutex<Status>>,
    pub output: Arc<Mutex<Vec<u8>>>,
    pub tx: Arc<AtomicU64>,
    pub skipped: Arc<AtomicU64>,
    pub tx_spacing: Arc<Mutex<Spacing>>,
    pub send_timing: Arc<Mutex<SendTiming>>,
    input_times: Arc<InputTimes>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    requested: (u32, u32),
    writable_lengths: Option<RangeInclusive<usize>>,
}
impl Connection {
    /// Start an identity-checked input-only or listen-only session without a recorder or publisher.
    pub fn start_input(
        settings: Settings,
        expected: ExpectedIdentity,
    ) -> std::result::Result<Self, String> {
        Self::start_with_publisher(
            settings,
            Slot::default(),
            1,
            Schedule::Deadline,
            false,
            Some(expected),
            None,
        )
    }

    /// Copy the latest diagnostics for inspection.
    pub fn diagnostics(&self) -> Diagnostics {
        snapshot(
            self.requested,
            &self.status,
            &self.tx,
            &self.skipped,
            &self.tx_spacing,
            &self.send_timing,
        )
    }

    /// Copy the current output buffer for display or inspection.
    pub fn output_bytes(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    /// Replace the payload within the configured length bounds on an authorized owner session.
    pub fn apply_output(&self, bytes: Vec<u8>) -> std::result::Result<(), String> {
        let lengths = self
            .writable_lengths
            .as_ref()
            .ok_or("Output requires an authorized owner session")?;
        if !lengths.contains(&bytes.len()) {
            return Err(format!(
                "Applied output length must be within {}–{} bytes",
                lengths.start(),
                lengths.end()
            ));
        }
        if self.stop.load(Relaxed) || self.status.lock().unwrap().state != "Receiving" {
            return Err("Output session is not receiving".into());
        }
        *self.output.lock().unwrap() = bytes;
        Ok(())
    }

    pub fn start(
        settings: Settings,
        slot: Slot,
        generation: u64,
        schedule: Schedule,
        permit_writes: bool,
    ) -> std::result::Result<Self, String> {
        Self::start_with_publisher(
            settings,
            slot,
            generation,
            schedule,
            permit_writes,
            None,
            None,
        )
    }

    pub fn start_verified_input(
        settings: Settings,
        slot: Slot,
        expected: IdentityKey,
    ) -> std::result::Result<Self, String> {
        Self::start_verified_input_with_schedule(settings, slot, expected, Schedule::Deadline)
    }

    pub fn start_verified_input_with_schedule(
        settings: Settings,
        slot: Slot,
        expected: IdentityKey,
        schedule: Schedule,
    ) -> std::result::Result<Self, String> {
        Self::start_with_publisher(
            settings,
            slot,
            1,
            schedule,
            false,
            Some(ExpectedIdentity::Model(expected)),
            None,
        )
    }

    pub fn start_pinned_input(
        settings: Settings,
        slot: Slot,
        generation: u64,
        schedule: Schedule,
        expected: IdentityPin,
    ) -> std::result::Result<Self, String> {
        Self::start_pinned(settings, slot, generation, schedule, false, expected)
    }

    pub fn start_pinned(
        settings: Settings,
        slot: Slot,
        generation: u64,
        schedule: Schedule,
        permit_writes: bool,
        expected: IdentityPin,
    ) -> std::result::Result<Self, String> {
        Self::start_with_publisher(
            settings,
            slot,
            generation,
            schedule,
            permit_writes,
            Some(ExpectedIdentity::Device(expected)),
            None,
        )
    }

    pub fn start_with_publisher(
        settings: Settings,
        slot: Slot,
        generation: u64,
        schedule: Schedule,
        permit_writes: bool,
        expected: Option<ExpectedIdentity>,
        publisher: Option<Publisher>,
    ) -> std::result::Result<Self, String> {
        settings.validate()?;
        if !permit_writes && !settings.input_only_without_writes() {
            return Err(
                "Confirm bench ownership, configuration and output permission first".into(),
            );
        }
        let mut shared = Shared {
            status: Arc::new(Mutex::new(Status {
                state: "Opening",
                ..Status::default()
            })),
            output: Arc::new(Mutex::new(settings.output()?)),
            stop: Arc::new(AtomicBool::new(false)),
            tx: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
            tx_spacing: Arc::new(Mutex::new(Spacing::default())),
            send_timing: Arc::new(Mutex::new(SendTiming::default())),
            input_times: Arc::new(InputTimes::new()),
            publisher,
        };
        let requested = (settings.output_rpi_us, settings.input_rpi_us);
        let writable_lengths = if permit_writes && !settings.input_only {
            Some(settings.output_lengths()?)
        } else {
            None
        };
        let connection = shared.clone();
        let worker = thread::Builder::new()
            .name("class1-session".into())
            .spawn(move || {
                if let Some(publisher) = &shared.publisher {
                    publisher.track_connection(generation, || DiagnosticsSource {
                        requested,
                        status: shared.status.clone(),
                        tx: shared.tx.clone(),
                        skipped: shared.skipped.clone(),
                        tx_spacing: shared.tx_spacing.clone(),
                        send_timing: shared.send_timing.clone(),
                        input_times: shared.input_times.clone(),
                    });
                }
                let result = run(
                    &settings,
                    &slot,
                    generation,
                    schedule,
                    expected,
                    &mut shared,
                );
                let mut status = shared.status.lock().unwrap();
                if let Err(error) = result {
                    status.state = "Error";
                    status.retryable = matches!(error, EipError::Io(_));
                    match error {
                        EipError::Cip { status: code, ext } => {
                            let context = if settings.listen_only && code == 1 && ext.first() == Some(&0x0119) {
                                " · The adapter has no controlling connection for this input assembly"
                            } else {
                                ""
                            };
                            status.cip_error = Some(format!(
                                "CIP general 0x{code:02X}, extended [{}]{context}",
                                ext.iter()
                                    .map(|x| format!("0x{x:04X}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ))
                        }
                        other => status.network_error = Some(other.to_string()),
                    }
                } else if status.state != "Timed out" {
                    status.state = match &status.forward_close {
                        Some(close) if !close.is_confirmed() => "Close unconfirmed",
                        _ => "Closed",
                    };
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            status: connection.status,
            output: connection.output,
            stop: connection.stop,
            tx: connection.tx,
            skipped: connection.skipped,
            tx_spacing: connection.tx_spacing,
            send_timing: connection.send_timing,
            input_times: connection.input_times,
            worker: Some(worker),
            requested,
            writable_lengths,
        })
    }
    pub fn first_input_at(&self) -> Option<Instant> {
        self.input_times.timestamp(&self.input_times.first_us)
    }
    pub fn input_age(&self) -> Option<Duration> {
        self.input_times.age()
    }
    pub fn stop(&self) {
        self.stop.store(true, Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Wait for the worker to finish and return its final diagnostics.
    pub fn finish(mut self) -> Diagnostics {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.diagnostics()
    }

    /// Request stop, wait for Forward_Close, then return final diagnostics.
    pub fn shutdown(self) -> Diagnostics {
        self.stop();
        self.finish()
    }

    pub fn join(self) {
        self.finish();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Clone)]
struct Shared {
    status: Arc<Mutex<Status>>,
    output: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    tx: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    tx_spacing: Arc<Mutex<Spacing>>,
    send_timing: Arc<Mutex<SendTiming>>,
    input_times: Arc<InputTimes>,
    publisher: Option<Publisher>,
}

pub(crate) fn forward_close(session: &mut Session, request: &ForwardCloseRequest) -> Result<()> {
    let (body, _) = session.cip(0x4e, 6, &request.encode(), None)?;
    let reply = ForwardCloseResponse::decode(&body)?;
    if reply.connection_serial != request.connection_serial
        || reply.originator_vendor != request.originator_vendor
        || reply.originator_serial != request.originator_serial
    {
        return Err(protocol("Mismatched Forward_Close connection identifiers"));
    }
    Ok(())
}

pub(crate) fn le16(raw: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(raw[at..at + 2].try_into().unwrap())
}
pub(crate) fn le32(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().unwrap())
}

fn segment(path: &mut Vec<u8>, code: u8, value: u16) {
    if let Ok(v) = u8::try_from(value) {
        path.extend([code, v]);
    } else {
        path.extend([code | 1, 0]);
        path.extend(value.to_le_bytes());
    }
}
fn path(cfg: &Settings, route: Option<&Route>) -> Vec<u8> {
    let mut path = route.map_or_else(Vec::new, Route::port_segments);
    if let Some(key) = cfg.electronic_key {
        path.extend([0x34, 4]);
        path.extend(key.vendor.to_le_bytes());
        path.extend(key.device_type.to_le_bytes());
        path.extend(key.product.to_le_bytes());
        path.extend([key.wire_major(), key.minor]);
    }
    path.extend([0x20, 4]);
    if let Some(instance) = cfg.config_instance {
        segment(&mut path, 0x24, instance);
    }
    segment(&mut path, 0x2c, cfg.output_instance);
    segment(&mut path, 0x2c, cfg.input_instance);
    path
}
fn params(
    size: u16,
    header: bool,
    priority: u8,
    variable_size: bool,
    multicast: bool,
) -> NetworkConnectionParameters {
    NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: if multicast { 1 } else { 2 },
        priority,
        variable_size,
        size: size + 2 + 4 * u16::from(header),
    }
}
pub(crate) fn encode_large_open(open: &ForwardOpenRequest) -> Vec<u8> {
    // The two network parameter words are the only fields that change width.
    // Preserve the full sizes when widening; standard encoding masks to 9 bits.
    fn large(params: NetworkConnectionParameters) -> [u8; 4] {
        ((u32::from(params.encode() & 0xfe00) << 16) | u32::from(params.size)).to_le_bytes()
    }
    let standard = open.encode();
    let mut body = Vec::with_capacity(standard.len() + 4);
    body.extend(&standard[..26]);
    body.extend(large(open.o_to_t_params));
    body.extend(&standard[28..32]);
    body.extend(large(open.t_to_o_params));
    body.extend(&standard[34..]);
    body
}

fn run(
    cfg: &Settings,
    slot: &Slot,
    generation: u64,
    schedule: Schedule,
    expected: Option<ExpectedIdentity>,
    shared: &mut Shared,
) -> Result<()> {
    let route = cfg.parsed_io_route().map_err(protocol)?;
    let mut session = Session::open(cfg.device_ip, cfg.local_ip)?;
    {
        let id = identity(&mut session, route.as_ref());
        let mut state = shared.status.lock().unwrap();
        match id {
            Ok(id) => {
                let mismatch = expected.and_then(|expected| expected.mismatch(&id));
                state.identity = Some(id);
                if let Some(message) = mismatch {
                    state.identity_error = Some(message.clone());
                    return Err(protocol(message));
                }
            }
            Err(error) => {
                state.identity_error = Some(error.to_string());
                if expected.is_some() || route.is_some() {
                    return Err(error);
                }
                if !matches!(error, EipError::Cip { .. }) {
                    drop(state);
                    session = Session::open(cfg.device_ip, cfg.local_ip)?;
                }
            }
        }
    }
    let udp = udp::bind(
        SocketAddrV4::new(cfg.local_ip, cfg.local_udp_port),
        cfg.input_multicast,
    )?;

    let mut ids = [0; 10];
    getrandom::fill(&mut ids).map_err(|e| protocol(e.to_string()))?;
    let close_path = path(cfg, route.as_ref());
    let mut open_path = close_path.clone();
    let config = cfg.configuration().map_err(protocol)?;
    if !config.is_empty() {
        open_path.extend([0x80, config.len().div_ceil(2) as u8]);
        open_path.extend(&config);
        if config.len() % 2 != 0 {
            open_path.push(0);
        }
    }
    let open = ForwardOpenRequest {
        priority_tick: 0x0a,
        timeout_ticks: 0x0a,
        o_to_t_connection_id: 0,
        t_to_o_connection_id: le32(&ids, 0).max(1),
        connection_serial: le16(&ids, 4).max(1),
        originator_vendor: 0,
        originator_serial: le32(&ids, 6).max(1),
        connection_timeout_mult: cfg.timeout_multiplier_code,
        o_to_t_rpi_us: cfg.output_rpi_us,
        o_to_t_params: params(
            cfg.output_size,
            cfg.output_run_idle,
            cfg.output_priority,
            cfg.output_min_size.is_some(),
            false,
        ),
        t_to_o_rpi_us: cfg.input_rpi_us,
        t_to_o_params: params(
            cfg.input_size,
            cfg.input_run_idle,
            cfg.input_priority,
            cfg.input_min_size.is_some(),
            cfg.input_multicast,
        ),
        transport_type: 1,
        connection_path: open_path,
    };
    if shared.stop.load(Relaxed) {
        return Ok(());
    }
    let local_addr = udp.local_addr()?;
    let requested_input = if cfg.input_multicast {
        // Ask the target to select the group. Some adapters require this
        // placeholder item and always return their own group and port.
        SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            if cfg.local_udp_port == 0 {
                udp::IO_PORT
            } else {
                cfg.local_udp_port
            },
        ))
    } else {
        local_addr
    };
    let (service, request) = if cfg.large_forward_open {
        (0x5b, encode_large_open(&open))
    } else {
        (0x54, open.encode())
    };
    let open_started = Instant::now();
    let open_result = session.cip(service, 6, &request, Some(requested_input));
    let open_completed = Instant::now();
    shared.status.lock().unwrap().forward_open_exchange_us =
        Some((open_completed - open_started).as_micros() as u64);
    let (body, envelope) = open_result?;
    let close = ForwardCloseRequest {
        priority_tick: open.priority_tick,
        timeout_ticks: open.timeout_ticks,
        connection_serial: open.connection_serial,
        originator_vendor: open.originator_vendor,
        originator_serial: open.originator_serial,
        connection_path: close_path,
    };
    let io_result = (|| {
        let reply = ForwardOpenResponse::decode(&body)?;
        if reply.connection_serial != open.connection_serial
            || reply.originator_vendor != open.originator_vendor
            || reply.originator_serial != open.originator_serial
            || reply.o_to_t_connection_id == 0
            || reply.t_to_o_connection_id == 0
            || reply.o_to_t_actual_rpi_us == 0
            || reply.t_to_o_actual_rpi_us == 0
        {
            return Err(protocol(
                "Forward_Open returned invalid connection identifiers or intervals",
            ));
        }
        shared.status.lock().unwrap().actual =
            Some((reply.o_to_t_actual_rpi_us, reply.t_to_o_actual_rpi_us));
        let peer = cpf::resolve_peer_udp(&envelope, OT_SOCKET, cfg.device_ip.into(), udp::IO_PORT);
        if peer.ip() != cfg.device_ip {
            return Err(protocol(format!(
                "Advertised O->T address {} differs from the configured cyclic peer {}",
                peer.ip(),
                cfg.device_ip
            )));
        }
        let destination = cfg
            .input_multicast
            .then(|| udp::multicast_destination(&envelope))
            .transpose()?;
        let input_udp = destination
            .map(|group| udp::join(&udp, cfg.local_ip, group))
            .transpose()?
            .flatten();
        let receiver = input_udp.as_ref().unwrap_or(&udp);
        {
            let mut status = shared.status.lock().unwrap();
            status.cyclic_io = Some(CyclicIo {
                local_udp: local_addr,
                output_destination: peer,
                input_multicast_destination: destination,
                input_local_udp: destination.map(|_| receiver.local_addr()).transpose()?,
                first_input_source: None,
                ot_connection_id: reply.o_to_t_connection_id,
                to_connection_id: reply.t_to_o_connection_id,
            });
            status.state = "Waiting for input";
        }
        if let Some(publisher) = &shared.publisher {
            publisher.set_active_generation(generation);
        }
        thread::scope(|scope| {
            let sender = scope.spawn(|| {
                let result = produce(&udp, peer, cfg, &reply, open_completed, shared, schedule);
                if result.is_err() {
                    shared.stop.store(true, Relaxed);
                }
                result
            });
            let result = receive(
                receiver,
                cfg,
                &reply,
                open_completed,
                slot,
                generation,
                shared,
            );
            shared.stop.store(true, Relaxed);
            let tx = sender
                .join()
                .map_err(|_| protocol("Cyclic sender panicked"))?;
            result.and(tx)
        })
    })();
    if let Some(publisher) = &shared.publisher {
        publisher.set_active_generation(0);
    }
    let close_result = forward_close(&mut session, &close);
    let mut status = shared.status.lock().unwrap();
    // A watchdog timeout remains the primary result. Preserve the existing
    // retry policy for explicit rejections and transport failures at close.
    status.retryable &= close_result
        .as_ref()
        .err()
        .is_none_or(|error| matches!(error, EipError::Io(_)));
    status.forward_close = Some(CloseOutcome::from_result(close_result));
    io_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_reports_short_intervals_exactly_and_bounds_long_percentiles() {
        let mut histogram = Histogram::new(1_000);
        for (us, count) in [(1_001, 50), (30_000, 45), (2_000_000, 4), (5_000_000, 1)] {
            for _ in 0..count {
                histogram.add(Duration::from_micros(us));
            }
        }
        let summary = histogram.snapshot();
        assert_eq!(summary.intervals, 100);
        assert_eq!(summary.p50_us, 1_001);
        assert_eq!(summary.p95_us, 30_000);
        assert!((2_000_000..=2_008_000).contains(&summary.p99_us));
        assert_eq!(summary.max_us, 5_000_000);

        let mut edge = Histogram::new(1_000);
        edge.add(Duration::from_micros(32_768));
        assert_eq!(edge.snapshot().p99_us, 32_768);
        edge.add(Duration::from_micros(u64::MAX));
        let summary = edge.snapshot();
        assert_eq!(summary.p99_us, u64::MAX);
        assert_eq!(summary.max_us, u64::MAX);
    }

    #[test]
    fn spacing_counts_long_intervals_against_the_actual_rpi_and_preserves_span() {
        let mut histogram = Histogram::new(1_250);
        assert_eq!(histogram.snapshot().span_us, 0);
        for ns in [2_499_999, 2_500_000, 4_999_999, 5_000_000] {
            histogram.add(Duration::from_nanos(ns));
        }
        let summary = histogram.snapshot();
        assert_eq!(summary.intervals, 4);
        assert_eq!(summary.span_us, 14_999);
        assert_eq!(summary.intervals_at_or_over_2x_rpi, 3);
        assert_eq!(summary.intervals_at_or_over_4x_rpi, 1);
    }

    #[test]
    fn packet_rate_measures_interval_count_over_elapsed_span() {
        let mut histogram = Histogram::new(1_000);
        assert!(histogram.snapshot().observed_packets_per_second.is_none());
        histogram.add(Duration::ZERO);
        assert!(histogram.snapshot().percent_of_negotiated_rate.is_none());
        for us in [1_000, 1_000, 6_000] {
            histogram.add(Duration::from_micros(us));
        }
        let summary = histogram.snapshot();
        assert_eq!(summary.span_us, 8_000);
        assert!((summary.observed_packets_per_second.unwrap() - 500.0).abs() < 1e-9);
        assert!((summary.percent_of_negotiated_rate.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn input_clock_preserves_the_first_packet_as_the_latest_advances() {
        let times = InputTimes::new();
        assert!(times.timestamp(&times.first_us).is_none());
        let first = times.origin + Duration::from_micros(800);
        let latest = times.origin + Duration::from_micros(1_800);
        times.record(first, true);
        times.record(latest, false);
        assert_eq!(times.timestamp(&times.first_us), Some(first));
        assert_eq!(times.timestamp(&times.latest_us), Some(latest));
    }

    #[test]
    fn reconnect_backoff_is_bounded_to_three_attempts() {
        let pin = IdentityPin {
            key: (1, 2, 3),
            serial: 4,
        };
        let mut reconnect = Reconnect::new(pin);
        let mut now = Instant::now();
        for (attempt, seconds) in [1, 2, 4].into_iter().enumerate() {
            assert!(reconnect.schedule(now));
            let due = now + Duration::from_secs(seconds);
            assert_eq!(reconnect.pending(), Some((attempt + 1, due)));
            assert_eq!(reconnect.take_ready(due - Duration::from_millis(1)), None);
            assert_eq!(reconnect.take_ready(due), Some(pin));
            assert_eq!(reconnect.take_ready(due), None);
            now = due;
        }
        assert!(!reconnect.schedule(now));
        assert_eq!(reconnect.pending(), None);
    }
}
