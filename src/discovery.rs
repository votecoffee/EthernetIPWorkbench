//! IPv4 EtherNet/IP discovery on an explicitly selected interface.

use crate::{
    encapsulation::{PORT, connect_tcp, read_tcp_frame},
    explicit::{EipError, Identity, IdentityKey, decode_identity, is_unicast},
};
use ethernetip_core::encap::{HEADER_LEN, Header};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{ErrorKind, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::{Duration, Instant},
};

const LIST_IDENTITY: u16 = 0x63;
const IDENTITY_ITEM: u16 = 0x0c;
const LIMIT: usize = 256;
const TCP_TIMEOUT: Duration = Duration::from_secs(2);

mod services;
pub use DiscoveryTransport as ServicesTransport;
pub use services::{
    CommunicationsService, ServiceItem, ServicesReport, list_services, list_services_with_transport,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryTransport {
    #[default]
    Udp,
    Tcp,
}

impl DiscoveryTransport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub source: SocketAddrV4,
    pub advertised_endpoint: SocketAddrV4,
    pub protocol_version: u16,
    pub state: u8,
    pub identity: Identity,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub transport: DiscoveryTransport,
    pub devices: Vec<Device>,
    pub invalid_replies: u64,
    /// The result cap was reached. There may be additional responders.
    pub limit_reached: bool,
}

impl Report {
    fn from_inventory(
        inventory: Inventory,
        invalid_replies: u64,
        transport: DiscoveryTransport,
    ) -> Self {
        Self {
            limit_reached: inventory.len() == LIMIT,
            devices: inventory.into_values().collect(),
            invalid_replies,
            transport,
        }
    }
}

type Inventory = BTreeMap<(Ipv4Addr, IdentityKey, u32), Device>;

fn record_devices(inventory: &mut Inventory, replies: Vec<Device>) {
    for device in replies {
        let key = (
            *device.source.ip(),
            device.identity.key(),
            device.identity.serial,
        );
        inventory.insert(key, device);
        if inventory.len() == LIMIT {
            break;
        }
    }
}

/// Query device identity before session registration using the selected transport.
pub fn discover_with_transport(
    local: Ipv4Addr,
    destination: Ipv4Addr,
    transport: DiscoveryTransport,
) -> Result<Report, EipError> {
    match transport {
        DiscoveryTransport::Udp => discover(local, destination),
        DiscoveryTransport::Tcp => discover_tcp(local, destination),
    }
}

