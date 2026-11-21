//! Read-only diagnostics from one CIP Ethernet Link Object instance.

use super::{Route, SavedReadBatch, batch::read_object_attributes};
use std::net::Ipv4Addr;

pub const ETHERNET_LINK_CLASS: u32 = 0xf6;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Interface speed"),
    (2, "Interface flags"),
    (3, "MAC address"),
    (4, "Interface counters"),
    (5, "Media counters"),
];
const INTERFACE_COUNTERS: &[&str] = &[
    "Input octets",
    "Input unicast packets",
    "Input non-unicast packets",
    "Input discards",
    "Input errors",
    "Input unknown protocols",
    "Output octets",
    "Output unicast packets",
    "Output non-unicast packets",
    "Output discards",
    "Output errors",
];
const MEDIA_COUNTERS: &[&str] = &[
    "Alignment errors",
    "FCS errors",
    "Single collisions",
    "Multiple collisions",
    "SQE test errors",
    "Deferred transmissions",
    "Late collisions",
    "Excessive collisions",
    "MAC transmit errors",
    "Carrier sense errors",
    "Frames too long",
    "MAC receive errors",
];

pub struct LinkInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl LinkInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }
}

/// Read the port's reported diagnostics and cumulative counters.
pub fn read_link_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<LinkInfo, String> {
    if instance == 0 {
        return Err("Ethernet Link inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        ETHERNET_LINK_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(LinkInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        1 => bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map(|rate| format!("{rate} Mb/s"))
            .map_err(|_| "Interface speed requires a four-byte UDINT".into()),
        2 => flags(bytes),
        3 => bytes
            .try_into()
            .map(format_mac)
            .map_err(|_| "MAC address requires six bytes".into()),
        4 => counters(bytes, INTERFACE_COUNTERS),
        5 => counters(bytes, MEDIA_COUNTERS),
        _ => return None,
    })
}

pub(super) fn format_mac([a, b, c, d, e, f]: &[u8; 6]) -> String {
    format!("{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{f:02X}")
}

fn flags(bytes: &[u8]) -> Result<String, String> {
    let flags = u32::from_le_bytes(
        bytes
            .try_into()
            .map_err(|_| "Interface flags require a four-byte DWORD")?,
    );
    let link = match flags & 3 {
        1 => "Link up, half duplex",
        3 => "Link up, full duplex",
        _ => "Link down",
    };
    let negotiation = (flags >> 2) & 7;
    let mut description = format!("0x{flags:08X} [{link}, negotiation status {negotiation}");
    if flags & (1 << 5) != 0 {
        description.push_str(", link changes require reset");
    }
    if flags & (1 << 6) != 0 {
        description.push_str(", local hardware fault");
    }
    description.push(']');
    Ok(description)
}

fn counters(bytes: &[u8], names: &[&str]) -> Result<String, String> {
    let expected = names.len() * 4;
    if bytes.len() != expected {
        return Err(format!(
            "Counter block requires {expected} bytes. Reply has {} bytes.",
            bytes.len()
        ));
    }
    Ok(names
        .iter()
        .zip(bytes.as_chunks::<4>().0)
        .map(|(name, raw)| format!("{name} {}", u32::from_le_bytes(*raw)))
        .collect::<Vec<_>>()
        .join(", "))
}
