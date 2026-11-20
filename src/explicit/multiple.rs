use super::{
    EipError, Reader, ReplyHeader, Result, RoutedReadError, SavedRead, SavedReadBatch,
    SavedReadOutcome, SavedReadResult, cip_data, le16, protocol, read_message, reply_body,
    validate_saved_reads,
};
use std::net::Ipv4Addr;

const SERVICE: u8 = 0x0a;
const EMBEDDED_SERVICE_ERROR: u8 = 0x1e;

/// Run direct saved reads in one unconnected Multiple Service Packet.
/// Return an error for a failed envelope or an inconsistent reply table.
pub fn run_saved_reads_packet(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> std::result::Result<SavedReadBatch, String> {
    if reads.is_empty() {
        return Err("The profile has no saved CIP reads".into());
    }
    validate_saved_reads(&reads)?;
    if reads.iter().any(|entry| entry.route.is_some()) {
        return Err("Multiple Service Packet requires direct saved reads".into());
    }
    let requests: Vec<_> = reads.iter().map(|entry| read_message(entry.path)).collect();
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let replies = reader
        .read_packet(&requests)
        .map_err(|error| format!("Multiple Service Packet: {error}"))?;
    let results = reads
        .into_iter()
        .zip(replies)
        .map(|(request, outcome)| SavedReadResult { request, outcome })
        .collect();
    Ok(SavedReadBatch {
        adapter_or_bridge: adapter,
        local_ip: local,
        results,
    })
}

impl Reader {
    fn read_packet(&mut self, requests: &[Vec<u8>]) -> Result<Vec<SavedReadOutcome>> {
        let session = self.0.as_mut().ok_or(EipError::Closed)?;
        let reply = session.unconnected(request_packet(requests), None)?;
        decode_packet(cip_data(&reply)?, requests)
    }
}

fn request_packet(requests: &[Vec<u8>]) -> Vec<u8> {
    // The validated profile contains at most 64 reads, each at most 20 bytes.
    let mut packet = vec![SERVICE, 2, 0x20, 2, 0x24, 1];
    packet.extend_from_slice(&(requests.len() as u16).to_le_bytes());
    let mut offset = 2 + 2 * requests.len();
    for request in requests {
        packet.extend_from_slice(&(offset as u16).to_le_bytes());
        offset += request.len();
    }
    for request in requests {
        packet.extend_from_slice(request);
    }
    packet
}

fn decode_packet(data: &[u8], requests: &[Vec<u8>]) -> Result<Vec<SavedReadOutcome>> {
    let header = ReplyHeader::parse(data)?;
    if data[0] != (SERVICE | 0x80) {
        return Err(protocol("Unexpected Multiple Service Packet reply service"));
    }
    if !matches!(header.general_status, 0 | EMBEDDED_SERVICE_ERROR) {
        return Err(EipError::Cip {
            status: header.general_status,
            ext: header.extended_status,
        });
    }

    let payload = &data[header.body_offset..];
    let table_end = 2 + requests.len() * 2;
    let table = payload
        .get(..table_end)
        .ok_or_else(|| protocol("Truncated Multiple Service Packet reply table"))?;
    if usize::from(le16(table, 0)) != requests.len() {
        return Err(protocol(
            "Multiple Service Packet reply count differs from the request",
        ));
    }
    let mut offsets: Vec<_> = table[2..]
        .chunks_exact(2)
        .map(|bytes| usize::from(le16(bytes, 0)))
        .collect();
    if offsets.first() != Some(&table_end)
        || offsets.last().is_none_or(|last| *last >= payload.len())
        || offsets.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(protocol("Invalid Multiple Service Packet reply offsets"));
    }
    offsets.push(payload.len());

    let mut outcomes = Vec::with_capacity(requests.len());
    for (request, pair) in requests.iter().zip(offsets.windows(2)) {
        let outcome = match reply_body(&payload[pair[0]..pair[1]], request[0]) {
            Ok(body) => SavedReadOutcome::Reply(body.to_vec()),
            Err(error @ EipError::Cip { .. }) => {
                SavedReadOutcome::Rejected(RoutedReadError::Target(error))
            }
            Err(error) => return Err(error),
        };
        outcomes.push(outcome);
    }
    if header.general_status == EMBEDDED_SERVICE_ERROR
        && !outcomes
            .iter()
            .any(|outcome| matches!(outcome, SavedReadOutcome::Rejected(_)))
    {
        return Err(protocol(
            "Multiple Service Packet reports an embedded error without a rejected reply",
        ));
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_offsets_or_reply_counts_that_cannot_match_requests() {
        let requests = [vec![0x0e], vec![0x01]];
        let data = [
            0x8a, 0, 0, 0, 2, 0, 6, 0, 11, 0, 0x8e, 0, 0, 0, 7, 0x81, 0, 0, 0,
        ];
        assert!(matches!(
            decode_packet(&data, &requests).unwrap().as_slice(),
            [SavedReadOutcome::Reply(first), SavedReadOutcome::Reply(second)]
                if first == &[7] && second.is_empty()
        ));
        for (index, value) in [(2, 0x1e), (4, 1), (6, 4), (8, 6), (8, 25), (15, 0x8e)] {
            let mut changed = data;
            changed[index] = value;
            assert!(decode_packet(&changed, &requests).is_err());
        }
    }
}
