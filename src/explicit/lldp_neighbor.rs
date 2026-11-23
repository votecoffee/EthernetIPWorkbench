//! Read-only inspection of one CIP LLDP Data Table entry.

use super::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes};
use crate::scanner::{le16, le32};
use serde::Serialize;
use std::net::Ipv4Addr;

pub const LLDP_DATA_TABLE_CLASS: u32 = 0x010a;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Local Ethernet Link instance"),
    (2, "Neighbor MAC address"),
    (3, "Neighbor interface label"),
    (4, "Advertised time to live"),
    (5, "System capabilities"),
    (6, "Neighbor management IPv4 addresses"),
    (7, "Advertised CIP identification"),
    (8, "Ethernet preemption capabilities"),
    (9, "Last change (device counter)"),
];

pub struct LldpNeighborReport {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

/// Values recorded by the queried adapter. See the batch for each raw outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LldpNeighbor {
    pub local_link_instance: Option<u16>,
    pub mac_address: Option<String>,
    pub interface_label: Option<String>,
    pub ttl_seconds: Option<u16>,
    pub system_capabilities: Option<LldpCapabilities>,
    pub management_ipv4: Option<Vec<Ipv4Addr>>,
    pub cip_identity: Option<LldpCipIdentity>,
    pub ethernet_preemption: Option<LldpPreemption>,
    pub last_change_raw: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LldpCapabilities {
    pub supported_mask: u16,
    pub enabled_mask: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LldpCipIdentity {
    pub vendor_id: u16,
    pub device_type: u16,
    pub product_code: u16,
    pub major: u8,
    pub minor: u8,
    pub serial: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LldpPreemption {
    pub supported: bool,
    pub enabled: bool,
    pub active: bool,
    pub additional_fragment_size_code: u8,
    pub additional_fragment_size_bytes: Option<u16>,
}

impl LldpNeighborReport {
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }

    pub fn neighbor(&self) -> LldpNeighbor {
        let reply = |id| match &self
            .batch
            .results
            .iter()
            .find(|row| row.request.path.attribute == Some(id))?
            .outcome
        {
            SavedReadOutcome::Reply(bytes) => Some(bytes.as_slice()),
            _ => None,
        };
        LldpNeighbor {
            local_link_instance: reply(1).and_then(|bytes| uint(bytes).ok()),
            mac_address: reply(2).and_then(|bytes| mac(bytes).ok().flatten()),
            interface_label: reply(3).and_then(|bytes| label(bytes).ok()),
            ttl_seconds: reply(4).and_then(|bytes| uint(bytes).ok()),
            system_capabilities: reply(5).and_then(|bytes| capabilities(bytes).ok()),
            management_ipv4: reply(6).and_then(|bytes| management_addresses(bytes).ok()),
            cip_identity: reply(7).and_then(|bytes| identity(bytes).ok().flatten()),
            ethernet_preemption: reply(8).and_then(|bytes| preemption(bytes).ok()),
            last_change_raw: reply(9).and_then(|bytes| udint(bytes).ok()),
        }
    }
}

/// Read one recorded neighbor from the target's LLDP Data Table Object.
pub fn read_lldp_neighbor(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<LldpNeighborReport, String> {
    if instance == 0 {
        return Err("LLDP neighbor inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        LLDP_DATA_TABLE_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(LldpNeighborReport {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        1 => uint(bytes).map(|instance| match instance {
            0 => "Unknown (0)".into(),
            _ => instance.to_string(),
        }),
        2 => mac(bytes).map(|mac| mac.unwrap_or_else(|| "No MAC reported".into())),
        3 => label(bytes).map(|label| {
            if label.is_empty() {
                "(empty)".into()
            } else {
                label
            }
        }),
        4 => uint(bytes).map(|ttl| format!("{ttl} s")),
        5 => capabilities(bytes).map(|caps| {
            format!(
                "Supported 0x{:04X}, enabled 0x{:04X}",
                caps.supported_mask, caps.enabled_mask
            )
        }),
        6 => management_addresses(bytes).map(|addresses| {
            if addresses.is_empty() {
                "No IPv4 management address reported".into()
            } else {
                addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }),
        7 => identity(bytes).map(|identity| match identity {
            Some(id) => format!(
                "Vendor {}, device type {}, product {}, revision {}.{}, serial 0x{:08X}",
                id.vendor_id, id.device_type, id.product_code, id.major, id.minor, id.serial
            ),
            None => "No CIP identification reported".into(),
        }),
        8 => preemption(bytes).map(|status| {
            let fragment = match status.additional_fragment_size_bytes {
                Some(size) => format!("{size} bytes"),
                None if !status.supported => "unavailable".into(),
                None => format!("reserved code {}", status.additional_fragment_size_code),
            };
            format!(
                "Supported {}, enabled {}, active {}, additional fragment size {fragment}",
                status.supported, status.enabled, status.active
            )
        }),
        // Device manuals disagree on the units. Keep the reported counter.
        9 => udint(bytes).map(|value| value.to_string()),
        _ => return None,
    })
}

fn uint(bytes: &[u8]) -> Result<u16, String> {
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| "Expected a two-byte UINT".into())
}

fn udint(bytes: &[u8]) -> Result<u32, String> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| "Expected a four-byte UDINT".into())
}

fn mac(bytes: &[u8]) -> Result<Option<String>, String> {
    let address: &[u8; 6] = bytes
        .try_into()
        .map_err(|_| "MAC address requires six bytes")?;
    Ok((*address != [0; 6]).then(|| super::link_info::format_mac(address)))
}

fn label(bytes: &[u8]) -> Result<String, String> {
    match bytes {
        [length, text @ ..] if *length <= 64 && usize::from(*length) == text.len() => {
            Ok(String::from_utf8_lossy(text).into_owned())
        }
        _ => Err("Interface label requires a SHORT_STRING with at most 64 bytes".into()),
    }
}

fn capabilities(bytes: &[u8]) -> Result<LldpCapabilities, String> {
    if bytes.len() != 4 {
        return Err("System capabilities require two WORD masks".into());
    }
    Ok(LldpCapabilities {
        supported_mask: le16(bytes, 0),
        enabled_mask: le16(bytes, 2),
    })
}

fn management_addresses(bytes: &[u8]) -> Result<Vec<Ipv4Addr>, String> {
    match bytes {
        [count, octets @ ..] if usize::from(*count) * 4 == octets.len() => Ok(octets
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(Ipv4Addr::from)
            .collect()),
        _ => Err("IPv4 address count does not match the four-byte address entries".into()),
    }
}

fn identity(bytes: &[u8]) -> Result<Option<LldpCipIdentity>, String> {
    if bytes.len() != 12 {
        return Err("CIP identification requires 12 bytes".into());
    }
    Ok((bytes != [0; 12]).then(|| LldpCipIdentity {
        vendor_id: le16(bytes, 0),
        device_type: le16(bytes, 2),
        product_code: le16(bytes, 4),
        major: bytes[6],
        minor: bytes[7],
        serial: le32(bytes, 8),
    }))
}

fn preemption(bytes: &[u8]) -> Result<LldpPreemption, String> {
    let &[supported @ 0..=1, enabled @ 0..=1, active @ 0..=1, code] = bytes else {
        return Err(
            "Ethernet preemption requires three BOOL bytes and a fragment size code".into(),
        );
    };
    Ok(LldpPreemption {
        supported: supported == 1,
        enabled: enabled == 1,
        active: active == 1,
        additional_fragment_size_code: code,
        additional_fragment_size_bytes: (supported == 1 && code <= 3)
            .then(|| (u16::from(code) + 1) * 64),
    })
}
