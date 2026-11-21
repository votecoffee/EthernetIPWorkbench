//! Read-only inspection of one CIP TCP/IP Interface Object instance.

use super::{Route, SavedReadBatch, batch::read_object_attributes, le32, route::link_object};
use std::net::Ipv4Addr;

pub const TCPIP_INTERFACE_CLASS: u32 = 0xf5;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Interface status"),
    (2, "Configuration capability"),
    (3, "Configuration control"),
    (4, "Physical Link Object path"),
    (5, "Interface configuration"),
    (6, "Host name"),
];

pub struct TcpIpInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl TcpIpInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }
}

/// Read the reported interface settings without changing the target's configuration.
pub fn read_tcpip_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<TcpIpInfo, String> {
    if instance == 0 {
        return Err("TCP/IP Interface inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        TCPIP_INTERFACE_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(TcpIpInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        1..=3 => decode_flags(attribute, bytes),
        4 => link_object(bytes),
        5 => decode_configuration(bytes),
        6 => cip_string(bytes, "Host name"),
        _ => return None,
    })
}

fn decode_flags(attribute: u32, bytes: &[u8]) -> Result<String, String> {
    let flags = u32::from_le_bytes(
        bytes
            .try_into()
            .map_err(|_| "TCP/IP Interface flags require a four-byte DWORD")?,
    );
    if attribute != 3 {
        return Ok(format!("0x{flags:08X}"));
    }
    let method = match flags & 0x0f {
        0 => "Static".into(),
        1 => "BOOTP".into(),
        2 => "DHCP".into(),
        code => format!("Startup method {code}"),
    };
    Ok(format!("0x{flags:08X} [{method}]"))
}

fn decode_configuration(bytes: &[u8]) -> Result<String, String> {
    let (addresses, domain) = bytes
        .split_first_chunk::<20>()
        .ok_or("Interface configuration requires five IPv4 fields")?;
    let ip = |offset| Ipv4Addr::from(le32(addresses, offset));
    let domain = cip_string(domain, "Domain name")?;
    Ok(format!(
        "IP {}, Mask {}, Gateway {}, Primary DNS {}, Secondary DNS {}, Domain {domain}",
        ip(0),
        ip(4),
        ip(8),
        ip(12),
        ip(16),
    ))
}

fn cip_string(bytes: &[u8], name: &str) -> Result<String, String> {
    let (length, text) = bytes
        .split_first_chunk()
        .ok_or_else(|| format!("{name} is missing its STRING length"))?;
    let length = usize::from(u16::from_le_bytes(*length));
    match text.split_at_checked(length) {
        Some((text, [])) => Ok(String::from_utf8_lossy(text).into_owned()),
        Some((text, [0])) if length % 2 == 1 => Ok(String::from_utf8_lossy(text).into_owned()),
        _ => Err(format!("{name} length does not match its STRING payload")),
    }
}
