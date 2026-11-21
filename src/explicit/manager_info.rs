//! Read-only counters from one CIP Connection Manager instance.

use super::{Route, SavedReadBatch, batch::read_object_attributes};
use std::net::Ipv4Addr;

pub const CONNECTION_MANAGER_CLASS: u32 = 0x06;
const COUNTERS: &[(u32, &str)] = &[
    (1, "Open requests"),
    (2, "Open format rejects"),
    (3, "Open resource rejects"),
    (4, "Open other rejects"),
    (5, "Close requests"),
    (6, "Close format rejects"),
    (7, "Close other rejects"),
    (8, "Connection timeouts"),
];

pub struct ManagerInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl ManagerInfo {
    /// True after a usable batch. Unsupported target counters may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }
}

/// Read the Connection Manager's reported counters without clearing them.
pub fn read_manager_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<ManagerInfo, String> {
    if instance == 0 {
        return Err("Connection Manager inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        CONNECTION_MANAGER_CLASS,
        instance,
        route.as_ref(),
        COUNTERS,
    )?;
    Ok(ManagerInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    if !(1..=8).contains(&attribute) {
        return None;
    }
    Some(match bytes {
        &[lo, hi] => Ok(u16::from_le_bytes([lo, hi]).to_string()),
        _ => Err(format!(
            "Connection Manager counter requires a two-byte UINT, got {} bytes",
            bytes.len()
        )),
    })
}
