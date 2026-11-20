use assembly_scanner::{
    explicit::{
        ReadPath, Reader, Route, SavedRead, SavedReadMode, read_reply_document,
        run_saved_reads_with_mode,
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

pub(super) fn all(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
    mode: SavedReadMode,
) -> Result<bool, String> {
    let report = run_saved_reads_with_mode(adapter, local, reads, mode)?;
    println!("{}", report.report_json()?);
    Ok(report.is_complete())
}
