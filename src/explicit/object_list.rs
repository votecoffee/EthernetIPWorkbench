//! Read the supported class IDs reported by a CIP Message Router.

use super::{
    Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes, class_info::decode_ids,
};
use std::net::Ipv4Addr;

pub(super) const MESSAGE_ROUTER_CLASS: u32 = 2;
pub(super) const MESSAGE_ROUTER_INSTANCE: u32 = 1;
pub(super) const OBJECT_LIST_ATTRIBUTE: u32 = 1;
const ATTRIBUTES: &[(u32, &str)] = &[(OBJECT_LIST_ATTRIBUTE, "Object List")];

pub struct ObjectList {
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
    /// Class IDs in device order. Absent when the read failed or could not be decoded.
    pub class_ids: Option<Vec<u16>>,
}

/// Read the Message Router's optional Object List on a direct or routed target.
pub fn read_object_list(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    route: Option<Route>,
) -> Result<ObjectList, String> {
    let batch = read_object_attributes(
        adapter,
        local,
        MESSAGE_ROUTER_CLASS,
        MESSAGE_ROUTER_INSTANCE,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    let class_ids = match batch.results.first().map(|entry| &entry.outcome) {
        Some(SavedReadOutcome::Reply(bytes)) => decode(bytes).ok(),
        _ => None,
    };
    Ok(ObjectList {
        route,
        batch,
        class_ids,
    })
}

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<u16>, String> {
    let ids = decode_ids(bytes, "Object List")?;
    if ids.contains(&0) {
        return Err("Object List contains class ID zero".into());
    }
    Ok(ids)
}
