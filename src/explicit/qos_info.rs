//! Read-only inspection of one CIP Quality of Service Object instance.

use super::{Route, SavedReadBatch, batch::read_object_attributes};
use std::net::Ipv4Addr;

pub const QOS_CLASS: u32 = 0x48;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "802.1Q tag enable"),
    (2, "PTP event DSCP"),
    (3, "PTP general DSCP"),
    (4, "Urgent DSCP"),
    (5, "Scheduled DSCP"),
    (6, "High DSCP"),
    (7, "Low DSCP"),
    (8, "Explicit DSCP"),
];

pub struct QosInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl QosInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }
}

/// Read the QoS settings reported by the target.
pub fn read_qos_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<QosInfo, String> {
    if instance == 0 {
        return Err("QoS inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        QOS_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(QosInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    if !(1..=8).contains(&attribute) {
        return None;
    }
    let &[value] = bytes else {
        return Some(Err("QoS attribute requires a one-byte USINT".into()));
    };
    Some(match (attribute, value) {
        (1, 0) => Ok("Disabled".into()),
        (1, 1) => Ok("Enabled".into()),
        (1, _) => Err(format!("802.1Q tag enable requires 0 or 1, got {value}")),
        (_, 0..=63) => Ok(value.to_string()),
        _ => Err(format!("DSCP requires a value from 0 to 63, got {value}")),
    })
}
