use crate::{
    capture::Slot,
    explicit::{Session, identity},
    integration::Publisher,
    model::{self, Settings, Value},
};
use ethernetip_connections::forward_open::{
    ForwardCloseRequest, ForwardCloseResponse, ForwardOpenRequest, ForwardOpenResponse,
    NetworkConnectionParameters,
};
use ethernetip_core::{cpf, error::EipError};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
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
use cyclic::{produce, receive};
#[cfg(test)]
use diagnostics::Histogram;
pub use diagnostics::*;

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
    writable_size: Option<usize>,
}
impl Connection {
    /// Start an identity-checked input-only session without a recorder or publisher.
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

    /// Read the latest diagnostics without holding any internal locks.
    pub fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            requested_ot_us: self.requested.0,
            requested_to_us: self.requested.1,
            status: self.status.lock().unwrap().clone(),
            sent: self.tx.load(Relaxed),
            skipped: self.skipped.load(Relaxed),
            send_spacing: *self.tx_spacing.lock().unwrap(),
            send_timing: *self.send_timing.lock().unwrap(),
        }
    }

    /// Copy the current output buffer for display or inspection.
    pub fn output_bytes(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    /// Replace exactly the configured payload on an authorized owner session.
    pub fn apply_output(&self, bytes: Vec<u8>) -> std::result::Result<(), String> {
        if self.writable_size != Some(bytes.len()) {
            return Err("Output requires an authorized owner session and exactly the configured assembly size".into());
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
        let writable_size =
            (permit_writes && !settings.input_only).then_some(settings.output_size as usize);
        let connection = shared.clone();
        let worker = thread::Builder::new()
            .name("class1-session".into())
            .spawn(move || {
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
                            status.cip_error = Some(format!(
                                "CIP general 0x{code:02X}, extended [{}]",
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
            writable_size,
        })
    }
    pub fn first_input_at(&self) -> Option<Instant> {
        self.input_times.timestamp(&self.input_times.first_us)
    }
    pub fn input_age(&self) -> Option<Duration> {
        self.input_times
            .timestamp(&self.input_times.latest_us)
            .map(|stamp| stamp.elapsed())
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

fn forward_close(session: &mut Session, request: &ForwardCloseRequest) -> Result<()> {
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
fn path(cfg: &Settings) -> Vec<u8> {
    let mut path = Vec::new();
    if let Some(key) = cfg.electronic_key {
        path.extend([0x34, 4]);
        path.extend(key.vendor.to_le_bytes());
        path.extend(key.device_type.to_le_bytes());
        path.extend(key.product.to_le_bytes());
        path.extend([key.major, key.minor]);
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
) -> NetworkConnectionParameters {
    NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: 2,
        priority,
        variable_size,
        size: size + 2 + 4 * u16::from(header),
    }
}
fn encode_large_open(open: &ForwardOpenRequest) -> Vec<u8> {
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
    let mut session = Session::open(cfg.device_ip, cfg.local_ip)?;
    {
        let id = identity(&mut session);
        let mut state = shared.status.lock().unwrap();
        if let Some(expected) = expected {
            match &id {
                Ok(actual) => {
                    if let Some(message) = expected.mismatch(actual) {
                        state.identity = Some(actual.clone());
                        state.identity_error = Some(message.clone());
                        return Err(protocol(message));
                    }
                }
                Err(error) => {
                    state.identity_error = Some(error.to_string());
                    return id.map(|_| ());
                }
            }
        }
        match id {
            Ok(id) => state.identity = Some(id),
            Err(e @ EipError::Cip { .. }) => state.identity_error = Some(e.to_string()),
            Err(e) => {
                state.identity_error = Some(e.to_string());
                drop(state);
                session = Session::open(cfg.device_ip, cfg.local_ip)?;
            }
        }
    }
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_recv_buffer_size(512 * 1024)?;
    socket.bind(&SocketAddr::from((cfg.local_ip, cfg.local_udp_port)).into())?;
    let udp: UdpSocket = socket.into();
    #[cfg(not(windows))]
    udp.set_read_timeout(Some(Duration::from_millis(2)))?;
    udp.set_write_timeout(Some(Duration::from_millis(10)))?;

    let mut ids = [0; 10];
    getrandom::fill(&mut ids).map_err(|e| protocol(e.to_string()))?;
    let close_path = path(cfg);
    let mut open_path = close_path.clone();
    let config = model::parse_hex(&cfg.config_hex).map_err(protocol)?;
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
            false,
        ),
        t_to_o_rpi_us: cfg.input_rpi_us,
        t_to_o_params: params(
            cfg.input_size,
            cfg.input_run_idle,
            cfg.input_priority,
            cfg.input_min_size.is_some(),
        ),
        transport_type: 1,
        connection_path: open_path,
    };
    if shared.stop.load(Relaxed) {
        return Ok(());
    }
    let local_addr = udp.local_addr()?;
    let (service, request) = if cfg.large_forward_open {
        (0x5b, encode_large_open(&open))
    } else {
        (0x54, open.encode())
    };
    let open_started = Instant::now();
    let open_result = session.cip(service, 6, &request, Some(local_addr));
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
        let peer = cpf::resolve_peer_udp(&envelope, OT_SOCKET, cfg.device_ip.into(), 2222);
        if peer.ip() != cfg.device_ip {
            return Err(protocol("Adapter advertised an unexpected O->T address"));
        }
        {
            let mut status = shared.status.lock().unwrap();
            status.actual = Some((reply.o_to_t_actual_rpi_us, reply.t_to_o_actual_rpi_us));
            status.cyclic_io = Some(CyclicIo {
                local_udp: local_addr,
                output_destination: peer,
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
                let result = produce(
                    &udp,
                    peer,
                    cfg.output_run_idle,
                    &reply,
                    open_completed,
                    shared,
                    schedule,
                );
                if result.is_err() {
                    shared.stop.store(true, Relaxed);
                }
                result
            });
            let result = receive(&udp, cfg, &reply, open_completed, slot, generation, shared);
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
    status.forward_close = Some(match close_result {
        Ok(()) => CloseOutcome::Confirmed,
        Err(EipError::Cip { status, ext }) => CloseOutcome::Rejected {
            general_status: status,
            extended_status: ext,
        },
        Err(error) => CloseOutcome::Unconfirmed {
            error: error.to_string(),
        },
    });
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
