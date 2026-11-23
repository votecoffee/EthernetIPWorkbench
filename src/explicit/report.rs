use super::{
    AttributeListReport, Class3Size, ClassInfo, ConnectedPacketBatch, ConnectedReadBatch, EipError,
    ReadPath, Route, RoutedReadError, SavedReadBatch, SavedReadOutcome, SavedReadReport,
    WriteDraft, WriteInspection,
};
use crate::model::{Selection, hex};
use crate::scanner::CloseOutcome;
use serde_json::{Value, json};
use std::net::Ipv4Addr;

impl super::ConnectionDataReport {
    pub fn report_json(&self) -> Result<String, String> {
        let path = super::connection_data::PATH;
        let mut document = read_reply_document(
            self.adapter_or_bridge,
            path,
            self.route.as_ref(),
            None,
            self.reply.as_deref(),
        )
        .unwrap_or_else(|error| json!({"path": path, "error": error}));
        match self.decoded() {
            Ok(Some(connection)) => document["decoded"] = json!(connection),
            Ok(None) => {}
            Err(error) => document["decode_error"] = json!(error),
        }
        if matches!(self.reply, Ok(_) | Err(RoutedReadError::Target(_))) {
            document["reply_from"] = json!("target");
        }
        document["adapter_or_bridge"] = json!(self.adapter_or_bridge);
        document["local_ip"] = json!(self.local_ip);
        document["route"] = json!(self.route);
        document["query"] = json!(self.query);
        document["service"] = json!(self.query.service_name());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl AttributeListReport {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = match &self.reply {
            Ok(reply) => {
                let mut report = json!({
                    "reply_from": "target",
                    "general_status": reply.general_status,
                    "extended_status": reply.extended_status,
                    "data_hex": hex(&reply.body),
                });
                match &reply.entries {
                    Ok(Some(entries)) => {
                        report["results"] = json!(entries.iter().map(|entry| {
                            let path = ReadPath {
                                attribute: Some(entry.attribute.into()),
                                ..self.path
                            };
                            json!({
                                "attribute": entry.attribute,
                                "status": entry.status,
                                "data_hex": entry.data.as_deref().map(hex),
                                "decoded": entry.data.as_deref()
                                    .and_then(|data| path.decode_reply(data, None).ok().flatten()),
                            })
                        }).collect::<Vec<_>>());
                    }
                    Ok(None) => {}
                    Err(error) => report["decode_error"] = json!(error),
                }
                report
            }
            Err(error) => read_reply_document(
                self.adapter_or_bridge,
                self.path,
                self.route.as_ref(),
                None,
                Err(error),
            )
            .unwrap_or_else(|error| json!({"error": error})),
        };
        document["service"] = json!("Get_Attribute_List");
        document["adapter_or_bridge"] = json!(self.adapter_or_bridge);
        document["local_ip"] = json!(self.local_ip);
        document["path"] = json!(self.path);
        document["route"] = json!(self.route);
        document["requested_attributes"] = json!(self.fields);
        document["complete"] = json!(self.is_complete());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

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
        document["optional_instance_attributes"] = json!(self.optional_instance_attributes());
        document["optional_services"] = json!(self.optional_services());
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
        let mut document = instance_document(
            &self.batch,
            super::TCPIP_INTERFACE_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        if let Some(diagnostics) = self.diagnostics() {
            document["network_diagnostics"] = json!(diagnostics);
        }
        if let Some(diagnostics) = self.address_conflicts() {
            document["address_conflict_detection"] = json!(diagnostics);
        }
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::LinkInfo {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::ETHERNET_LINK_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        if let Some(details) = self.port_details() {
            document["port_details"] = json!(details);
        }
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::LldpNeighborReport {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::LLDP_DATA_TABLE_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        document["neighbor"] = json!(self.neighbor());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::DlrReport {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::DLR_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        document["dlr"] = json!(self.status());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::DlrParticipantReport {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::DLR_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        document["participant_list"] = json!({
            "count_before": self.count_before,
            "count_after": self.count_after,
            "returned_count": self.participants.as_ref().map(Vec::len),
            "count_status": self.count_status(),
            "entries": self.participants,
        });
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::TimeSyncReport {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::TIME_SYNC_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        document["time_sync"] = json!(self.status());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::ManagerInfo {
    pub fn report_json(&self) -> Result<String, String> {
        instance_json(
            &self.batch,
            super::CONNECTION_MANAGER_CLASS,
            self.instance,
            self.route.as_ref(),
        )
    }
}

impl super::QosInfo {
    pub fn report_json(&self) -> Result<String, String> {
        instance_json(
            &self.batch,
            super::QOS_CLASS,
            self.instance,
            self.route.as_ref(),
        )
    }
}

impl super::AssemblyInfo {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::ASSEMBLY_CLASS,
            self.instance,
            self.route.as_ref(),
        )?;
        document["declared_size_bytes"] = json!(self.declared_size());
        document["data_size_bytes"] = json!(self.data_size());
        document["sizes_match"] = json!(self.sizes_match());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::ProfileAssemblyCheck {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = self.batch.report_document()?;
        if let Some(route) = &self.route {
            document["route"] = json!(route);
        }
        document["checks"] = json!(self.checks);
        document["matches_profile"] = json!(self.matches_profile());
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::ObjectList {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = instance_document(
            &self.batch,
            super::object_list::MESSAGE_ROUTER_CLASS,
            super::object_list::MESSAGE_ROUTER_INSTANCE,
            self.route.as_ref(),
        )?;
        document["class_ids"] = json!(self.class_ids);
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

impl super::InstancePage {
    pub fn report_json(&self) -> Result<String, String> {
        let mut document = read_reply_document(
            self.adapter_or_bridge,
            self.path(),
            self.route.as_ref(),
            None,
            self.reply.as_deref(),
        )
        .unwrap_or_else(|error| json!({"path": self.path(), "error": error}));
        document["adapter_or_bridge"] = json!(self.adapter_or_bridge);
        document["local_ip"] = json!(self.local_ip);
        document["service"] = json!("Find_Next_Object_Instance");
        document["requested_max"] = json!(super::INSTANCE_PAGE_SIZE);
        if let Some(route) = &self.route {
            document["route"] = json!(route);
        }
        let decoded = match self.decoded() {
            Ok(ids) => ids,
            Err(error) => {
                document["decode_error"] = json!(error);
                None
            }
        };
        document["instance_ids"] = json!(decoded.as_ref().map(|page| &page.ids));
        document["next_after"] = json!(decoded.and_then(|page| page.next_after));
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
    }
}

fn instance_json(
    batch: &SavedReadBatch,
    class: u32,
    instance: u32,
    route: Option<&Route>,
) -> Result<String, String> {
    let document = instance_document(batch, class, instance, route)?;
    serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
}

fn instance_document(
    batch: &SavedReadBatch,
    class: u32,
    instance: u32,
    route: Option<&Route>,
) -> Result<Value, String> {
    let mut document = batch.report_document()?;
    document["class"] = json!(class);
    document["instance"] = json!(instance);
    if let Some(route) = route {
        document["route"] = json!(route);
    }
    Ok(document)
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
