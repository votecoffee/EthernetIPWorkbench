use assembly_scanner::{
    explicit::{
        ReadPath, Reader, Route, SavedRead, SavedReadOutcome, read_reply_document, run_saved_reads,
        run_saved_reads_connected, run_saved_reads_connected_packet, run_saved_reads_packet,
    },
    model::Selection,
};
use std::net::Ipv4Addr;

pub(super) fn single(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    path: ReadPath,
    route: Option<&Route>,
    selection: Option<Selection>,
) -> Result<bool, String> {
    path.validate()?;
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let reply = reader.read_request(path, route);
    drop(reader);
    let document = read_reply_document(adapter, path, route, selection, reply.as_deref())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?
    );
    Ok(reply.is_ok())
}

pub(super) enum Mode {
    Sequential,
    Packet,
    Connected,
    ConnectedPacket,
}

pub(super) fn all(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    mode: Mode,
) -> Result<bool, String> {
    let (batch, confirmed) = match mode {
        Mode::ConnectedPacket => {
            let packet = run_saved_reads_connected_packet(adapter, local, reads)?;
            println!("{}", packet.report_json()?);
            let complete =
                packet.packet_error.is_none() && packet.connection.forward_close.is_confirmed();
            (packet.connection.batch, complete)
        }
        Mode::Connected => {
            let connected = run_saved_reads_connected(adapter, local, reads)?;
            println!("{}", connected.report_json()?);
            (connected.batch, connected.forward_close.is_confirmed())
        }
        Mode::Sequential | Mode::Packet => {
            let batch = if matches!(mode, Mode::Packet) {
                run_saved_reads_packet(adapter, local, reads)?
            } else {
                run_saved_reads(adapter, local, reads)?
            };
            println!("{}", batch.report_json()?);
            (batch, true)
        }
    };
    Ok(confirmed
        && batch
            .results
            .iter()
            .all(|entry| matches!(entry.outcome, SavedReadOutcome::Reply(_))))
}
