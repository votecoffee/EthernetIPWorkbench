//! Sessionless EtherNet/IP ListServices over unicast UDP.

use super::{
    EipError, HEADER_LEN, Header, Ipv4Addr, PORT, UdpSocket, is_unicast, unregistered_request,
};
use crate::model::hex;
use serde::Serialize;
use std::{
    io,
    time::{Duration, Instant},
};

const LIST_SERVICES: u16 = 0x0004;
const COMMUNICATIONS_ITEM: u16 = 0x0100;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
pub struct ServicesReport {
    pub adapter_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub services: Vec<ServiceItem>,
}

#[derive(Debug, Serialize)]
pub struct ServiceItem {
    pub type_id: u16,
    pub data_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communications: Option<CommunicationsService>,
}

#[derive(Debug, Serialize)]
pub struct CommunicationsService {
    pub name: String,
    pub protocol_version: u16,
    pub capability_flags: u16,
    pub cip_over_tcp: bool,
    pub cyclic_io_over_udp: bool,
}

/// Ask one IPv4 endpoint which EtherNet/IP encapsulation services it advertises.
pub fn list_services(adapter: Ipv4Addr, local: Ipv4Addr) -> Result<ServicesReport, EipError> {
    if !is_unicast(adapter) || !is_unicast(local) {
        return Err(EipError::Protocol(
            "Enter unicast adapter and local NIC IPv4 addresses".into(),
        ));
    }
    let socket = UdpSocket::bind((local, 0))?;
    socket.connect((adapter, PORT))?;
    let request = unregistered_request(LIST_SERVICES)?;
    let context = &request[12..20];
    socket.send(&request)?;

    let deadline = Instant::now() + TIMEOUT;
    let mut datagram = [0; 65_536];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                io::Error::new(io::ErrorKind::TimedOut, "ListServices reply deadline").into(),
            );
        }
        socket.set_read_timeout(Some(remaining))?;
        let count = match socket.recv(&mut datagram) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        let packet = &datagram[..count];
        let header = Header::parse(packet)?;
        if header.sender_context.as_slice() != context {
            continue;
        }
        if header.command != LIST_SERVICES
            || header.session_handle != 0
            || header.options != 0
            || packet.len() != HEADER_LEN + usize::from(header.length)
        {
            return Err(EipError::Protocol(
                "Invalid ListServices encapsulation reply".into(),
            ));
        }
        if header.status != 0 {
            return Err(EipError::Encap(header.status));
        }
        let services = parse_items(&packet[HEADER_LEN..])
            .ok_or_else(|| EipError::Protocol("Invalid ListServices item list".into()))?;
        return Ok(ServicesReport {
            adapter_ip: adapter,
            local_ip: local,
            services,
        });
    }
}

fn parse_items(payload: &[u8]) -> Option<Vec<ServiceItem>> {
    let (count, mut remaining) = payload.split_first_chunk()?;
    let mut services = Vec::new();
    for _ in 0..u16::from_le_bytes(*count) {
        let (&[a, b, c, d], items) = remaining.split_first_chunk()?;
        let type_id = u16::from_le_bytes([a, b]);
        let (data, rest) = items.split_at_checked(usize::from(u16::from_le_bytes([c, d])))?;
        remaining = rest;
        let communications = if type_id == COMMUNICATIONS_ITEM {
            let &[v0, v1, f0, f1, name @ ..] = <&[u8; 20]>::try_from(data).ok()?;
            let capability_flags = u16::from_le_bytes([f0, f1]);
            let name = name.split(|&byte| byte == 0).next()?;
            Some(CommunicationsService {
                name: String::from_utf8_lossy(name).into_owned(),
                protocol_version: u16::from_le_bytes([v0, v1]),
                capability_flags,
                cip_over_tcp: capability_flags & 0x0020 != 0,
                cyclic_io_over_udp: capability_flags & 0x0100 != 0,
            })
        } else {
            None
        };
        services.push(ServiceItem {
            type_id,
            data_hex: hex(data),
            communications,
        });
    }
    remaining.is_empty().then_some(services)
}
