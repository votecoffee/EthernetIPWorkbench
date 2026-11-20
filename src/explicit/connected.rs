use super::{
    Command, EipError, Envelope, Item, Result, RoutedReadError, SavedRead, SavedReadBatch, Session,
    batch::run_ordered, cpf, is_unicast, item_type, le16, le32, protocol, read_message, reply_body,
    validate_saved_reads,
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
    pub forward_close: CloseOutcome,
}

/// Run direct saved reads over one short-lived Class 3 TCP connection.
/// A CIP rejection is saved with its request. An exchange failure stops subsequent reads.
pub fn run_saved_reads_connected(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> std::result::Result<ConnectedReadBatch, String> {
    if reads.is_empty() {
        return Err("The profile has no saved CIP reads".into());
    }
    validate_saved_reads(&reads)?;
    if reads.iter().any(|request| request.route.is_some()) {
        return Err("Class 3 connected mode requires direct saved reads".into());
    }
    if !is_unicast(adapter) || !is_unicast(local) {
        return Err("Enter unicast adapter and local NIC IPv4 addresses".into());
    }

    let mut session = Session::open(adapter, local).map_err(|error| error.to_string())?;
    let open = open_request().map_err(|error| error.to_string())?;
    let (body, _) = session
        .cip(0x54, 6, &open.encode(), None)
        .map_err(|error| format!("Class 3 Forward_Open: {error}"))?;
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
            return Err(format!(
                "Class 3 Forward_Open: {error}. Forward_Close {outcome}"
            ));
        }
    };
    let mut sequence = 0u16;
    let results = run_ordered(reads, |request| {
        sequence = sequence.wrapping_add(1);
        let cip = read_message(request.path);
        connected_read(&mut session, &reply, sequence, &cip).map_err(|error| match error {
            error @ EipError::Cip { .. } => RoutedReadError::Target(error),
            error => RoutedReadError::Exchange(error),
        })
    });
    let forward_close = CloseOutcome::from_result(forward_close(&mut session, &close));
    Ok(ConnectedReadBatch {
        batch: SavedReadBatch {
            adapter_or_bridge: adapter,
            local_ip: local,
            results,
        },
        forward_close,
    })
}

fn open_request() -> Result<ForwardOpenRequest> {
    let mut ids = [0; 10];
    getrandom::fill(&mut ids).map_err(|error| protocol(error.to_string()))?;
    let params = NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: 2,
        priority: 0,
        variable_size: true,
        size: CONNECTED_SIZE,
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
        connection_path: MESSAGE_ROUTER.to_vec(),
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

fn connected_read(
    session: &mut Session,
    open: &ForwardOpenResponse,
    sequence: u16,
    cip: &[u8],
) -> Result<Vec<u8>> {
    let mut payload = sequence.to_le_bytes().to_vec();
    payload.extend_from_slice(cip);
    let items = [
        Item::connected_address(open.o_to_t_connection_id),
        Item::new(item_type::CONNECTED_DATA, payload),
    ];
    let (_, response) =
        session.exchange(Command::SendUnitData, &cpf::encode_envelope(0, 0, &items))?;
    let envelope = Envelope::parse(&response)?;
    connected_reply(&envelope, open.t_to_o_connection_id, sequence, cip[0])
}

fn connected_reply(
    envelope: &Envelope,
    connection_id: u32,
    sequence: u16,
    service: u8,
) -> Result<Vec<u8>> {
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
    Ok(reply_body(&data.data[2..], service)?.to_vec())
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
        assert_eq!(connected_reply(&frame, 123, 2, 0x0e).unwrap(), [7]);
        assert!(connected_reply(&frame, 124, 2, 0x0e).is_err());
        assert!(connected_reply(&frame, 123, 1, 0x0e).is_err());
        assert!(connected_reply(&frame, 123, 2, 0x01).is_err());
    }
}
