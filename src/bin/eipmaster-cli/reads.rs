use assembly_scanner::{
    explicit::{
        EipError, ReadPath, Reader, Route, RoutedReadError, SavedRead, SavedReadOutcome,
        run_saved_reads,
    },
    model::{Selection, hex},
};
use serde_json::{Value, json};
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
    let document = read_document(adapter, path, route, selection, reply.as_deref())?;
    print_json(&document)?;
    Ok(reply.is_ok())
}

pub(super) fn all(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    reads: Vec<SavedRead>,
) -> Result<bool, String> {
    let batch = run_saved_reads(adapter, local, reads)?;
    let mut success = true;
    let mut results = Vec::with_capacity(batch.results.len());
    for entry in batch.results {
        let saved = entry.request;
        let report = |reply| {
            read_document(
                adapter,
                saved.path,
                saved.route.as_ref(),
                saved.selection,
                reply,
            )
        };
        let (mut document, status) = match &entry.outcome {
            SavedReadOutcome::Reply(bytes) => (report(Ok(bytes.as_slice()))?, "success"),
            SavedReadOutcome::Rejected(error) => (report(Err(error))?, "cip_rejected"),
            SavedReadOutcome::ExchangeFailed(error) => (
                json!({"path": saved.path, "error": error.to_string()}),
                "exchange_failed",
            ),
            SavedReadOutcome::NotAttempted => (json!({"path": saved.path}), "not_attempted"),
        };
        success &= status == "success";
        if let Some(route) = &saved.route {
            document["route"] = json!(route);
        }
        document["status"] = json!(status);
        document["name"] = json!(saved.name);
        results.push(document);
    }
    print_json(&json!({"adapter_or_bridge": adapter, "local_ip": local, "results": results}))?;
    Ok(success)
}

fn read_document(
    adapter: Ipv4Addr,
    path: ReadPath,
    route: Option<&Route>,
    selection: Option<Selection>,
    result: Result<&[u8], &RoutedReadError>,
) -> Result<Value, String> {
    let mut report = match result {
        Ok(bytes) => {
            let decoded = path.decode_reply(bytes, selection);
            let mut report = json!({
                "path": path,
                "data_hex": hex(bytes),
                "bytes": bytes.len(),
                "decoded": decoded.as_ref().ok().and_then(|value| value.as_ref()),
            });
            if let Some(selection) = selection {
                report["selection"] = json!(selection);
            }
            if let Err(error) = decoded {
                report["decode_error"] = json!(error);
            }
            report
        }
        Err(
            RoutedReadError::Router(EipError::Cip { status, ext })
            | RoutedReadError::Target(EipError::Cip { status, ext }),
        ) => json!({"path": path, "general_status": status, "extended_status": ext}),
        Err(error) => return Err(error.to_string()),
    };
    if let Some(route) = route {
        report["bridge"] = json!(adapter);
        report["route"] = json!(route);
        report["reply_from"] = json!(if matches!(result, Err(RoutedReadError::Router(_))) {
            "router"
        } else {
            "target"
        });
    }
    Ok(report)
}

fn print_json(document: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(document).map_err(|error| error.to_string())?
    );
    Ok(())
}
