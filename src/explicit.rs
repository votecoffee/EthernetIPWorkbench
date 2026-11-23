//! EtherNet/IP explicit messaging for CIP reads and reviewed attribute writes.
use crate::{
    encapsulation,
    model::{Selection, Settings},
    scanner::{le16, le32, protocol},
};
pub use ethernetip_core::error::EipError;
use ethernetip_core::{
    cip::ReplyHeader,
    cpf::{self, Envelope, Item, item_type},
    encap::{self, Command, Header},
    error::Result,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};
mod assembly_info;
mod attribute_list;
mod attributes;
mod batch;
mod class_info;
mod connected;
mod connection_data;
mod dlr_info;
mod instances;
mod link_info;
mod lldp_neighbor;
mod manager_info;
mod multiple;
mod object_list;
mod port_info;
mod profile_assemblies;
mod qos_info;
mod report;
mod route;
mod tcpip_info;
mod write;
mod write_draft;
mod write_receipt;
pub use assembly_info::{ASSEMBLY_CLASS, AssemblyInfo, read_assembly_info};
pub use attribute_list::{
    AttributeListEntry, AttributeListReply, AttributeListReport, AttributeWidth,
    parse_attribute_widths, read_attribute_list,
};
pub use attributes::known_attributes;
pub use batch::{
    SavedReadBatch, SavedReadMode, SavedReadOutcome, SavedReadReport, SavedReadResult,
    run_saved_reads, run_saved_reads_with_class3_size, run_saved_reads_with_mode,
};
pub use class_info::{ClassInfo, read_class_info};
pub use connected::{
    Class3Size, ConnectedPacketBatch, ConnectedReadBatch, run_saved_reads_connected,
    run_saved_reads_connected_packet,
};
pub use connection_data::{
    ConnectionData, ConnectionDataQuery, ConnectionDataReport, ConnectionDirection,
    ConnectionTriad, read_connection_data,
};
pub use dlr_info::{DLR_CLASS, DlrNodeAddress, DlrReport, DlrStatus, read_dlr_info};
pub use instances::{INSTANCE_PAGE_SIZE, InstanceIds, InstancePage, find_instances_page};
pub use link_info::{
    ETHERNET_LINK_CLASS, LinkCapabilities, LinkControl, LinkInfo, LinkPortDetails, LinkSpeedDuplex,
    read_link_info, read_link_port_details,
};
pub use lldp_neighbor::{
    LLDP_DATA_TABLE_CLASS, LldpCapabilities, LldpCipIdentity, LldpNeighbor, LldpNeighborReport,
    LldpPreemption, read_lldp_neighbor,
};
pub use manager_info::{CONNECTION_MANAGER_CLASS, ManagerInfo, read_manager_info};
pub use multiple::run_saved_reads_packet;
pub use object_list::{ObjectList, read_object_list};
pub use port_info::{PORT_OBJECT_CLASS, PortInfo, read_port_info};
pub use profile_assemblies::{AssemblySizeCheck, ProfileAssemblyCheck, check_profile_assemblies};
pub use qos_info::{QOS_CLASS, QosInfo, read_qos_info};
pub use report::read_reply_document;
pub use route::{Route, RoutedReadError};
pub use tcpip_info::{
    AddressConflictDiagnostics, AddressConflictRecord, Ipv4Arp, MulticastConfiguration,
    TCPIP_INTERFACE_CLASS, TcpIpDiagnostics, TcpIpInfo, read_tcpip_address_conflicts,
    read_tcpip_diagnostics, read_tcpip_info,
};
pub use write::{
    AttributeWrite, CurrentValueCheck, ReadbackOutcome, WriteCurrentCheck, WriteInspection,
    WriteOptions, WriteOutcome, WriteReport,
};
pub use write_draft::WriteDraft;
pub use write_receipt::{SavedWriteOutcome, SavedWriteReceipt, SavedWriteRecord};
pub(crate) const TO_SOCKET: u16 = 0x8001;
pub(crate) const MAX_CIP_PATH_BYTES: usize = u8::MAX as usize * 2;
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

    /// Interpret a documented reply field or use the standard attribute description.
    /// A selection error leaves the successfully received bytes available to the caller.
    pub fn decode_reply(
        self,
        bytes: &[u8],
        selection: Option<Selection>,
    ) -> std::result::Result<Option<String>, String> {
        let Some(selection) = selection else {
            if let Some(attribute) = self.attribute {
                if self.instance == 0 && (1..=7).contains(&attribute) {
                    return class_info::decode(attribute, bytes).map(Some);
                }
                let decoded = match (self.class, self.instance) {
                    (object_list::MESSAGE_ROUTER_CLASS, object_list::MESSAGE_ROUTER_INSTANCE)
                        if attribute == object_list::OBJECT_LIST_ATTRIBUTE =>
                    {
                        Some(object_list::decode(bytes).map(|ids| class_info::format_ids(&ids)))
                    }
                    (PORT_OBJECT_CLASS, instance) => port_info::decode(instance, attribute, bytes),
                    (TCPIP_INTERFACE_CLASS, 1..) => tcpip_info::decode(attribute, bytes),
                    (ETHERNET_LINK_CLASS, 1..) => link_info::decode(attribute, bytes),
                    (LLDP_DATA_TABLE_CLASS, 1..) => lldp_neighbor::decode(attribute, bytes),
                    (DLR_CLASS, 1..) => dlr_info::decode(attribute, bytes),
                    (CONNECTION_MANAGER_CLASS, 1..) => manager_info::decode(attribute, bytes),
                    (QOS_CLASS, 1..) => qos_info::decode(attribute, bytes),
                    (ASSEMBLY_CLASS, 1..) => assembly_info::decode(attribute, bytes),
                    _ => None,
                };
                if let Some(decoded) = decoded {
                    return decoded.map(Some);
                }
            }
            return Ok(self.describe(bytes));
        };
        let width = selection.validated_width()?;
        selection
            .decode(bytes)
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                format!(
                    "Selection at byte {} needs {width} bytes. Reply has {} bytes.",
                    selection.offset,
                    bytes.len()
                )
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedRead {
    pub name: String,
    pub path: ReadPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
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
        if let Some(selection) = read.selection {
            selection
                .validated_width()
                .map_err(|error| format!("Saved read {:?}: {error}", read.name))?;
        }
    }
    Ok(())
}

