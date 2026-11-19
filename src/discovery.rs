//! IPv4 EtherNet/IP ListIdentity discovery on an explicitly selected interface.

use crate::explicit::{EipError, Identity, PORT, decode_identity, is_unicast};
use ethernetip_core::encap::{HEADER_LEN, Header};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::{Duration, Instant},
};

const LIST_IDENTITY: u16 = 0x63;
const IDENTITY_ITEM: u16 = 0x0c;
const LIMIT: usize = 256;

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
    pub devices: Vec<Device>,
    pub invalid_replies: u64,
    /// The result cap was reached. There may be additional responders.
    pub limit_reached: bool,
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
    let mut request = [0; HEADER_LEN];
    request[..2].copy_from_slice(&LIST_IDENTITY.to_le_bytes());
    // The first context word remains zero to request the standard broadcast delay.
    getrandom::fill(&mut request[14..20]).map_err(|e| EipError::Protocol(e.to_string()))?;
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
                    for device in replies {
                        let key = (*source.ip(), device.identity.key(), device.identity.serial);
                        devices.insert(key, device);
                        if devices.len() == LIMIT {
                            break;
                        }
                    }
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
    Ok(Report {
        limit_reached: devices.len() == LIMIT,
        devices: devices.into_values().collect(),
        invalid_replies,
    })
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
    let mut payload = raw.get(HEADER_LEN..)?;
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
