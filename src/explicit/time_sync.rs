//! Read-only CIP Sync diagnostics from a selected Time Sync Object instance.

use super::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes};
use crate::scanner::le16;
use serde::Serialize;
use std::net::Ipv4Addr;

pub const TIME_SYNC_CLASS: u32 = 0x43;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "PTP enabled"),
    (2, "Clock synchronization"),
    (8, "Grandmaster clock"),
    (18, "PTP domain number"),
    (5, "Offset from master"),
    (6, "Maximum offset from master"),
    (7, "Mean path delay to master"),
    (12, "PTP port states"),
    (27, "Steps removed"),
];

pub struct TimeSyncReport {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimeSyncStatus {
    pub ptp_enabled: Option<bool>,
    pub synchronized: Option<bool>,
    pub domain_number: Option<u8>,
    pub offset_from_master_ns: Option<i64>,
    pub max_offset_from_master_ns: Option<u64>,
    pub mean_path_delay_to_master_ns: Option<i64>,
    pub steps_removed: Option<u16>,
    pub grandmaster: Option<GrandmasterClock>,
    pub port_states: Option<Vec<PtpPortState>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrandmasterClock {
    pub clock_identity: Option<String>,
    pub clock_class: u16,
    pub time_accuracy_code: u16,
    pub offset_scaled_log_variance: u16,
    pub current_utc_offset_raw: u16,
    pub time_property_flags: u16,
    pub time_source_code: u16,
    pub priority1: u16,
    pub priority2: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PtpPortState {
    pub port_number: u16,
    pub state_code: u16,
}

impl PtpPortState {
    fn description(self) -> &'static str {
        match self.state_code {
            1 => "Initializing",
            2 => "Faulty",
            3 => "Disabled",
            4 => "Listening",
            5 => "Pre-master",
            6 => "Master",
            7 => "Passive",
            8 => "Uncalibrated",
            9 => "Slave",
            _ => "Unknown",
        }
    }
}

impl TimeSyncReport {
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }

    pub fn synchronized(&self) -> Option<bool> {
        boolean(self.reply(2)?).ok()
    }

    /// Return decoded fields from the device. The batch holds the individual replies.
    pub fn status(&self) -> TimeSyncStatus {
        let reply = |id| self.reply(id);
        TimeSyncStatus {
            ptp_enabled: reply(1).and_then(|bytes| boolean(bytes).ok()),
            synchronized: self.synchronized(),
            domain_number: reply(18).and_then(|bytes| usint(bytes).ok()),
            offset_from_master_ns: reply(5).and_then(|bytes| lint(bytes).ok()),
            max_offset_from_master_ns: reply(6).and_then(|bytes| ulint(bytes).ok()),
            mean_path_delay_to_master_ns: reply(7).and_then(|bytes| lint(bytes).ok()),
            steps_removed: reply(27).and_then(|bytes| uint(bytes).ok()),
            grandmaster: reply(8).and_then(|bytes| grandmaster(bytes).ok()),
            port_states: reply(12).and_then(|bytes| port_states(bytes).ok()),
        }
    }

    fn reply(&self, id: u32) -> Option<&[u8]> {
        let row = self
            .batch
            .results
            .iter()
            .find(|row| row.request.path.attribute == Some(id))?;
        match &row.outcome {
            SavedReadOutcome::Reply(bytes) => Some(bytes.as_slice()),
            _ => None,
        }
    }
}

/// Inspect the device's reported PTP state without adjusting the clock.
pub fn read_time_sync(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<TimeSyncReport, String> {
    if instance == 0 {
        return Err("Time Sync inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        TIME_SYNC_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(TimeSyncReport {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        1 => boolean(bytes).map(|enabled| if enabled { "Enabled" } else { "Disabled" }.into()),
        2 => boolean(bytes).map(|synced| if synced { "Synchronized" } else { "Unsynchronized" }.into()),
        5 | 7 => lint(bytes).map(|value| format!("{value} ns")),
        6 => ulint(bytes).map(|value| format!("{value} ns")),
        8 => grandmaster(bytes).map(|clock| {
            let id = clock.clock_identity.as_deref().unwrap_or("No clock identity reported");
            format!(
                "{id} · Class {}, time accuracy 0x{:04X}, log variance 0x{:04X}, UTC offset raw {}, flags 0x{:04X}, source 0x{:04X}, priorities {} / {}",
                clock.clock_class, clock.time_accuracy_code, clock.offset_scaled_log_variance,
                clock.current_utc_offset_raw, clock.time_property_flags, clock.time_source_code,
                clock.priority1, clock.priority2
            )
        }),
        12 => port_states(bytes).map(|ports| {
            if ports.is_empty() {
                return "No PTP ports reported".into();
            }
            ports.iter().map(|port| {
                format!("Port {} · {} ({})", port.port_number, port.description(), port.state_code)
            }).collect::<Vec<_>>().join("\n")
        }),
        18 => usint(bytes).map(|domain| domain.to_string()),
        27 => uint(bytes).map(|steps| steps.to_string()),
        _ => return None,
    })
}

fn boolean(bytes: &[u8]) -> Result<bool, String> {
    match bytes {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err("Expected a one-byte BOOL of 0 or 1".into()),
    }
}

fn usint(bytes: &[u8]) -> Result<u8, String> {
    let &[value] = bytes else {
        return Err("Expected a one-byte USINT".into());
    };
    Ok(value)
}

fn uint(bytes: &[u8]) -> Result<u16, String> {
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| "Expected a two-byte UINT".into())
}

fn lint(bytes: &[u8]) -> Result<i64, String> {
    bytes
        .try_into()
        .map(i64::from_le_bytes)
        .map_err(|_| "Expected an eight-byte LINT".into())
}

fn ulint(bytes: &[u8]) -> Result<u64, String> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| "Expected an eight-byte ULINT".into())
}

fn grandmaster(bytes: &[u8]) -> Result<GrandmasterClock, String> {
    if bytes.len() != 24 {
        return Err("Grandmaster clock information requires 24 bytes".into());
    }
    let identity = &bytes[..8];
    Ok(GrandmasterClock {
        clock_identity: (identity != [0; 8]).then(|| crate::model::hex(identity).replace(' ', ":")),
        clock_class: le16(bytes, 8),
        time_accuracy_code: le16(bytes, 10),
        offset_scaled_log_variance: le16(bytes, 12),
        current_utc_offset_raw: le16(bytes, 14),
        time_property_flags: le16(bytes, 16),
        time_source_code: le16(bytes, 18),
        priority1: le16(bytes, 20),
        priority2: le16(bytes, 22),
    })
}

fn port_states(bytes: &[u8]) -> Result<Vec<PtpPortState>, String> {
    let Some((count, entries)) = bytes.split_first_chunk::<2>() else {
        return Err("PTP port state reply requires a two-byte count".into());
    };
    let count = usize::from(u16::from_le_bytes(*count));
    if entries.len() != count * 4 {
        return Err(format!(
            "PTP port state count {count} does not match {} entry bytes",
            entries.len()
        ));
    }
    Ok(entries
        .as_chunks::<4>()
        .0
        .iter()
        .map(|entry| PtpPortState {
            port_number: le16(entry, 0),
            state_code: le16(entry, 2),
        })
        .collect())
}
