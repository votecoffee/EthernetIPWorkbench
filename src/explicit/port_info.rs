//! Read-only inspection of one CIP Port Object directory or port instance.

use super::{ReadPath, Route, SavedRead, SavedReadBatch, le16, run_saved_reads};
use crate::model::hex;
use std::net::Ipv4Addr;

pub const PORT_OBJECT_CLASS: u32 = 0xf4;
const DIRECTORY: &[(u32, &str)] = &[
    (2, "Maximum instance ID"),
    (3, "Instance count"),
    (8, "Entry port instance"),
    (9, "All ports"),
];
const INSTANCE: &[(u32, &str)] = &[
    (1, "Port type"),
    (2, "Port number"),
    (3, "Link Object path"),
    (4, "Port name"),
    (7, "Port and node address"),
    (10, "Routing capabilities"),
];

pub struct PortInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl PortInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }
}

/// Inspect Port Object instance zero for its directory, or one selected port.
/// Attribute replies and rejections use the same report as saved reads.
pub fn read_port_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<PortInfo, String> {
    let attributes = if instance == 0 { DIRECTORY } else { INSTANCE };
    let reads = attributes
        .iter()
        .map(|&(attribute, name)| SavedRead {
            name: name.into(),
            path: ReadPath {
                class: PORT_OBJECT_CLASS,
                instance,
                attribute: Some(attribute),
            },
            route: route.clone(),
            selection: None,
        })
        .collect();
    let batch = run_saved_reads(adapter, local, reads)?;
    Ok(PortInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(
    instance: u32,
    attribute: u32,
    bytes: &[u8],
) -> Option<Result<String, String>> {
    Some(match (instance, attribute) {
        (0, 8) | (1.., 1 | 2) => match bytes {
            &[lo, hi] => Ok(u16::from_le_bytes([lo, hi]).to_string()),
            _ => Err("Port Object attribute requires a two-byte UINT".into()),
        },
        (0, 9) => port_pairs(bytes),
        (1.., 3) => link_object(bytes),
        (1.., 4) => match bytes {
            [length, name @ ..] if usize::from(*length) == name.len() => {
                Ok(String::from_utf8_lossy(name).into_owned())
            }
            _ => Err("Port name length does not match its SHORT_STRING".into()),
        },
        (1.., 7) => Route::from_port_segment(bytes)
            .map(|segment| segment.to_string())
            .map_err(str::to_owned),
        (1.., 10) => capabilities(bytes),
        _ => return None,
    })
}

fn port_pairs(bytes: &[u8]) -> Result<String, String> {
    let pairs = bytes.chunks_exact(4);
    if !pairs.remainder().is_empty() {
        return Err("All ports requires complete four-byte type/number pairs".into());
    }
    let ports = pairs
        .map(|pair| format!("type {} / port {}", le16(pair, 0), le16(pair, 2)))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("[{ports}]"))
}

fn link_object(bytes: &[u8]) -> Result<String, String> {
    let (size, path) = bytes
        .split_first_chunk()
        .ok_or("Link Object is missing its word count")?;
    let words = usize::from(u16::from_le_bytes(*size));
    if path.len() != words * 2 {
        return Err(format!(
            "Link Object declares {words} words, with {} path bytes",
            path.len()
        ));
    }
    Ok(hex(path))
}

fn capabilities(bytes: &[u8]) -> Result<String, String> {
    let &[a, b, c, d] = bytes else {
        return Err("Routing capabilities requires a four-byte DWORD".into());
    };
    let mask = u32::from_le_bytes([a, b, c, d]);
    let labels = [
        "Incoming unconnected",
        "Outgoing unconnected",
        "Incoming Class 0/1",
        "Outgoing Class 0/1",
        "Incoming Class 2/3",
        "Outgoing Class 2/3",
        "Outgoing DeviceNet Safety",
    ];
    let enabled = labels
        .into_iter()
        .enumerate()
        .filter_map(|(bit, name)| (mask & (1 << bit) != 0).then_some(name))
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        Ok(format!("0x{mask:08X}"))
    } else {
        Ok(format!("0x{mask:08X} [{}]", enabled.join(", ")))
    }
}
