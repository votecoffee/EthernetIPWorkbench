use super::multiple::{decode_packet, request_packet};
use super::{
    Command, EipError, Envelope, Item, MAX_CIP_PATH_BYTES, Result, Route, RoutedReadError,
    SavedRead, SavedReadBatch, SavedReadMode, SavedReadResult, Session, batch::run_ordered, cpf,
    is_unicast, item_type, le16, le32, protocol, read_message, reply_body, validate_saved_reads,
};
use crate::scanner::{CloseOutcome, encode_large_open, forward_close};
use ethernetip_connections::forward_open::{
    ForwardCloseRequest, ForwardOpenRequest, ForwardOpenResponse, NetworkConnectionParameters,
};
use std::{fmt, net::Ipv4Addr, ops::RangeInclusive};

const MESSAGE_ROUTER: [u8; 4] = [0x20, 2, 0x24, 1];
const RPI_US: u32 = 2_000_000;

/// Requested symmetric Class 3 connection size, including the two-byte sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Class3Size(u16);

impl Class3Size {
    pub const RANGE: RangeInclusive<u16> = 8..=4002;

    pub fn bytes(self) -> u16 {
        self.0
    }

    pub fn uses_large_forward_open(self) -> bool {
        self.0 > 0x01ff
    }

    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn check_request(self, bytes: usize, kind: &str) -> std::result::Result<(), String> {
        let limit = usize::from(self.0) - 2;
        if bytes > limit {
            return Err(format!(
                "Class 3 {kind} needs {bytes} CIP bytes. The limit is {limit} bytes."
            ));
        }
        Ok(())
    }
}

impl Default for Class3Size {
    fn default() -> Self {
        Self(504)
    }
}

impl TryFrom<u16> for Class3Size {
    type Error = String;

    fn try_from(size: u16) -> std::result::Result<Self, Self::Error> {
        if Self::RANGE.contains(&size) {
            Ok(Self(size))
        } else {
            Err(format!(
                "Class 3 connection size must be {}–{} bytes",
                Self::RANGE.start(),
                Self::RANGE.end()
            ))
        }
    }
}

impl From<Class3Size> for u16 {
    fn from(size: Class3Size) -> Self {
        size.bytes()
    }
}

pub struct ConnectedReadBatch {
    pub batch: SavedReadBatch,
    pub route: Option<Route>,
    pub size: Class3Size,
    pub forward_close: CloseOutcome,
}

pub struct ConnectedPacketBatch {
    pub connection: ConnectedReadBatch,
    pub packet_error: Option<EipError>,
}

/// Run direct saved reads or reads with one shared route over one Class 3 TCP connection.
/// A CIP rejection is saved with its request. An exchange failure stops subsequent reads.
pub fn run_saved_reads_connected(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> std::result::Result<ConnectedReadBatch, String> {
    run_connected_sized(adapter, local, reads, Class3Size::default())
}

pub(super) fn run_connected_sized(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    size: Class3Size,
) -> std::result::Result<ConnectedReadBatch, String> {
    let route = connection_route(&reads)?;
    for request in &reads {
        size.check_request(read_message(request.path).len(), "read")?;
    }
    let (results, forward_close) =
        with_connection(adapter, local, route.as_ref(), size, |connection| {
            run_ordered(reads, |request| {
                let cip = read_message(request.path);
                connection.request(&cip).map_err(|error| match error {
                    error @ EipError::Cip { .. } => RoutedReadError::Target(error),
                    error => RoutedReadError::Exchange(error),
                })
            })
        })
        .map_err(|error| error.to_string())?;
    Ok(ConnectedReadBatch {
        batch: SavedReadBatch {
            adapter_or_bridge: adapter,
            local_ip: local,
            results,
        },
        route,
        size,
        forward_close,
    })
}

/// Run a Multiple Service Packet on one direct or routed Class 3 connection.
/// A whole-packet error leaves the individual outcomes unavailable and retains Forward_Close.
pub fn run_saved_reads_connected_packet(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> std::result::Result<ConnectedPacketBatch, String> {
    run_connected_packet_sized(adapter, local, reads, Class3Size::default())
}

pub(super) fn run_connected_packet_sized(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    size: Class3Size,
) -> std::result::Result<ConnectedPacketBatch, String> {
    let route = connection_route(&reads)?;
    let requests: Vec<_> = reads.iter().map(|entry| read_message(entry.path)).collect();
    let packet = request_packet(&requests);
    size.check_request(packet.len(), "packet")?;
    let (reply, forward_close) =
        with_connection(adapter, local, route.as_ref(), size, |connection| {
            connection.exchange(&packet, |data| decode_packet(data, &requests))
        })
        .map_err(|error| error.to_string())?;
    let (results, packet_error) = match reply {
        Ok(outcomes) => (
            reads
                .into_iter()
                .zip(outcomes)
                .map(|(request, outcome)| SavedReadResult { request, outcome })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error)),
    };
    Ok(ConnectedPacketBatch {
        connection: ConnectedReadBatch {
            batch: SavedReadBatch {
                adapter_or_bridge: adapter,
                local_ip: local,
                results,
            },
            route,
            size,
            forward_close,
        },
        packet_error,
    })
}

fn connection_route(reads: &[SavedRead]) -> std::result::Result<Option<Route>, String> {
    if reads.is_empty() {
        return Err("The profile has no saved CIP reads".into());
    }
    validate_saved_reads(reads)?;
    Ok(SavedReadMode::Connected.batch_route(reads)?.cloned())
}

pub(super) struct Class3OpenError {
    pub(super) message: String,
    pub(super) forward_close: Option<CloseOutcome>,
}

impl From<String> for Class3OpenError {
    fn from(message: String) -> Self {
        Self {
            message,
            forward_close: None,
        }
    }
}

impl fmt::Display for Class3OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(close) = &self.forward_close {
            write!(f, ". Forward_Close {close}")?;
        }
        Ok(())
    }
}

