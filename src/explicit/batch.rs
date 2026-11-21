use super::{
    Class3Size, EipError, ReadPath, Reader, Route, RoutedReadError, SavedRead,
    connected::{run_connected_packet_sized, run_connected_sized},
    run_saved_reads_packet, validate_saved_reads,
};
use crate::scanner::CloseOutcome;
use std::net::Ipv4Addr;

pub(super) fn read_object_attributes(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    class: u32,
    instance: u32,
    route: Option<&Route>,
    attributes: &[(u32, &str)],
) -> Result<SavedReadBatch, String> {
    let reads = attributes
        .iter()
        .map(|&(attribute, name)| SavedRead {
            name: name.into(),
            path: ReadPath {
                class,
                instance,
                attribute: Some(attribute),
            },
            route: route.cloned(),
            selection: None,
        })
        .collect();
    run_saved_reads(adapter, local, reads)
}

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

impl SavedReadBatch {
    pub(super) fn is_usable_inspection(&self) -> bool {
        let mut decoded = false;
        for entry in &self.results {
            match &entry.outcome {
                SavedReadOutcome::Reply(bytes) => {
                    if !matches!(entry.request.path.decode_reply(bytes, None), Ok(Some(_))) {
                        return false;
                    }
                    decoded = true;
                }
                SavedReadOutcome::Rejected(RoutedReadError::Target(EipError::Cip {
                    status: 0x08 | 0x14,
                    ..
                })) => {}
                _ => return false,
            }
        }
        decoded
    }
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
    /// Every mode can now address routed reads subject to the mode's routing rules.
    #[deprecated(note = "Use validate_routes to check which reads can run together")]
    pub fn requires_direct(self) -> bool {
        false
    }

    /// Check whether the selected mode can address the saved requests together.
    pub fn validate_routes(self, reads: &[SavedRead]) -> Result<(), &'static str> {
        self.batch_route(reads).map(|_| ())
    }

    pub(super) fn batch_route(self, reads: &[SavedRead]) -> Result<Option<&Route>, &'static str> {
        let error = match self {
            Self::Sequential => return Ok(None),
            Self::Packet => "Multiple Service Packet requires one shared route or all direct reads",
            Self::Connected | Self::ConnectedPacket => {
                "Class 3 connected mode requires one shared route or all direct reads"
            }
        };
        let route = reads.first().and_then(|entry| entry.route.as_ref());
        if reads.iter().any(|entry| entry.route.as_ref() != route) {
            return Err(error);
        }
        Ok(route)
    }
}

pub struct SavedReadReport {
    pub mode: SavedReadMode,
    pub batch: SavedReadBatch,
    /// Path used by a routed Class 3 connection.
    pub connection_route: Option<Route>,
    /// Requested symmetric size when the batch used Class 3.
    pub class3_size: Option<Class3Size>,
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
    run_saved_reads_with_class3_size(adapter, local, reads, mode, Class3Size::default())
}

/// Run a saved batch with the given size for Class 3 modes.
pub fn run_saved_reads_with_class3_size(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    mode: SavedReadMode,
    size: Class3Size,
) -> Result<SavedReadReport, String> {
    let (batch, connection_route, class3_size, forward_close, packet_error) = match mode {
        SavedReadMode::Sequential => (
            run_saved_reads(adapter, local, reads)?,
            None,
            None,
            None,
            None,
        ),
        SavedReadMode::Packet => (
            run_saved_reads_packet(adapter, local, reads)?,
            None,
            None,
            None,
            None,
        ),
        SavedReadMode::Connected => {
            let connected = run_connected_sized(adapter, local, reads, size)?;
            (
                connected.batch,
                connected.route,
                Some(connected.size),
                Some(connected.forward_close),
                None,
            )
        }
        SavedReadMode::ConnectedPacket => {
            let packet = run_connected_packet_sized(adapter, local, reads, size)?;
            (
                packet.connection.batch,
                packet.connection.route,
                Some(packet.connection.size),
                Some(packet.connection.forward_close),
                packet.packet_error,
            )
        }
    };
    Ok(SavedReadReport {
        mode,
        batch,
        connection_route,
        class3_size,
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
