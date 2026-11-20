use assembly_scanner::{
    explicit::{EipError, ReadPath, Reader, Route, RoutedReadError, SavedRead},
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
    let (document, success) = read(&mut reader, adapter, path, route, selection)?;
    drop(reader);
    print_json(&document)?;
    Ok(success)
}

pub(super) fn all(adapter: Ipv4Addr, local: Ipv4Addr, reads: &[SavedRead]) -> Result<bool, String> {
    if reads.is_empty() {
        return Err("The profile has no saved CIP reads".into());
    }
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let mut session_lost = false;
    let mut success = true;
    let mut results = Vec::with_capacity(reads.len());
    for saved in reads {
        let mut document = json!({"path": saved.path, "status": "not_attempted"});
        if !session_lost {
            match read(
                &mut reader,
                adapter,
                saved.path,
                saved.route.as_ref(),
                saved.selection,
            ) {
                Ok((reply, ok)) => {
                    document = reply;
                    document["status"] = json!(if ok { "success" } else { "cip_rejected" });
                    success &= ok;
                }
                Err(error) => {
                    document["status"] = json!("exchange_failed");
                    document["error"] = json!(error);
                    session_lost = true;
                    success = false;
                }
            }
        }
        if let Some(route) = &saved.route {
            document["route"] = json!(route);
        }
        document["name"] = json!(saved.name);
        results.push(document);
    }
    drop(reader);
    print_json(&json!({"adapter_or_bridge": adapter, "local_ip": local, "results": results}))?;
    Ok(success)
}

fn read(
    reader: &mut Reader,
    adapter: Ipv4Addr,
    path: ReadPath,
    route: Option<&Route>,
    selection: Option<Selection>,
) -> Result<(Value, bool), String> {
    let result = match route {
        Some(route) => reader.read_routed(path, route),
        None => reader.read(path).map_err(|error| match error {
            error @ EipError::Cip { .. } => RoutedReadError::Target(error),
            error => RoutedReadError::Exchange(error),
        }),
    };
    let mut report = match &result {
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
    Ok((report, result.is_ok()))
}

fn print_json(document: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(document).map_err(|error| error.to_string())?
    );
    Ok(())
}
