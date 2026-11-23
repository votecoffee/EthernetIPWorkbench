//! Read-only inspection of one CIP TCP/IP Interface Object instance.

use super::{
    Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes, le32,
    route::link_object,
};
use serde::Serialize;
use std::net::Ipv4Addr;

mod acd;
pub use acd::{AddressConflictDiagnostics, AddressConflictRecord, Ipv4Arp};

pub const TCPIP_INTERFACE_CLASS: u32 = 0xf5;
const BASIC_ATTRIBUTE_COUNT: usize = 6;
const NETWORK_ATTRIBUTE_COUNT: usize = 9;
const STATUS: u32 = 1;
const CAPABILITY: u32 = 2;
const STATUS_ATTRIBUTE: (u32, &str) = (STATUS, "Interface status");
const CAPABILITY_ATTRIBUTE: (u32, &str) = (CAPABILITY, "Configuration capability");
const MULTICAST_TTL: u32 = 8;
const MULTICAST_CONFIGURATION: u32 = 9;
const ENCAPSULATION_INACTIVITY_TIMEOUT: u32 = 13;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    STATUS_ATTRIBUTE,
    CAPABILITY_ATTRIBUTE,
    (3, "Configuration control"),
    (4, "Physical Link Object path"),
    (5, "Interface configuration"),
    (6, "Host name"),
    (MULTICAST_TTL, "Multicast TTL"),
    (MULTICAST_CONFIGURATION, "Multicast address allocation"),
    (
        ENCAPSULATION_INACTIVITY_TIMEOUT,
        "Encapsulation inactivity timeout",
    ),
    acd::SELECT_ATTRIBUTE,
    acd::CONFLICT_ATTRIBUTE,
];

pub struct TcpIpInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TcpIpDiagnostics {
    pub multicast_pending: Option<bool>,
    pub multicast_ttl: Option<u8>,
    pub multicast_configuration: Option<MulticastConfiguration>,
    /// A reported zero disables the adapter's TCP encapsulation inactivity timer.
    pub encapsulation_inactivity_timeout_seconds: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MulticastConfiguration {
    pub allocation_control: u8,
    pub reserved: u8,
    pub address_count: u16,
    pub start_address: Ipv4Addr,
}

impl TcpIpInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }

    /// Return the decoded network settings when requested. See the batch for any missing value.
    pub fn diagnostics(&self) -> Option<TcpIpDiagnostics> {
        self.entry(MULTICAST_TTL)?;
        Some(TcpIpDiagnostics {
            multicast_pending: self.reply(STATUS).and_then(|bytes| {
                let flags = dword(bytes).ok()?;
                Some(flags & (1 << 4) != 0)
            }),
            multicast_ttl: self
                .reply(MULTICAST_TTL)
                .and_then(|bytes| multicast_ttl(bytes).ok()),
            multicast_configuration: self
                .reply(MULTICAST_CONFIGURATION)
                .and_then(|bytes| multicast_configuration(bytes).ok()),
            encapsulation_inactivity_timeout_seconds: self
                .reply(ENCAPSULATION_INACTIVITY_TIMEOUT)
                .and_then(|bytes| inactivity_timeout(bytes).ok()),
        })
    }

    fn entry(&self, attribute: u32) -> Option<&super::SavedReadResult> {
        self.batch
            .results
            .iter()
            .find(|entry| entry.request.path.attribute == Some(attribute))
    }

    fn reply(&self, attribute: u32) -> Option<&[u8]> {
        match &self.entry(attribute)?.outcome {
            SavedReadOutcome::Reply(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// Read the reported interface settings without changing the target's configuration.
pub fn read_tcpip_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<TcpIpInfo, String> {
    read_info(
        adapter,
        local,
        instance,
        route,
        &ATTRIBUTES[..BASIC_ATTRIBUTE_COUNT],
    )
}

/// Include multicast settings and encapsulation inactivity timeout in the interface inspection.
pub fn read_tcpip_diagnostics(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<TcpIpInfo, String> {
    read_info(
        adapter,
        local,
        instance,
        route,
        &ATTRIBUTES[..NETWORK_ATTRIBUTE_COUNT],
    )
}

/// Read the device's address conflict status, configuration and recorded evidence.
pub fn read_tcpip_address_conflicts(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<TcpIpInfo, String> {
    read_info(
        adapter,
        local,
        instance,
        route,
        &[
            STATUS_ATTRIBUTE,
            CAPABILITY_ATTRIBUTE,
            acd::SELECT_ATTRIBUTE,
            acd::CONFLICT_ATTRIBUTE,
        ],
    )
}

fn read_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
    attributes: &[(u32, &str)],
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
        attributes,
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
        MULTICAST_TTL => multicast_ttl(bytes).map(|ttl| format!("{ttl}")),
        MULTICAST_CONFIGURATION => multicast_configuration(bytes).map(|configuration| {
            let allocation = match configuration.allocation_control {
                0 => "Default allocation".into(),
                1 => "Configured allocation".into(),
                code => format!("Allocation control {code}"),
            };
            format!(
                "{allocation} · Address count {} · Start {}",
                configuration.address_count, configuration.start_address
            )
        }),
        ENCAPSULATION_INACTIVITY_TIMEOUT => {
            inactivity_timeout(bytes).map(|seconds| match seconds {
                0 => "0 s (disabled)".into(),
                _ => format!("{seconds} s"),
            })
        }
        _ => return acd::decode(attribute, bytes),
    })
}

fn dword(bytes: &[u8]) -> Result<u32, String> {
    Ok(u32::from_le_bytes(bytes.try_into().map_err(
        |_| "TCP/IP Interface flags require a four-byte DWORD",
    )?))
}

fn multicast_ttl(bytes: &[u8]) -> Result<u8, String> {
    let &[ttl] = bytes else {
        return Err("Multicast TTL requires one byte".into());
    };
    Ok(ttl)
}

fn multicast_configuration(bytes: &[u8]) -> Result<MulticastConfiguration, String> {
    let &[allocation_control, reserved, count_lo, count_hi, a, b, c, d] = bytes else {
        return Err("Multicast address allocation requires eight bytes".into());
    };
    Ok(MulticastConfiguration {
        allocation_control,
        reserved,
        address_count: u16::from_le_bytes([count_lo, count_hi]),
        start_address: Ipv4Addr::from(u32::from_le_bytes([a, b, c, d])),
    })
}

fn inactivity_timeout(bytes: &[u8]) -> Result<u16, String> {
    let &[lo, hi] = bytes else {
        return Err("Encapsulation inactivity timeout requires two bytes".into());
    };
    Ok(u16::from_le_bytes([lo, hi]))
}

fn decode_flags(attribute: u32, bytes: &[u8]) -> Result<String, String> {
    let flags = dword(bytes)?;
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

pub(super) fn cip_string(bytes: &[u8], name: &str) -> Result<String, String> {
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
