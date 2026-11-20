use super::{EipError, ReadPath, Route, RoutedReadError, SavedReadBatch, SavedReadOutcome};
use crate::model::{Selection, hex};
use serde_json::{Value, json};
use std::net::Ipv4Addr;

impl SavedReadBatch {
    /// Format this batch for the CLI and for saving the GUI results.
    pub fn report_json(&self) -> Result<String, String> {
        let mut results = Vec::with_capacity(self.results.len());
        for entry in &self.results {
            let saved = &entry.request;
            let report = |reply| {
                read_reply_document(
                    self.adapter_or_bridge,
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
            if let Some(route) = &saved.route {
                document["route"] = json!(route);
            }
            document["status"] = json!(status);
            document["name"] = json!(saved.name);
            results.push(document);
        }
        serde_json::to_string_pretty(&json!({
            "adapter_or_bridge": self.adapter_or_bridge,
            "local_ip": self.local_ip,
            "results": results,
        }))
        .map_err(|error| error.to_string())
    }
}

/// Format a successful reply or a CIP rejection with its raw data and source.
pub fn read_reply_document(
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
