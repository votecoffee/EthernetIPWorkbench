use super::multiple::{decode_packet, request_packet};
use super::{
    Command, EipError, Envelope, Item, Result, Route, RoutedReadError, SavedRead, SavedReadBatch,
    SavedReadMode, SavedReadResult, Session, batch::run_ordered, cpf, is_unicast, item_type, le16,
    le32, protocol, read_message, reply_body, validate_saved_reads,
};
use crate::scanner::{CloseOutcome, forward_close};
use ethernetip_connections::forward_open::{
    ForwardCloseRequest, ForwardOpenRequest, ForwardOpenResponse, NetworkConnectionParameters,
};
use std::net::Ipv4Addr;

const MESSAGE_ROUTER: [u8; 4] = [0x20, 2, 0x24, 1];
const CONNECTED_SIZE: u16 = 504;
const RPI_US: u32 = 2_000_000;

pub struct ConnectedReadBatch {
    pub batch: SavedReadBatch,
    pub route: Option<Route>,
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
    let route = connection_route(&reads)?;
    let (results, forward_close) =
        with_connection(adapter, local, route.as_ref(), |session, reply| {
            let mut sequence = 0u16;
            run_ordered(reads, |request| {
                sequence = sequence.wrapping_add(1);
                let cip = read_message(request.path);
                connected_exchange(session, reply, sequence, &cip, |data| {
                    Ok(reply_body(data, cip[0])?.to_vec())
                })
                .map_err(|error| match error {
                    error @ EipError::Cip { .. } => RoutedReadError::Target(error),
                    error => RoutedReadError::Exchange(error),
                })
            })
        })?;
    Ok(ConnectedReadBatch {
        batch: SavedReadBatch {
            adapter_or_bridge: adapter,
            local_ip: local,
            results,
        },
        route,
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
    let route = connection_route(&reads)?;
    let requests: Vec<_> = reads.iter().map(|entry| read_message(entry.path)).collect();
    let packet = request_packet(&requests);
    let limit = usize::from(CONNECTED_SIZE) - 2;
    if packet.len() > limit {
        return Err(format!(
            "Class 3 packet needs {} CIP bytes. The limit is {limit} bytes.",
            packet.len()
        ));
    }
    let (reply, forward_close) =
        with_connection(adapter, local, route.as_ref(), |session, open| {
            connected_exchange(session, open, 1, &packet, |data| {
                decode_packet(data, &requests)
            })
        })?;
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

fn with_connection<T>(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    route: Option<&Route>,
    run: impl FnOnce(&mut Session, &ForwardOpenResponse) -> T,
) -> std::result::Result<(T, CloseOutcome), String> {
    if !is_unicast(adapter) || !is_unicast(local) {
        return Err("Enter unicast adapter and local NIC IPv4 addresses".into());
    }

    let mut session = Session::open(adapter, local).map_err(|error| error.to_string())?;
    let open = open_request(route).map_err(|error| error.to_string())?;
    let operation = || match route {
        Some(route) => format!("Class 3 Forward_Open via {route}"),
        None => "Class 3 Forward_Open".to_owned(),
    };
    let (body, _) = session
        .cip(0x54, 6, &open.encode(), None)
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
            return Err(format!("{}: {error}. Forward_Close {outcome}", operation(),));
        }
    };
    let result = run(&mut session, &reply);
    let forward_close = CloseOutcome::from_result(forward_close(&mut session, &close));
    Ok((result, forward_close))
}

fn open_request(route: Option<&Route>) -> Result<ForwardOpenRequest> {
    let mut ids = [0; 10];
    getrandom::fill(&mut ids).map_err(|error| protocol(error.to_string()))?;
    let params = NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: 2,
        priority: 0,
        variable_size: true,
        size: CONNECTED_SIZE,
    };
    let mut connection_path = route.map(Route::port_segments).unwrap_or_default();
    connection_path.extend(MESSAGE_ROUTER);
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

fn connected_exchange<T>(
    session: &mut Session,
    open: &ForwardOpenResponse,
    sequence: u16,
    cip: &[u8],
    decode: impl FnOnce(&[u8]) -> Result<T>,
) -> Result<T> {
    let mut payload = sequence.to_le_bytes().to_vec();
    payload.extend_from_slice(cip);
    let items = [
        Item::connected_address(open.o_to_t_connection_id),
        Item::new(item_type::CONNECTED_DATA, payload),
    ];
    let (_, response) =
        session.exchange(Command::SendUnitData, &cpf::encode_envelope(0, 0, &items))?;
    let envelope = Envelope::parse(&response)?;
    decode(connected_payload(
        &envelope,
        open.t_to_o_connection_id,
        sequence,
    )?)
}

fn connected_payload(envelope: &Envelope, connection_id: u32, sequence: u16) -> Result<&[u8]> {
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
        || data.data.len() > usize::from(CONNECTED_SIZE)
        || data.data.get(..2) != Some(sequence.to_le_bytes().as_slice())
    {
        return Err(protocol("Invalid Class 3 reply data or sequence"));
    }
    Ok(&data.data[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class3_reply_requires_both_the_connection_and_sequence() {
        let items = [
            Item::connected_address(123),
            Item::new(item_type::CONNECTED_DATA, vec![2, 0, 0x8e, 0, 0, 0, 7]),
        ];
        let frame = Envelope::parse(&cpf::encode_envelope(0, 0, &items)).unwrap();
        let reply = connected_payload(&frame, 123, 2).unwrap();
        assert_eq!(reply_body(reply, 0x0e).unwrap(), [7]);
        assert!(connected_payload(&frame, 124, 2).is_err());
        assert!(connected_payload(&frame, 123, 1).is_err());
        assert!(reply_body(reply, 0x01).is_err());
    }
}
