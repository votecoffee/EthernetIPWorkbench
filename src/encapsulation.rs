//! Shared EtherNet/IP TCP connection and frame reads.

use ethernetip_core::{
    encap::{HEADER_LEN, Header},
    error::Result,
};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::{self, Read},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

pub(crate) const PORT: u16 = 44818;

pub(crate) fn connect_tcp(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    timeout: Duration,
) -> Result<TcpStream> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.bind(&SocketAddr::from((local, 0)).into())?;
    socket.connect_timeout(&SocketAddr::from((adapter, PORT)).into(), timeout)?;
    let stream: TcpStream = socket.into();
    stream.set_write_timeout(Some(timeout))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

pub(crate) fn read_tcp_frame(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<(Header, Vec<u8>)> {
    let deadline = Instant::now() + timeout;
    let mut raw = [0; HEADER_LEN];
    read_before(stream, &mut raw, deadline)?;
    let header = Header::parse(&raw)?;
    let mut body = vec![0; usize::from(header.length)];
    read_before(stream, &mut body, deadline)?;
    Ok((header, body))
}

fn read_before(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                io::Error::new(io::ErrorKind::TimedOut, "EtherNet/IP reply deadline").into(),
            );
        }
        stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
        match stream.read(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EtherNet/IP reply ended early",
                )
                .into());
            }
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
