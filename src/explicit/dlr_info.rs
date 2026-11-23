//! Read-only Device Level Ring diagnostics from a selected CIP object instance.

use super::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes};
use serde::Serialize;
use std::net::Ipv4Addr;

mod participants;
pub use participants::{DlrParticipantCountStatus, DlrParticipantReport, read_dlr_participants};

pub const DLR_CLASS: u32 = 0x47;
const BASIC_ATTRIBUTE_COUNT: usize = 4;
const SUPERVISOR_ATTRIBUTE_COUNT: usize = 10;
const PARTICIPANTS_ATTRIBUTE: (u32, &str) = (9, "Ring participant list");
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Network topology"),
    (2, "Network status"),
    (10, "Active supervisor address"),
    (12, "DLR capability flags"),
    (3, "Ring supervisor status"),
    (5, "Ring fault count"),
    (6, "Last active node on port 1"),
    (7, "Last active node on port 2"),
    (8, "Ring participant count"),
    (11, "Active supervisor precedence"),
    PARTICIPANTS_ATTRIBUTE,
];

const TOPOLOGY: &[&str] = &["Linear", "Ring"];
const NETWORK_STATUS: &[&str] = &[
    "Normal",
    "Ring fault",
    "Unexpected loop detected",
    "Partial network fault",
    "Rapid fault/restore cycle",
];
const SUPERVISOR_STATUS: &[&str] = &[
    "Backup ring supervisor",
    "Active ring supervisor",
    "Ring node",
    "Non-DLR topology",
    "Cannot support ring parameters",
];
const CAPABILITIES: &[(u32, &str)] = &[
    (0x01, "announce-based node"),
    (0x02, "beacon-based node"),
    (0x20, "supervisor capable"),
    (0x40, "redundant gateway capable"),
    (0x80, "Flush Tables frame capable"),
];

pub struct DlrReport {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DlrStatus {
    pub topology_code: Option<u8>,
    pub network_status_code: Option<u8>,
    pub active_supervisor: Option<DlrNodeAddress>,
    pub capability_flags: Option<u32>,
    pub supervisor_status_code: Option<u8>,
    pub fault_count: Option<u16>,
    pub last_active_port_1: Option<DlrNodeAddress>,
    pub last_active_port_2: Option<DlrNodeAddress>,
    pub participant_count: Option<u16>,
    pub active_supervisor_precedence: Option<u8>,
}

/// Zero IP and MAC fields are represented as absent independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DlrNodeAddress {
    pub ipv4: Option<Ipv4Addr>,
    pub mac_address: Option<String>,
}

impl DlrReport {
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }

    pub fn network_status_label(&self) -> Option<&'static str> {
        NETWORK_STATUS
            .get(usize::from(usint(self.reply(2)?).ok()?))
            .copied()
    }

    /// Return parsed fields from this inspection. See the batch for each raw outcome.
    pub fn status(&self) -> DlrStatus {
        let reply = |id| self.reply(id);
        DlrStatus {
            topology_code: reply(1).and_then(|bytes| usint(bytes).ok()),
            network_status_code: reply(2).and_then(|bytes| usint(bytes).ok()),
            active_supervisor: reply(10).and_then(|bytes| node_address(bytes).ok()),
            capability_flags: reply(12).and_then(|bytes| capability_flags(bytes).ok()),
            supervisor_status_code: reply(3).and_then(|bytes| usint(bytes).ok()),
            fault_count: reply(5).and_then(|bytes| uint(bytes).ok()),
            last_active_port_1: reply(6).and_then(|bytes| node_address(bytes).ok()),
            last_active_port_2: reply(7).and_then(|bytes| node_address(bytes).ok()),
            participant_count: reply(8).and_then(|bytes| uint(bytes).ok()),
            active_supervisor_precedence: reply(11).and_then(|bytes| usint(bytes).ok()),
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

/// Read the target's topology, network status, supervisor address and capability flags.
/// Set `supervisor_details` to include status, counters and reported fault endpoints.
pub fn read_dlr_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
    supervisor_details: bool,
) -> Result<DlrReport, String> {
    if instance == 0 {
        return Err("DLR inspection requires a nonzero instance".into());
    }
    let attributes = if supervisor_details {
        &ATTRIBUTES[..SUPERVISOR_ATTRIBUTE_COUNT]
    } else {
        &ATTRIBUTES[..BASIC_ATTRIBUTE_COUNT]
    };
    let batch = read_object_attributes(
        adapter,
        local,
        DLR_CLASS,
        instance,
        route.as_ref(),
        attributes,
    )?;
    Ok(DlrReport {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        1 => describe_code(bytes, TOPOLOGY),
        2 => describe_code(bytes, NETWORK_STATUS),
        3 => describe_code(bytes, SUPERVISOR_STATUS),
        5 | 8 => uint(bytes).map(|value| value.to_string()),
        6 | 7 | 10 => {
            node_address(bytes).map(|address| match (address.ipv4, address.mac_address) {
                (Some(ipv4), Some(mac)) => format!("{ipv4} · {mac}"),
                (Some(ipv4), None) => ipv4.to_string(),
                (None, Some(mac)) => mac,
                (None, None) => "No address reported".into(),
            })
        }
        9 => participants::describe(bytes),
        11 => usint(bytes).map(|precedence| precedence.to_string()),
        12 => capability_flags(bytes).map(|flags| {
            let names = CAPABILITIES
                .iter()
                .filter_map(|&(mask, name)| (flags & mask != 0).then_some(name))
                .collect::<Vec<_>>()
                .join(", ");
            if names.is_empty() {
                format!("0x{flags:08X}")
            } else {
                format!("0x{flags:08X} · {names}")
            }
        }),
        _ => return None,
    })
}

fn usint(bytes: &[u8]) -> Result<u8, String> {
    let &[value] = bytes else {
        return Err("Expected a one-byte USINT".into());
    };
    Ok(value)
}

fn describe_code(bytes: &[u8], names: &[&str]) -> Result<String, String> {
    let code = usint(bytes)?;
    let name = names.get(usize::from(code)).copied().unwrap_or("Unknown");
    Ok(format!("{name} ({code})"))
}

fn uint(bytes: &[u8]) -> Result<u16, String> {
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| "Expected a two-byte UINT".into())
}

fn capability_flags(bytes: &[u8]) -> Result<u32, String> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| "Capability flags require a four-byte DWORD".into())
}

fn node_address(bytes: &[u8]) -> Result<DlrNodeAddress, String> {
    bytes
        .try_into()
        .map(decode_node_address)
        .map_err(|_| "DLR node address requires a four-byte IPv4 UDINT and six-byte MAC".into())
}

fn decode_node_address(&[a, b, c, d, e, f, g, h, i, j]: &[u8; 10]) -> DlrNodeAddress {
    let address = u32::from_le_bytes([a, b, c, d]);
    let mac = [e, f, g, h, i, j];
    DlrNodeAddress {
        ipv4: (address != 0).then(|| Ipv4Addr::from(address)),
        mac_address: (mac != [0; 6]).then(|| super::link_info::format_mac(&mac)),
    }
}