/// Send one UDP ListIdentity request and collect replies for 2.2 seconds at most.
/// The destination can be a single adapter or an IPv4 broadcast address.
pub fn discover(local: Ipv4Addr, destination: Ipv4Addr) -> Result<Report, EipError> {
    if !is_unicast(local) || destination.is_unspecified() || destination.is_multicast() {
        return Err(EipError::Protocol(
            "Enter an assigned NIC IPv4 address and a unicast or broadcast destination".into(),
        ));
    }
    let socket = UdpSocket::bind((local, 0))?;
    socket.set_broadcast(true)?;
    let request = unregistered_request(LIST_IDENTITY)?;
    let context = request[12..20].try_into().unwrap();
    socket.send_to(&request, (destination, PORT))?;

    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut packet = [0; 65_536];
    let mut devices = BTreeMap::new();
    let mut invalid_replies = 0;
    while devices.len() < LIMIT {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        socket.set_read_timeout(Some(remaining))?;
        match socket.recv_from(&mut packet) {
            Ok((n, SocketAddr::V4(source))) if is_unicast(*source.ip()) => {
                if let Some(replies) = parse_reply(&packet[..n], source, context) {
                    record_devices(&mut devices, replies);
                } else {
                    invalid_replies += 1;
                }
            }
            Ok(_) => invalid_replies += 1,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Report::from_inventory(
        devices,
        invalid_replies,
        DiscoveryTransport::Udp,
    ))
}

fn discover_tcp(local: Ipv4Addr, destination: Ipv4Addr) -> Result<Report, EipError> {
    if !is_unicast(local) || !is_unicast(destination) {
        return Err(EipError::Protocol(
            "TCP discovery requires unicast NIC and destination IPv4 addresses".into(),
        ));
    }
    let request = unregistered_request(LIST_IDENTITY)?;
    let (header, payload) = query_tcp(destination, local, &request)?;
    if header.command != LIST_IDENTITY || header.session_handle != 0 || header.options != 0 {
        return Err(EipError::Protocol(
            "Invalid ListIdentity encapsulation reply".into(),
        ));
    }
    if header.sender_context.as_slice() != &request[12..20] {
        return Err(EipError::Protocol(
            "ListIdentity reply sender context does not match".into(),
        ));
    }
    if header.status != 0 {
        return Err(EipError::Encap(header.status));
    }
    let source = SocketAddrV4::new(destination, PORT);
    let replies = parse_items(&payload, source)
        .ok_or_else(|| EipError::Protocol("Invalid ListIdentity item list".into()))?;
    let mut devices = Inventory::new();
    record_devices(&mut devices, replies);
    Ok(Report::from_inventory(devices, 0, DiscoveryTransport::Tcp))
}

fn query_tcp(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    request: &[u8; HEADER_LEN],
) -> Result<(Header, Vec<u8>), EipError> {
    let mut stream = connect_tcp(adapter, local, TCP_TIMEOUT)?;
    stream.write_all(request)?;
    read_tcp_frame(&mut stream, TCP_TIMEOUT)
}

fn unregistered_request(command: u16) -> Result<[u8; HEADER_LEN], EipError> {
    let mut request = [0; HEADER_LEN];
    request[..2].copy_from_slice(&command.to_le_bytes());
    // Keep the first context word zero so ListIdentity requests use the standard delay.
    getrandom::fill(&mut request[14..20]).map_err(|e| EipError::Protocol(e.to_string()))?;
    Ok(request)
}

fn parse_reply(raw: &[u8], source: SocketAddrV4, context: [u8; 8]) -> Option<Vec<Device>> {
    let header = Header::parse(raw).ok()?;
    if header.command != LIST_IDENTITY
        || header.session_handle != 0
        || header.status != 0
        || header.options != 0
        || header.sender_context != context
        || raw.len() != HEADER_LEN + usize::from(header.length)
    {
        return None;
    }
    parse_items(raw.get(HEADER_LEN..)?, source)
}

fn parse_items(mut payload: &[u8], source: SocketAddrV4) -> Option<Vec<Device>> {
    let count = u16::from_le_bytes(payload.first_chunk::<2>().copied()?);
    payload = &payload[2..];
    let mut devices = Vec::new();
    for _ in 0..count {
        let &[a, b, c, d] = payload.first_chunk()?;
        let length = usize::from(u16::from_le_bytes([c, d]));
        let (item, rest) = payload.get(4..)?.split_at_checked(length)?;
        payload = rest;
        if u16::from_le_bytes([a, b]) != IDENTITY_ITEM {
            continue;
        }
        // Version (2), IPv4 sockaddr (16), Identity data and one state byte.
        let fixed = item.first_chunk::<18>()?;
        if fixed[2..4] != [0, 2] {
            return None;
        }
        let (state, identity) = item[18..].split_last()?;
        if identity.len() != 15 + usize::from(*identity.get(14)?) {
            return None;
        }
        devices.push(Device {
            source,
            advertised_endpoint: SocketAddrV4::new(
                Ipv4Addr::new(fixed[6], fixed[7], fixed[8], fixed[9]),
                u16::from_be_bytes([fixed[4], fixed[5]]),
            ),
            protocol_version: u16::from_le_bytes([fixed[0], fixed[1]]),
            state: *state,
            identity: decode_identity(identity).ok()?,
        });
    }
    (payload.is_empty() && !devices.is_empty()).then_some(devices)
}
