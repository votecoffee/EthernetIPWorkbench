//! Class 1 UDP endpoints and multicast membership.

use super::{Result, protocol};
use crate::explicit::TO_SOCKET;
use ethernetip_core::cpf::{Envelope, decode_sockaddr_in_v4};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

pub(super) const IO_PORT: u16 = 2222;

pub(super) fn bind(address: SocketAddrV4, exclusive: bool) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    #[cfg(windows)]
    if exclusive {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            SO_EXCLUSIVEADDRUSE, SOCKET_ERROR, SOL_SOCKET, WSAGetLastError, setsockopt,
        };
        let enabled = 1i32;
        // SAFETY: socket owns the handle, and WinSock reads one initialized i32.
        let result = unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                (&enabled as *const i32).cast(),
                size_of::<i32>() as i32,
            )
        };
        if result == SOCKET_ERROR {
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
        }
    }
    #[cfg(not(windows))]
    let _ = exclusive; // Do not opt into address reuse.
    socket.set_recv_buffer_size(512 * 1024)?;
    socket.bind(&SocketAddr::V4(address).into())?;
    let udp: UdpSocket = socket.into();
    #[cfg(not(windows))]
    udp.set_read_timeout(Some(Duration::from_millis(2)))?;
    udp.set_write_timeout(Some(Duration::from_millis(10)))?;
    Ok(udp)
}

pub(super) fn multicast_destination(reply: &Envelope) -> Result<SocketAddrV4> {
    let mut items = reply.items.iter().filter(|item| item.type_id == TO_SOCKET);
    let item = items
        .next()
        .ok_or_else(|| protocol("Forward_Open omitted the T->O multicast socket address"))?;
    if items.next().is_some() {
        return Err(protocol(
            "Forward_Open returned duplicate T->O socket addresses",
        ));
    }
    // The shared CPF decoder extracts the address and port. Check the full
    // IPv4 sockaddr shape here before joining a group on the network.
    if item.data.len() != 16 || item.data[..2] != [0, 2] {
        return Err(protocol(
            "Forward_Open returned an invalid T->O IPv4 socket address",
        ));
    }
    let destination = decode_sockaddr_in_v4(&item.data)?;
    if !destination.ip().is_multicast() || destination.port() == 0 {
        return Err(protocol(format!(
            "Forward_Open returned {destination} for multicast T->O; expected a multicast group and nonzero port"
        )));
    }
    Ok(destination)
}

/// Join on the selected interface. On Windows the receiver can share this
/// connection's source socket when the adapter returns the same port.
pub(super) fn join(
    output: &UdpSocket,
    interface: Ipv4Addr,
    destination: SocketAddrV4,
) -> io::Result<Option<UdpSocket>> {
    #[cfg(windows)]
    let input = if output.local_addr()?.port() == destination.port() {
        None
    } else {
        Some(
            bind(SocketAddrV4::new(interface, destination.port()), true).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Bind multicast input on {interface}:{}: {error}",
                        destination.port()
                    ),
                )
            })?,
        )
    };
    // POSIX stacks generally need a group or wildcard bind for multicast.
    #[cfg(not(windows))]
    let input = Some(bind(destination, true)?);
    input
        .as_ref()
        .unwrap_or(output)
        .join_multicast_v4(destination.ip(), &interface)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Join multicast {destination} on {interface}: {error}"),
            )
        })?;
    Ok(input)
}
