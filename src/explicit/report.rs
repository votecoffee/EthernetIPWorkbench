use super::{
    Class3Size, ClassInfo, ConnectedPacketBatch, ConnectedReadBatch, EipError, ReadPath, Route,
    RoutedReadError, SavedReadBatch, SavedReadOutcome, SavedReadReport, WriteDraft,
    WriteInspection,
};
use crate::model::{Selection, hex};
use crate::scanner::CloseOutcome;
use serde_json::{Value, json};
use std::net::Ipv4Addr;

impl WriteDraft {
    /// Describe the exact wire bytes and transport shared by write previews and receipts.
    pub fn report_document(&self) -> Value {
        let request = &self.request;
        let mut document = json!({
            "adapter_or_bridge": request.adapter_or_bridge,
            "local_ip": request.local_ip,
            "route": request.route,
            "path": request.path,
            "service": "Set_Attribute_Single",
            "value_hex": hex(&request.value),
            "bytes": request.value.len(),
        });
        if self.readback {
            document["readback_requested"] = json!(true);
        }
        if let Some(bytes) = &self.expected_current {
            document["expected_current_hex"] = json!(hex(bytes));
        }
        if let Some(size) = self.class3_size {
            document["class3_size"] = json!(size);
            document["large_forward_open"] = json!(size.uses_large_forward_open());
        }
        document
    }
}

impl WriteInspection {
    /// Format the attribute read and any Class 3 close as separate results.
    pub fn result_document(&self) -> Value {
        let current = match &self.current {
            Ok(data) => json!({"outcome": "read", "data_hex": hex(data), "bytes": data.len()}),
            Err(RoutedReadError::Target(EipError::Cip { status, ext })) => {
                json!({"outcome": "rejected", "general_status": status, "extended_status": ext})
            }
            Err(RoutedReadError::Router(EipError::Cip { status, ext })) => {
                json!({"outcome": "router_error", "general_status": status, "extended_status": ext})
            }
            Err(error) => json!({"outcome": "unavailable", "error": error.to_string()}),
        };
        let mut report = json!({
            "outcome": "inspection",
            "service": "Get_Attribute_Single",
            "observed_identity": self.identity,
            "current": current,
        });
        if let Some(close) = &self.forward_close {
            report["forward_close"] = json!(close);
        }
        report
    }
}

impl SavedReadBatch {
    /// Format this batch for the CLI and for saving the GUI results.
    pub fn report_json(&self) -> Result<String, String> {
        batch_json(self, None, None, None, None)
    }

    fn report_document(&self) -> Result<Value, String> {
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
        Ok(json!({
            "adapter_or_bridge": self.adapter_or_bridge,
            "local_ip": self.local_ip,
            "results": results,
        }))
    }
}

impl ConnectedReadBatch {
    /// Format the read results with the separate Class 3 Forward_Close outcome.
    pub fn report_json(&self) -> Result<String, String> {
        batch_json(
            &self.batch,
            self.route.as_ref(),
            Some(self.size),
            Some(&self.forward_close),
            None,
        )
    }
}

impl ClassInfo {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = self.batch.report_document()?;
        document["class"] = json!(self.class);
        if let Some(route) = &self.route {
            document["route"] = json!(route);
        }
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::PortInfo {
    pub fn report_json(&self) -> Result<String, String> {
        instance_json(
            &self.batch,
            super::PORT_OBJECT_CLASS,
            self.instance,
            self.route.as_ref(),
        )
    }
}

impl super::TcpIpInfo {
    pub fn report_json(&self) -> Result<String, String> {
        instance_json(
            &self.batch,
            super::TCPIP_INTERFACE_CLASS,
            self.instance,
            self.route.as_ref(),
        )
    }
}

impl super::LinkInfo {
    pub fn report_json(&self) -> Result<String, String> {
        instance_json(
            &self.batch,
            super::ETHERNET_LINK_CLASS,
            self.instance,
            self.route.as_ref(),
        )
    }
}

fn instance_json(
    batch: &SavedReadBatch,
    class: u32,
    instance: u32,
    route: Option<&Route>,
) -> Result<String, String> {
    let mut document = batch.report_document()?;
    document["class"] = json!(class);
    document["instance"] = json!(instance);
    if let Some(route) = route {
        document["route"] = json!(route);
    }
    serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
}

impl ConnectedPacketBatch {
    /// Format the results, any whole-packet failure, and the Class 3 close outcome.
    pub fn report_json(&self) -> Result<String, String> {
        batch_json(
            &self.connection.batch,
            self.connection.route.as_ref(),
            Some(self.connection.size),
            Some(&self.connection.forward_close),
            self.packet_error.as_ref(),
        )
    }
}

impl SavedReadReport {
    /// Format the batch using the same report fields as the individual read runners.
    pub fn report_json(&self) -> Result<String, String> {
        batch_json(
            &self.batch,
            self.connection_route.as_ref(),
            self.class3_size,
            self.forward_close.as_ref(),
            self.packet_error.as_ref(),
        )
    }
}

fn batch_json(
    batch: &SavedReadBatch,
    connection_route: Option<&Route>,
    class3_size: Option<Class3Size>,
    forward_close: Option<&CloseOutcome>,
    packet_error: Option<&EipError>,
) -> Result<String, String> {
    let mut document = batch.report_document()?;
    if let Some(route) = connection_route {
        document["connection_route"] = json!(route);
    }
    if let Some(size) = class3_size {
        document["class3_size"] = json!(size);
        document["large_forward_open"] = json!(size.uses_large_forward_open());
    }
    if let Some(outcome) = forward_close {
        document["forward_close"] = json!(outcome);
    }
    if let Some(error) = packet_error {
        document["packet_error"] = match error {
            EipError::Cip { status, ext } => {
                json!({"general_status": status, "extended_status": ext})
            }
            error => json!({"error": error.to_string()}),
        };
    }
    serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
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
