use super::{Identity, Value};
use std::{
    fmt,
    net::SocketAddr,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Spacing {
    pub intervals: u64,
    pub span_us: u64,
    pub observed_packets_per_second: Option<f64>,
    pub percent_of_negotiated_rate: Option<f64>,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub intervals_at_or_over_2x_rpi: u64,
    pub intervals_at_or_over_4x_rpi: u64,
}
pub(super) struct Histogram {
    bins: Vec<u64>,
    rpi_us: u64,
    count: u64,
    span: Duration,
    max: u64,
    at_or_over_2x: u64,
    at_or_over_4x: u64,
}
impl Histogram {
    // Exact microsecond buckets through 32,767 µs, then 256 buckets per doubling.
    const EXACT_BITS: u32 = 15;
    const PRECISION_BITS: u32 = 8;
    const EXACT_BINS: usize = 1 << Self::EXACT_BITS;
    const BINS_PER_OCTAVE: usize = 1 << Self::PRECISION_BITS;
    const TOTAL_BINS: usize =
        Self::EXACT_BINS + (u64::BITS - Self::EXACT_BITS) as usize * Self::BINS_PER_OCTAVE;

    pub(super) fn new(actual_rpi_us: u32) -> Self {
        Self {
            bins: vec![0; Self::TOTAL_BINS],
            rpi_us: u64::from(actual_rpi_us),
            count: 0,
            span: Duration::ZERO,
            max: 0,
            at_or_over_2x: 0,
            at_or_over_4x: 0,
        }
    }
    fn bucket(us: u64) -> usize {
        if us < Self::EXACT_BINS as u64 {
            return us as usize;
        }
        let octave = us.ilog2();
        let shift = octave - Self::PRECISION_BITS;
        let offset = (us >> shift) as usize - Self::BINS_PER_OCTAVE;
        Self::EXACT_BINS + (octave - Self::EXACT_BITS) as usize * Self::BINS_PER_OCTAVE + offset
    }

    fn bucket_upper(index: usize) -> u64 {
        if index < Self::EXACT_BINS {
            return index as u64;
        }
        let coarse = index - Self::EXACT_BINS;
        let octave = Self::EXACT_BITS + (coarse / Self::BINS_PER_OCTAVE) as u32;
        let shift = octave - Self::PRECISION_BITS;
        let mantissa = (Self::BINS_PER_OCTAVE + coarse % Self::BINS_PER_OCTAVE) as u64;
        (mantissa << shift) | ((1u64 << shift) - 1)
    }

    pub(super) fn add(&mut self, gap: Duration) {
        let us = gap.as_micros().min(u64::MAX.into()) as u64;
        self.bins[Self::bucket(us)] += 1;
        self.count += 1;
        self.span += gap;
        self.max = self.max.max(us);
        self.at_or_over_2x += u64::from(us >= self.rpi_us * 2);
        self.at_or_over_4x += u64::from(us >= self.rpi_us * 4);
    }
    pub(super) fn snapshot(&self) -> Spacing {
        let mut values = [0; 3];
        let targets = [50, 95, 99].map(|pct| (self.count * pct).div_ceil(100));
        let mut cumulative = 0;
        let mut i = 0;
        for (index, n) in self.bins.iter().enumerate() {
            cumulative += n;
            while i < 3 && cumulative >= targets[i] {
                values[i] = Self::bucket_upper(index).min(self.max);
                i += 1;
            }
            if i == 3 {
                break;
            }
        }
        let observed_packets_per_second =
            (!self.span.is_zero()).then(|| self.count as f64 / self.span.as_secs_f64());
        let negotiated_packets_per_second = 1_000_000.0 / self.rpi_us as f64;
        Spacing {
            intervals: self.count,
            span_us: self.span.as_micros().min(u64::MAX.into()) as u64,
            observed_packets_per_second,
            percent_of_negotiated_rate: observed_packets_per_second
                .map(|rate| 100.0 * rate / negotiated_packets_per_second),
            p50_us: values[0],
            p95_us: values[1],
            p99_us: values[2],
            max_us: self.max,
            intervals_at_or_over_2x_rpi: self.at_or_over_2x,
            intervals_at_or_over_4x_rpi: self.at_or_over_4x,
        }
    }
}

/// Terminal receive timing. The silence includes the interval after the last valid packet.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ReceiveTimeout {
    pub waiting_for_first_input: bool,
    pub limit_us: u64,
    pub valid_input_silence_us: u64,
    pub last_udp_age_us: Option<u64>,
    pub last_sequence: Option<u32>,
    /// Successful sends observed by the receiver since its last accepted input.
    pub ot_sends_during_silence: u64,
    /// Gaps between the timestamps used for the receive watchdog checks.
    pub last_check_gap_us: u64,
    pub longest_check_gap_us: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CloseOutcome {
    Confirmed,
    Rejected {
        general_status: u8,
        extended_status: Vec<u16>,
    },
    Unconfirmed {
        error: String,
    },
}

impl CloseOutcome {
    pub(crate) fn from_result(result: ethernetip_core::error::Result<()>) -> Self {
        match result {
            Ok(()) => Self::Confirmed,
            Err(ethernetip_core::error::EipError::Cip { status, ext }) => Self::Rejected {
                general_status: status,
                extended_status: ext,
            },
            Err(error) => Self::Unconfirmed {
                error: error.to_string(),
            },
        }
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

impl fmt::Display for CloseOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirmed => f.write_str("confirmed"),
            Self::Rejected {
                general_status,
                extended_status,
            } => {
                write!(f, "rejected (CIP general 0x{general_status:02X}")?;
                for word in extended_status {
                    write!(f, ", extended 0x{word:04X}")?;
                }
                f.write_str(")")
            }
            Self::Unconfirmed { error } => write!(f, "unconfirmed ({error})"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub state: &'static str,
    pub identity: Option<Identity>,
    pub identity_error: Option<String>,
    pub network_error: Option<String>,
    pub cip_error: Option<String>,
    pub actual: Option<(u32, u32)>,
    pub cyclic_io: Option<CyclicIo>,
    pub forward_open_exchange_us: Option<u64>,
    pub first_udp_after_open_us: Option<u64>,
    pub first_input_after_open_us: Option<u64>,
    pub last_input: Option<Instant>,
    pub input: Vec<u8>,
    pub value: Option<Value>,
    pub input_run: Option<bool>,
    pub valid: u64,
    pub gaps: u64,
    pub duplicates: u64,
    pub late: u64,
    pub invalid: u64,
    pub timeouts: u64,
    pub receive_timeout: Option<ReceiveTimeout>,
    pub forward_close: Option<CloseOutcome>,
    pub retryable: bool,
    pub spacing: Spacing,
}

/// Latest published receive and producer measurements for a scanner session.
#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub requested_ot_us: u32,
    pub requested_to_us: u32,
    pub status: Status,
    pub sent: u64,
    pub skipped: u64,
    pub send_spacing: Spacing,
    pub send_timing: SendTiming,
}

/// Socket endpoints and connection IDs used by an accepted Forward_Open.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct CyclicIo {
    pub local_udp: SocketAddr,
    pub output_destination: SocketAddr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_multicast_destination: Option<std::net::SocketAddrV4>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_local_udp: Option<SocketAddr>,
    pub first_input_source: Option<SocketAddr>,
    pub ot_connection_id: u32,
    pub to_connection_id: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    Deadline,
    DeadlineAboveNormal,
    Tokio,
}

/// Wall-clock timing measured on the cyclic producer thread.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct SendTiming {
    pub above_normal_applied: Option<bool>,
    pub max_dispatch_lateness_us: Option<u64>,
    pub max_send_call_us: u64,
    pub send_calls_at_or_over_rpi: u64,
    #[serde(skip)]
    pub first_send_after_open_us: Option<u64>,
}

/// Monotonic host timings for the explicit exchange and the start of cyclic I/O.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct StartupTiming {
    pub forward_open_exchange_us: u64,
    pub first_send_after_open_us: Option<u64>,
    pub first_udp_after_open_us: Option<u64>,
    pub first_input_after_open_us: Option<u64>,
}

impl Status {
    pub fn startup_timing(&self, send_timing: SendTiming) -> Option<StartupTiming> {
        Some(StartupTiming {
            forward_open_exchange_us: self.forward_open_exchange_us?,
            first_send_after_open_us: send_timing.first_send_after_open_us,
            first_udp_after_open_us: self.first_udp_after_open_us,
            first_input_after_open_us: self.first_input_after_open_us,
        })
    }
}