pub(super) fn with_connection<T>(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    route: Option<&Route>,
    size: Class3Size,
    run: impl FnOnce(&mut Class3<'_>) -> T,
) -> std::result::Result<(T, CloseOutcome), Class3OpenError> {
    if !is_unicast(adapter) || !is_unicast(local) {
        return Err("Enter unicast adapter and local NIC IPv4 addresses"
            .to_owned()
            .into());
    }

    let open = open_request(route, size).map_err(|error| error.to_string())?;
    let mut session = Session::open(adapter, local).map_err(|error| error.to_string())?;
    let (service, request, name) = if size.uses_large_forward_open() {
        (0x5b, encode_large_open(&open), "Large Forward_Open")
    } else {
        (0x54, open.encode(), "Forward_Open")
    };
    let operation = || match route {
        Some(route) => format!("Class 3 {name} via {route}"),
        None => format!("Class 3 {name}"),
    };
    let (body, _) = session
        .cip(service, 6, &request, None)
        .map_err(|error| format!("{}: {error}", operation()))?;
    let close = ForwardCloseRequest {
        priority_tick: open.priority_tick,
        timeout_ticks: open.timeout_ticks,
        connection_serial: open.connection_serial,
        originator_vendor: open.originator_vendor,
        originator_serial: open.originator_serial,
        connection_path: open.connection_path.clone(),
    };
    let reply = match decode_open(&body, &open) {
        Ok(reply) => reply,
        Err(error) => {
            let outcome = CloseOutcome::from_result(forward_close(&mut session, &close));
            return Err(Class3OpenError {
                message: format!("{}: {error}", operation()),
                forward_close: Some(outcome),
            });
        }
    };
    let result = run(&mut Class3 {
        session: &mut session,
        open: reply,
        size,
        sequence: 0,
    });
    let forward_close = CloseOutcome::from_result(forward_close(&mut session, &close));
    Ok((result, forward_close))
}

pub(super) fn connection_path(route: Option<&Route>) -> std::result::Result<Vec<u8>, String> {
    let mut path = route.map(Route::port_segments).unwrap_or_default();
    path.extend(MESSAGE_ROUTER);
    if path.len() > MAX_CIP_PATH_BYTES {
        return Err(
            "Class 3 connection path exceeds 255 words including the Message Router".into(),
        );
    }
    Ok(path)
}

fn open_request(route: Option<&Route>, size: Class3Size) -> Result<ForwardOpenRequest> {
    let connection_path = connection_path(route).map_err(protocol)?;
    let mut ids = [0; 10];
    getrandom::fill(&mut ids).map_err(|error| protocol(error.to_string()))?;
    let params = NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: 2,
        priority: 0,
        variable_size: true,
        size: size.bytes(),
    };
    Ok(ForwardOpenRequest {
        priority_tick: 0x0a,
        timeout_ticks: 0x0a,
        o_to_t_connection_id: 0,
        t_to_o_connection_id: le32(&ids, 0).max(1),
        connection_serial: le16(&ids, 4).max(1),
        originator_vendor: 0,
        originator_serial: le32(&ids, 6).max(1),
        connection_timeout_mult: 3,
        o_to_t_rpi_us: RPI_US,
        o_to_t_params: params,
        t_to_o_rpi_us: RPI_US,
        t_to_o_params: params,
        transport_type: 0xa3,
        connection_path,
    })
}

