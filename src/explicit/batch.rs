use super::{
    EipError, Reader, RoutedReadError, SavedRead, run_saved_reads_connected,
    run_saved_reads_connected_packet, run_saved_reads_packet, validate_saved_reads,
};
use crate::scanner::CloseOutcome;
use std::net::Ipv4Addr;

pub struct SavedReadBatch {
    pub adapter_or_bridge: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub results: Vec<SavedReadResult>,
}

pub struct SavedReadResult {
    pub request: SavedRead,
    pub outcome: SavedReadOutcome,
}

pub enum SavedReadOutcome {
    Reply(Vec<u8>),
    Rejected(RoutedReadError),
    ExchangeFailed(RoutedReadError),
    NotAttempted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SavedReadMode {
    #[default]
    Sequential,
    Packet,
    Connected,
    ConnectedPacket,
}

impl SavedReadMode {
    /// Only sequential unconnected reads can use individual CIP routes.
    pub fn requires_direct(self) -> bool {
        self != Self::Sequential
    }
}

pub struct SavedReadReport {
    pub mode: SavedReadMode,
    pub batch: SavedReadBatch,
    pub forward_close: Option<CloseOutcome>,
    pub packet_error: Option<EipError>,
}

impl SavedReadReport {
    /// True when every request got a reply and any Class 3 close was confirmed.
    pub fn is_complete(&self) -> bool {
        self.packet_error.is_none()
            && self
                .forward_close
                .as_ref()
                .is_none_or(CloseOutcome::is_confirmed)
            && self
                .batch
                .results
                .iter()
                .all(|entry| matches!(entry.outcome, SavedReadOutcome::Reply(_)))
    }
}

/// Execute a saved batch in the selected mode and retain its read and connection outcomes.
pub fn run_saved_reads_with_mode(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    mode: SavedReadMode,
) -> Result<SavedReadReport, String> {
    let (batch, forward_close, packet_error) = match mode {
        SavedReadMode::Sequential => (run_saved_reads(adapter, local, reads)?, None, None),
        SavedReadMode::Packet => (run_saved_reads_packet(adapter, local, reads)?, None, None),
        SavedReadMode::Connected => {
            let connected = run_saved_reads_connected(adapter, local, reads)?;
            (connected.batch, Some(connected.forward_close), None)
        }
        SavedReadMode::ConnectedPacket => {
            let packet = run_saved_reads_connected_packet(adapter, local, reads)?;
            (
                packet.connection.batch,
                Some(packet.connection.forward_close),
                packet.packet_error,
            )
        }
    };
    Ok(SavedReadReport {
        mode,
        batch,
        forward_close,
        packet_error,
    })
}

/// Run the saved requests in order on one registered explicit session.
/// Retain CIP rejections and stop attempting requests after an exchange failure.
pub fn run_saved_reads(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> Result<SavedReadBatch, String> {
    if reads.is_empty() {
        return Err("The profile has no saved CIP reads".into());
    }
    validate_saved_reads(&reads)?;
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let results = run_ordered(reads, |request| {
        reader.read_request(request.path, request.route.as_ref())
    });
    Ok(SavedReadBatch {
        adapter_or_bridge: adapter,
        local_ip: local,
        results,
    })
}

pub(super) fn run_ordered(
    reads: Vec<SavedRead>,
    mut query: impl FnMut(&SavedRead) -> Result<Vec<u8>, RoutedReadError>,
) -> Vec<SavedReadResult> {
    let mut session_lost = false;
    let mut results = Vec::with_capacity(reads.len());
    for request in reads {
        let outcome = if session_lost {
            SavedReadOutcome::NotAttempted
        } else {
            match query(&request) {
                Ok(bytes) => SavedReadOutcome::Reply(bytes),
                Err(error @ RoutedReadError::Exchange(_)) => {
                    session_lost = true;
                    SavedReadOutcome::ExchangeFailed(error)
                }
                Err(error) => SavedReadOutcome::Rejected(error),
            }
        };
        results.push(SavedReadResult { request, outcome });
    }
    results
}
