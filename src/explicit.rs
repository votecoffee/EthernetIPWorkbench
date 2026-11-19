//! Direct EtherNet/IP explicit messaging and read-only CIP requests.
use crate::{
    model::Settings,
    scanner::{le16, le32, protocol},
};
pub use ethernetip_core::error::EipError;
use ethernetip_core::{
    cip::ReplyHeader,
    cpf::{self, Envelope, Item, item_type},
    encap::{self, Command, HEADER_LEN, Header},
    error::Result,
};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};
const TO_SOCKET: u16 = 0x8001;
pub(crate) const PORT: u16 = 44818;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2);

/// A direct logical CIP address. Instance zero reads a class attribute.
/// Omitting the attribute selects Get_Attributes_All for the instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadPath {
    pub class: u32,
    pub instance: u32,
    pub attribute: Option<u32>,
}

impl ReadPath {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        if self.class == 0 || self.attribute == Some(0) {
            return Err("CIP class and attribute IDs must be nonzero");
        }
        Ok(())
    }

    /// Decode common standard attributes when the reply length matches.
    /// Use the raw payload for vendor-specific or object-wide replies.
    pub fn describe(self, bytes: &[u8]) -> Option<String> {
        match (self.class, self.instance, self.attribute, bytes) {
            (1, 1, Some(1..=3), [lo, hi]) => Some(u16::from_le_bytes([*lo, *hi]).to_string()),
            (1, 1, Some(4), [major, minor]) => Some(format!("{major}.{minor}")),
            (1, 1, Some(5), [lo, hi]) => Some(format!("0x{:04X}", u16::from_le_bytes([*lo, *hi]))),
            (1, 1, Some(6), &[a, b, c, d]) => {
                Some(format!("0x{:08X}", u32::from_le_bytes([a, b, c, d])))
            }
            (1, 1, Some(7), [length, name @ ..]) if usize::from(*length) == name.len() => {
                Some(String::from_utf8_lossy(name).into_owned())
            }
            (0xf5, 1, Some(1), &[a, b, c, d]) => {
                Some(format!("0x{:08X}", u32::from_le_bytes([a, b, c, d])))
            }
            (0xf6, 1, Some(1), &[a, b, c, d]) => {
                Some(format!("{} Mb/s", u32::from_le_bytes([a, b, c, d])))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedRead {
    pub name: String,
    pub path: ReadPath,
}

pub fn validate_saved_reads(reads: &[SavedRead]) -> std::result::Result<(), String> {
    if reads.len() > 64 {
        return Err("A profile can store up to 64 CIP reads".into());
    }
    let mut names = std::collections::HashSet::new();
    for read in reads {
        read.path.validate().map_err(str::to_owned)?;
        if read.name.trim().is_empty() || read.name.len() > 128 || read.name != read.name.trim() {
            return Err(
                "Each saved read needs a name of 1–128 bytes with no surrounding spaces".into(),
            );
        }
        if !names.insert(read.name.as_str()) {
            return Err(format!("Duplicate saved read name {:?}", read.name));
        }
    }
    Ok(())
}

/// Registered explicit TCP session. Only direct CIP read operations are exposed.
pub struct Reader(Option<Session>);

impl Reader {
    pub fn connect(device: Ipv4Addr, local: Ipv4Addr) -> Result<Self> {
        if !is_unicast(device) || !is_unicast(local) {
            return Err(protocol(
                "Enter unicast adapter and local NIC IPv4 addresses",
            ));
        }
        Ok(Self(Some(Session::open(device, local)?)))
    }

    /// Return the CIP application payload. CIP rejections preserve the status words.
    /// A transport or protocol error ends this Reader's session.
    pub fn read(&mut self, path: ReadPath) -> Result<Vec<u8>> {
        path.validate().map_err(protocol)?;
        let result = self.0.as_mut().ok_or(EipError::Closed)?.read(path);
        if matches!(&result, Err(error) if !matches!(error, EipError::Cip { .. })) {
            self.0 = None;
        }
        result
    }
}

pub(crate) fn is_unicast(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified() && !ip.is_broadcast() && !ip.is_multicast()
}

pub fn parse_id(text: &str) -> std::result::Result<u32, String> {
    let text = text.trim();
    let value = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .map_or_else(|| text.parse(), |hex| u32::from_str_radix(hex, 16));
    value.map_err(|_| format!("Invalid CIP identifier {text:?}. Use decimal or 0x hex."))
}

fn logical_segment(path: &mut Vec<u8>, code: u8, value: u32) {
    if let Ok(value) = u8::try_from(value) {
        path.extend([code, value]);
    } else if let Ok(value) = u16::try_from(value) {
        path.extend([code | 1, 0]);
        path.extend(value.to_le_bytes());
    } else {
        path.extend([code | 2, 0]);
        path.extend(value.to_le_bytes());
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Identity {
    pub vendor: u16,
    pub device_type: u16,
    pub code: u16,
    pub major: u8,
    pub minor: u8,
    pub status: u16,
    pub serial: u32,
    pub name: String,
}
pub type IdentityKey = (u16, u16, u16);

impl Identity {
    pub fn key(&self) -> IdentityKey {
        (self.vendor, self.device_type, self.code)
    }

    pub fn pin(&self) -> IdentityPin {
        IdentityPin {
            key: self.key(),
            serial: self.serial,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityPin {
    pub key: IdentityKey,
    pub serial: u32,
}

pub(crate) struct Session {
    tcp: TcpStream,
    handle: u32,
    context: u64,
}
impl Session {
    pub(crate) fn open(device_ip: Ipv4Addr, local_ip: Ipv4Addr) -> Result<Self> {
        let tcp = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        tcp.bind(&SocketAddr::from((local_ip, 0)).into())?;
        tcp.connect_timeout(
            &SocketAddr::from((device_ip, PORT)).into(),
            EXCHANGE_TIMEOUT,
        )?;
        let tcp: TcpStream = tcp.into();
        tcp.set_write_timeout(Some(EXCHANGE_TIMEOUT))?;
        tcp.set_nodelay(true)?;
        let mut session = Self {
            tcp,
            handle: 0,
            context: 0,
        };
        let (header, body) = session.exchange(Command::RegisterSession, &[1, 0, 0, 0])?;
        if header.session_handle == 0 || body != [1, 0, 0, 0] {
            return Err(protocol("Invalid RegisterSession reply"));
        }
        session.handle = header.session_handle;
        Ok(session)
    }
    fn exchange(&mut self, cmd: Command, payload: &[u8]) -> Result<(Header, Vec<u8>)> {
        self.context += 1;
        let context = self.context.to_le_bytes();
        self.tcp
            .write_all(&encap::encode_frame(cmd, self.handle, context, payload))?;
        let deadline = Instant::now() + EXCHANGE_TIMEOUT;
        let mut raw = [0; HEADER_LEN];
        self.read_before(&mut raw, deadline)?;
        let header = Header::parse(&raw)?;
        let mut body = vec![0; header.length as usize];
        self.read_before(&mut body, deadline)?;
        if header.command != cmd.as_u16()
            || header.sender_context != context
            || (self.handle != 0 && header.session_handle != self.handle)
            || header.options != 0
        {
            return Err(protocol(format!(
                "Invalid encapsulation reply to {cmd:?}, status 0x{:08X}",
                header.status
            )));
        }
        if header.status != 0 {
            return Err(EipError::Encap(header.status));
        }
        Ok((header, body))
    }

    fn read_before(&mut self, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
        use std::io::{Error, ErrorKind};
        while !bytes.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(ErrorKind::TimedOut, "EtherNet/IP reply deadline").into());
            }
            self.tcp
                .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
            match self.tcp.read(bytes) {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::UnexpectedEof,
                        "EtherNet/IP reply ended early",
                    )
                    .into());
                }
                Ok(n) => bytes = &mut bytes[n..],
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    pub(crate) fn cip(
        &mut self,
        service: u8,
        class: u8,
        body: &[u8],
        endpoint: Option<SocketAddr>,
    ) -> Result<(Vec<u8>, Envelope)> {
        self.cip_path(service, &[0x20, class, 0x24, 1], body, endpoint)
    }

    fn read(&mut self, address: ReadPath) -> Result<Vec<u8>> {
        let mut path = Vec::with_capacity(18);
        logical_segment(&mut path, 0x20, address.class);
        logical_segment(&mut path, 0x24, address.instance);
        let service = if let Some(attribute) = address.attribute {
            logical_segment(&mut path, 0x30, attribute);
            0x0e
        } else {
            0x01
        };
        self.cip_path(service, &path, &[], None)
            .map(|(body, _)| body)
    }

    fn cip_path(
        &mut self,
        service: u8,
        path: &[u8],
        body: &[u8],
        endpoint: Option<SocketAddr>,
    ) -> Result<(Vec<u8>, Envelope)> {
        let mut request = vec![service, (path.len() / 2) as u8];
        request.extend_from_slice(path);
        request.extend_from_slice(body);
        let mut items = vec![
            Item::null_address(),
            Item::new(item_type::UNCONNECTED_DATA, request),
        ];
        if let Some(addr) = endpoint {
            items.push(Item::new(TO_SOCKET, cpf::encode_sockaddr_in_v4(addr)?));
        }
        let (_, reply) = self.exchange(Command::SendRRData, &cpf::encode_envelope(0, 5, &items))?;
        let envelope = Envelope::parse(&reply)?;
        let item = envelope
            .find(item_type::UNCONNECTED_DATA)
            .ok_or_else(|| protocol("No CIP reply item"))?;
        let header = ReplyHeader::parse(&item.data)?;
        if item.data[0] != (service | 0x80) {
            return Err(protocol("Unexpected CIP reply service"));
        }
        if header.general_status != 0 {
            return Err(EipError::Cip {
                status: header.general_status,
                ext: header.extended_status,
            });
        }
        Ok((item.data[header.body_offset..].to_vec(), envelope))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.handle != 0 {
            let _ = self.tcp.write_all(&encap::encode_frame(
                Command::UnRegisterSession,
                self.handle,
                [0; 8],
                &[],
            ));
        }
    }
}
pub fn read_identity(cfg: &Settings) -> std::result::Result<Identity, String> {
    identity(&mut Session::open(cfg.device_ip, cfg.local_ip).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
pub(crate) fn identity(session: &mut Session) -> Result<Identity> {
    match session.cip(1, 1, &[], None) {
        Ok((body, _)) => decode_identity(&body),
        Err(EipError::Cip { status: 0x08, .. }) => {
            // Some adapters implement only Get_Attribute_Single.
            let mut body = Vec::new();
            for (id, size) in [(1, 2), (2, 2), (3, 2), (4, 2), (5, 2), (6, 4), (7, 0)] {
                let bytes = session.read(ReadPath {
                    class: 1,
                    instance: 1,
                    attribute: Some(id),
                })?;
                if (id != 7 && bytes.len() != size)
                    || (id == 7
                        && bytes
                            .first()
                            .is_none_or(|n| usize::from(*n) + 1 != bytes.len()))
                {
                    return Err(protocol(format!("Invalid Identity attribute {id} size")));
                }
                body.extend(bytes);
            }
            decode_identity(&body)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn decode_identity(body: &[u8]) -> Result<Identity> {
    if body.len() < 15 || body.len() < 15 + body[14] as usize {
        return Err(protocol("Short Identity Object reply"));
    }
    Ok(Identity {
        vendor: le16(&body, 0),
        device_type: le16(&body, 2),
        code: le16(&body, 4),
        major: body[6],
        minor: body[7],
        status: le16(&body, 8),
        serial: le32(&body, 10),
        name: String::from_utf8_lossy(&body[15..15 + body[14] as usize]).into_owned(),
    })
}