fn decode_open(body: &[u8], open: &ForwardOpenRequest) -> Result<ForwardOpenResponse> {
    let reply = ForwardOpenResponse::decode(body)?;
    if reply.connection_serial != open.connection_serial
        || reply.originator_vendor != open.originator_vendor
        || reply.originator_serial != open.originator_serial
        || reply.o_to_t_connection_id == 0
        || reply.t_to_o_connection_id == 0
    {
        return Err(protocol("Invalid Class 3 connection identifiers"));
    }
    Ok(reply)
}

pub(super) struct Class3<'a> {
    session: &'a mut Session,
    open: ForwardOpenResponse,
    size: Class3Size,
    sequence: u16,
}

impl Class3<'_> {
    pub(super) fn request(&mut self, cip: &[u8]) -> Result<Vec<u8>> {
        self.exchange(cip, |data| Ok(reply_body(data, cip[0])?.to_vec()))
    }

    fn exchange<T>(&mut self, cip: &[u8], decode: impl FnOnce(&[u8]) -> Result<T>) -> Result<T> {
        self.sequence = self.sequence.wrapping_add(1);
        let mut payload = self.sequence.to_le_bytes().to_vec();
        payload.extend_from_slice(cip);
        let items = [
            Item::connected_address(self.open.o_to_t_connection_id),
            Item::new(item_type::CONNECTED_DATA, payload),
        ];
        let (_, response) = self
            .session
            .exchange(Command::SendUnitData, &cpf::encode_envelope(0, 0, &items))?;
        let envelope = Envelope::parse(&response)?;
        decode(connected_payload(
            &envelope,
            self.open.t_to_o_connection_id,
            self.sequence,
            self.size,
        )?)
    }
}

fn connected_payload(
    envelope: &Envelope,
    connection_id: u32,
    sequence: u16,
    size: Class3Size,
) -> Result<&[u8]> {
    let [address, data] = envelope.items.as_slice() else {
        return Err(protocol(
            "Class 3 reply must contain an address and data item",
        ));
    };
    if address.type_id != item_type::CONNECTED_ADDRESS
        || address.data.as_slice() != connection_id.to_le_bytes()
    {
        return Err(protocol("Mismatched Class 3 reply connection ID"));
    }
    if data.type_id != item_type::CONNECTED_DATA
        || data.data.get(..2) != Some(sequence.to_le_bytes().as_slice())
    {
        return Err(protocol("Invalid Class 3 reply data or sequence"));
    }
    if data.data.len() > usize::from(size.bytes()) {
        return Err(protocol(format!(
            "Class 3 reply has {} bytes, beyond the requested {} bytes",
            data.data.len(),
            size.bytes()
        )));
    }
    Ok(&data.data[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class3_path_reserves_words_for_the_message_router() {
        let route: Route = format!("1/hex:{}/1/hex:{}", "ab".repeat(255), "cd".repeat(246))
            .parse()
            .unwrap();
        let path = connection_path(Some(&route)).unwrap();
        assert_eq!(path.len(), MAX_CIP_PATH_BYTES);
        assert!(path.ends_with(&MESSAGE_ROUTER));
        let larger: Route = format!("{route}/1/9").parse().unwrap();
        assert!(connection_path(Some(&larger)).is_err());
    }

    #[test]
    fn class3_reply_requires_both_the_connection_and_sequence() {
        let items = [
            Item::connected_address(123),
            Item::new(item_type::CONNECTED_DATA, vec![2, 0, 0x8e, 0, 0, 0, 7]),
        ];
        let frame = Envelope::parse(&cpf::encode_envelope(0, 0, &items)).unwrap();
        let size = Class3Size::default();
        let reply = connected_payload(&frame, 123, 2, size).unwrap();
        assert_eq!(reply_body(reply, 0x0e).unwrap(), [7]);
        assert!(connected_payload(&frame, 124, 2, size).is_err());
        assert!(connected_payload(&frame, 123, 1, size).is_err());
        assert!(reply_body(reply, 0x01).is_err());
    }

    #[test]
    fn class3_reply_size_counts_the_sequence_for_either_open_service() {
        let size = Class3Size::try_from(512).unwrap();
        assert!(size.uses_large_forward_open());
        let mut payload = vec![0; 512];
        payload[..2].copy_from_slice(&1u16.to_le_bytes());
        let items = [
            Item::connected_address(123),
            Item::new(item_type::CONNECTED_DATA, payload),
        ];
        let frame = Envelope::parse(&cpf::encode_envelope(0, 0, &items)).unwrap();
        assert_eq!(connected_payload(&frame, 123, 1, size).unwrap().len(), 510);
        assert!(connected_payload(&frame, 123, 1, Class3Size::try_from(511).unwrap()).is_err());
    }
}
