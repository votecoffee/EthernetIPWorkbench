use super::{Reader, RoutedReadError, SavedRead, validate_saved_reads};
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
