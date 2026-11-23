//! Read the DLR supervisor's participant list and compare its two count samples.

use super::{DLR_CLASS, DlrNodeAddress, PARTICIPANTS_ATTRIBUTE, decode_node_address, uint};
use crate::explicit::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes};
use serde::Serialize;
use std::net::Ipv4Addr;

mod pages;
pub use pages::{DlrParticipantPage, read_dlr_participants_paged};

const ATTRIBUTES: &[(u32, &str)] = &[
    (8, "Ring participant count before"),
    PARTICIPANTS_ATTRIBUTE,
    (8, "Ring participant count after"),
];

pub struct DlrParticipantReport {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
    pub count_before: Option<u16>,
    pub participants: Option<Vec<DlrNodeAddress>>,
    pub count_after: Option<u16>,
    /// Page results are present when the caller selected Get_Member.
    pub pages: Option<Vec<DlrParticipantPage>>,
    pub list_issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DlrParticipantCountStatus {
    CountsMatch,
    CountsChanged,
    CountMismatch,
    Unavailable,
}

impl DlrParticipantReport {
    pub fn count_status(&self) -> DlrParticipantCountStatus {
        use DlrParticipantCountStatus::*;
        match (
            self.count_before,
            self.count_after,
            self.participants.as_ref(),
        ) {
            (Some(before), Some(after), _) if before != after => CountsChanged,
            (Some(count), Some(_), Some(nodes)) if usize::from(count) == nodes.len() => CountsMatch,
            (Some(_), Some(_), Some(_)) => CountMismatch,
            _ => Unavailable,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.count_status() == DlrParticipantCountStatus::CountsMatch && self.list_issue.is_none()
    }
}

/// Read one DLR instance with Get_Attribute_Single over an explicit session.
/// The target must support the optional participant list. Counts bracket the list
/// to detect some changes during the inspection.
pub fn read_dlr_participants(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<DlrParticipantReport, String> {
    if instance == 0 {
        return Err("DLR participant inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        DLR_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    let reply = |index: usize| match &batch.results.get(index)?.outcome {
        SavedReadOutcome::Reply(bytes) => Some(bytes.as_slice()),
        _ => None,
    };
    let count_before = reply(0).and_then(|bytes| uint(bytes).ok());
    let decoded = reply(1).and_then(|bytes| records(bytes).ok());
    let participants = decoded.as_ref().map(RingRecords::nodes);
    let list_issue = decoded
        .is_some_and(|records| records.overflow)
        .then(|| OVERFLOW_MESSAGE.into());
    let count_after = reply(2).and_then(|bytes| uint(bytes).ok());
    Ok(DlrParticipantReport {
        instance,
        route,
        batch,
        count_before,
        participants,
        count_after,
        pages: None,
        list_issue,
    })
}

pub(super) fn describe(bytes: &[u8]) -> Result<String, String> {
    let reply = records(bytes)?;
    let count = reply.addresses.len();
    if reply.overflow {
        Ok(format!("{count} participant records · {OVERFLOW_MESSAGE}"))
    } else {
        Ok(format!("{count} participant records"))
    }
}

const OVERFLOW_MESSAGE: &str = "Supervisor participant list capacity exceeded.";

struct RingRecords<'a> {
    addresses: &'a [[u8; 10]],
    overflow: bool,
}

impl RingRecords<'_> {
    fn nodes(&self) -> Vec<DlrNodeAddress> {
        self.addresses.iter().map(decode_node_address).collect()
    }
}

fn records(bytes: &[u8]) -> Result<RingRecords<'_>, String> {
    let (records, trailing) = bytes.as_chunks::<10>();
    if !trailing.is_empty() {
        return Err(format!(
            "DLR participant list has {} bytes. Each record requires 10 bytes.",
            bytes.len()
        ));
    }
    const OVERFLOW_RECORD: [u8; 10] = [0xff; 10];
    let (addresses, overflow) = match records.split_last() {
        Some((last, addresses)) if *last == OVERFLOW_RECORD => (addresses, true),
        _ => (records, false),
    };
    if addresses.contains(&OVERFLOW_RECORD) {
        return Err("DLR overflow marker appears before the last record".into());
    }
    Ok(RingRecords {
        addresses,
        overflow,
    })
}