/// Registered explicit TCP session for direct or routed CIP reads.
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

    /// Read an attribute directly or through the supplied route on this session.
    pub fn read_request(
        &mut self,
        path: ReadPath,
        route: Option<&Route>,
    ) -> std::result::Result<Vec<u8>, RoutedReadError> {
        match route {
            Some(route) => self.read_routed(path, route),
            None => self.read(path).map_err(|error| match error {
                error @ EipError::Cip { .. } => RoutedReadError::Target(error),
                error => RoutedReadError::Exchange(error),
            }),
        }
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

    /// Read through the bridge connected to this session. The route begins at
    /// that bridge and its target reply is distinct from a routing rejection.
    pub fn read_routed(
        &mut self,
        path: ReadPath,
        route: &Route,
    ) -> std::result::Result<Vec<u8>, RoutedReadError> {
        path.validate()
            .map_err(|error| RoutedReadError::Exchange(protocol(error)))?;
        let result = self
            .0
            .as_mut()
            .ok_or(RoutedReadError::Exchange(EipError::Closed))
            .and_then(|session| session.read_routed(path, route));
        if matches!(&result, Err(RoutedReadError::Exchange(_))) {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
        let tcp = encapsulation::connect_tcp(device_ip, local_ip, EXCHANGE_TIMEOUT)?;
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
        let connected = matches!(cmd, Command::SendUnitData);
        let context = if connected {
            [0; 8]
        } else {
            self.context += 1;
            self.context.to_le_bytes()
        };
        self.tcp
            .write_all(&encap::encode_frame(cmd, self.handle, context, payload))?;
        let (header, body) = encapsulation::read_tcp_frame(&mut self.tcp, EXCHANGE_TIMEOUT)?;
        if header.command != cmd.as_u16()
            || (!connected && header.sender_context != context)
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
        let request = read_message(address);
        let service = request[0];
        let reply = self.unconnected(request, None)?;
        Ok(reply_body(cip_data(&reply)?, service)?.to_vec())
    }

    fn read_routed(
        &mut self,
        path: ReadPath,
        route: &Route,
    ) -> std::result::Result<Vec<u8>, RoutedReadError> {
        let message = read_message(path);
        self.routed(&message, route, |data| {
            Ok(reply_body(data, message[0])?.to_vec())
        })
    }

    fn routed<T>(
        &mut self,
        message: &[u8],
        route: &Route,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> std::result::Result<T, RoutedReadError> {
        let reply = self
            .unconnected(route.wrap(message), None)
            .map_err(RoutedReadError::Exchange)?;
        let data = cip_data(&reply).map_err(RoutedReadError::Exchange)?;
        route::decode_routed_reply(data, decode)
    }

    fn read_service(
        &mut self,
        request: Vec<u8>,
        route: Option<&Route>,
    ) -> std::result::Result<Vec<u8>, RoutedReadError> {
        let service = request[0];
        let decode = |data: &[u8]| reply_body(data, service).map(<[u8]>::to_vec);
        if let Some(route) = route {
            self.routed(&request, route, decode)
        } else {
            self.unconnected(request, None)
                .and_then(|reply| decode(cip_data(&reply)?))
                .map_err(|error| match error {
                    error @ EipError::Cip { .. } => RoutedReadError::Target(error),
                    error => RoutedReadError::Exchange(error),
                })
        }
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
        let reply = self.unconnected(request, endpoint)?;
        let body = reply_body(cip_data(&reply)?, service)?.to_vec();
        Ok((body, reply))
    }

    fn unconnected(&mut self, request: Vec<u8>, endpoint: Option<SocketAddr>) -> Result<Envelope> {
        let mut items = vec![
            Item::null_address(),
            Item::new(item_type::UNCONNECTED_DATA, request),
        ];
        if let Some(addr) = endpoint {
            items.push(Item::new(TO_SOCKET, cpf::encode_sockaddr_in_v4(addr)?));
        }
        let (_, reply) = self.exchange(Command::SendRRData, &cpf::encode_envelope(0, 5, &items))?;
        Envelope::parse(&reply)
    }
}

fn read_message(address: ReadPath) -> Vec<u8> {
    let service = if address.attribute.is_some() {
        0x0e
    } else {
        0x01
    };
    encode_read_path(service, address)
}

fn encode_read_path(service: u8, address: ReadPath) -> Vec<u8> {
    let mut request = Vec::with_capacity(20);
    request.extend([service, 0]);
    logical_segment(&mut request, 0x20, address.class);
    logical_segment(&mut request, 0x24, address.instance);
    if let Some(attribute) = address.attribute {
        logical_segment(&mut request, 0x30, attribute);
    }
    request[1] = ((request.len() - 2) / 2) as u8;
    request
}

fn cip_data(envelope: &Envelope) -> Result<&[u8]> {
    envelope
        .find(item_type::UNCONNECTED_DATA)
        .map(|item| item.data.as_slice())
        .ok_or_else(|| protocol("No CIP reply item"))
}

fn reply_body(data: &[u8], service: u8) -> Result<&[u8]> {
    let header = ReplyHeader::parse(data)?;
    if data[0] != (service | 0x80) {
        return Err(protocol("Unexpected CIP reply service"));
    }
    if header.general_status != 0 {
        return Err(EipError::Cip {
            status: header.general_status,
            ext: header.extended_status,
        });
    }
    Ok(&data[header.body_offset..])
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
    let route = cfg.parsed_io_route()?;
    let mut session = Session::open(cfg.device_ip, cfg.local_ip).map_err(|e| e.to_string())?;
    identity(&mut session, route.as_ref()).map_err(|e| e.to_string())
}
pub(crate) fn identity(session: &mut Session, route: Option<&Route>) -> Result<Identity> {
    identity_from(|address| match route {
        None => session.read(address),
        Some(route) => session
            .read_routed(address, route)
            .map_err(|error| match error {
                RoutedReadError::Target(error) => error,
                error => protocol(error.to_string()),
            }),
    })
}

fn identity_from(mut request: impl FnMut(ReadPath) -> Result<Vec<u8>>) -> Result<Identity> {
    let mut read = |attribute| {
        request(ReadPath {
            class: 1,
            instance: 1,
            attribute,
        })
    };
    match read(None) {
        Ok(body) => decode_identity(&body),
        Err(EipError::Cip { status: 0x08, .. }) => {
            // Some adapters implement only Get_Attribute_Single.
            let mut body = Vec::new();
            for (id, size) in [(1, 2), (2, 2), (3, 2), (4, 2), (5, 2), (6, 4), (7, 0)] {
                let bytes = read(Some(id))?;
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
