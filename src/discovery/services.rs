//! Sessionless EtherNet/IP ListServices over unicast UDP or TCP.

use super::{DiscoveryTransport as ServicesTransport, query_tcp, unregistered_request};
use crate::{
    encapsulation::PORT,
    explicit::{EipError, is_unicast},
    model::hex,
};
use ethernetip_core::encap::{HEADER_LEN, Header};
use serde::Serialize;
use std::{
    io,
    net::{Ipv4Addr, UdpSocket},
    time::{Duration, Instant},
};

const LIST_SERVICES: u16 = 0x0004;
const COMMUNICATIONS_ITEM: u16 = 0x0100;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
pub struct ServicesReport {
    pub adapter_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub transport: ServicesTransport,
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

/// Ask one IPv4 endpoint which EtherNet/IP encapsulation services it advertises over UDP.
pub fn list_services(adapter: Ipv4Addr, local: Ipv4Addr) -> Result<ServicesReport, EipError> {
    list_services_with_transport(adapter, local, ServicesTransport::Udp)
}

/// Query the addressed endpoint before registering an encapsulation session.
pub fn list_services_with_transport(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    transport: ServicesTransport,
) -> Result<ServicesReport, EipError> {
    if !is_unicast(adapter) || !is_unicast(local) {
        return Err(EipError::Protocol(
            "Enter unicast adapter and local NIC IPv4 addresses".into(),
        ));
    }
    let request = unregistered_request(LIST_SERVICES)?;
    let services = match transport {
        ServicesTransport::Udp => query_udp(adapter, local, &request)?,
        ServicesTransport::Tcp => {
            let (header, body) = query_tcp(adapter, local, &request)?;
            decode_reply(header, &body, &request[12..20])?
        }
    };
    Ok(ServicesReport {
        adapter_ip: adapter,
        local_ip: local,
        transport,
        services,
    })
}

fn query_udp(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    request: &[u8; HEADER_LEN],
) -> Result<Vec<ServiceItem>, EipError> {
    let socket = UdpSocket::bind((local, 0))?;
    socket.connect((adapter, PORT))?;
    let context = &request[12..20];
    socket.send(request)?;

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
        return decode_reply(header, &packet[HEADER_LEN..], context);
    }
}

fn decode_reply(
    header: Header,
    payload: &[u8],
    context: &[u8],
) -> Result<Vec<ServiceItem>, EipError> {
    if header.command != LIST_SERVICES
        || header.session_handle != 0
        || header.options != 0
        || payload.len() != usize::from(header.length)
    {
        return Err(EipError::Protocol(
            "Invalid ListServices encapsulation reply".into(),
        ));
    }
    if header.sender_context.as_slice() != context {
        return Err(EipError::Protocol(
            "ListServices reply sender context does not match".into(),
        ));
    }
    if header.status != 0 {
        return Err(EipError::Encap(header.status));
    }
    parse_items(payload).ok_or_else(|| EipError::Protocol("Invalid ListServices item list".into()))
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
