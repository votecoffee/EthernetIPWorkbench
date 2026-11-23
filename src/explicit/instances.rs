//! Bounded browsing of CIP instance IDs with Find_Next_Object_Instance.

use super::{ReadPath, Reader, Route, RoutedReadError, encode_read_path, le16};
use std::net::Ipv4Addr;

const SERVICE: u8 = 0x11;
pub const INSTANCE_PAGE_SIZE: u8 = 64;

pub struct InstancePage {
    pub adapter_or_bridge: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub class: u32,
    pub after_instance: u16,
    pub route: Option<Route>,
    pub reply: Result<Vec<u8>, RoutedReadError>,
}

pub struct InstanceIds {
    pub ids: Vec<u16>,
    pub next_after: Option<u16>,
}

impl InstancePage {
    pub fn decoded(&self) -> Result<Option<InstanceIds>, String> {
        self.reply
            .as_ref()
            .ok()
            .map(|bytes| decode(bytes, self.after_instance))
            .transpose()
    }

    pub(super) fn path(&self) -> ReadPath {
        ReadPath {
            class: self.class,
            instance: u32::from(self.after_instance),
            attribute: None,
        }
    }
}

/// Read one page of existing instance IDs strictly above the supplied cursor.
pub fn find_instances_page(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    class: u32,
    after_instance: u16,
    route: Option<Route>,
) -> Result<InstancePage, String> {
    let path = ReadPath {
        class,
        instance: u32::from(after_instance),
        attribute: None,
    };
    path.validate().map_err(str::to_owned)?;
    let mut request = encode_read_path(SERVICE, path);
    request.push(INSTANCE_PAGE_SIZE);
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let reply = reader.read_service(request, route.as_ref());
    Ok(InstancePage {
        adapter_or_bridge: adapter,
        local_ip: local,
        class,
        after_instance,
        route,
        reply,
    })
}

fn decode(bytes: &[u8], after: u16) -> Result<InstanceIds, String> {
    let (&count, data) = bytes.split_first().ok_or("Instance list has no count")?;
    if count > INSTANCE_PAGE_SIZE || data.len() != usize::from(count) * 2 {
        return Err(format!(
            "Instance list declares {count} IDs, with {} bytes after its count",
            data.len()
        ));
    }
    let mut ids: Vec<_> = data.chunks_exact(2).map(|word| le16(word, 0)).collect();
    let end = ids.is_empty() || ids.last() == Some(&0);
    if ids.last() == Some(&0) {
        ids.pop();
    }
    if ids.first().is_some_and(|&id| id <= after) || ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("Instance IDs must increase beyond the request cursor".into());
    }
    let next_after = if end { None } else { ids.last().copied() };
    Ok(InstanceIds { ids, next_after })
}
