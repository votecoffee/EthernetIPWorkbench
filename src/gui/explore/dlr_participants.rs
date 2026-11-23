use super::{AMBER, DlrParticipantReport, Explorer, Ipv4Addr, Response, Ui, batch, egui, explicit};
use explicit::DlrParticipantCountStatus;

impl Explorer {
    pub(super) fn dlr_participants_action(
        &mut self,
        ui: &mut Ui,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) {
        if !ui
            .add_enabled(!self.busy(), egui::Button::new("Read ring participants"))
            .on_hover_text(
                "Read the selected supervisor's participant list and compare its reported counts.",
            )
            .clicked()
        {
            return;
        }
        self.dlr_participants = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_dlr_participants(adapter, local, instance, route)
                    .map(Response::DlrParticipants)
            }),
            Err(error) => self.notice = Some(error),
        }
    }
}

pub(super) fn show(ui: &mut Ui, info: &DlrParticipantReport) {
    egui::CollapsingHeader::new(format!("Ring participants · Instance {}", info.instance))
        .id_salt("dlr-participant-list")
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                info.batch.adapter_or_bridge, info.batch.local_ip
            ));
            if let Some(route) = &info.route {
                ui.small(format!("Route {route}"));
            }
            let count = |value: Option<u16>| {
                value.map_or_else(|| "Unavailable".into(), |value| value.to_string())
            };
            let before = count(info.count_before);
            let after = count(info.count_after);
            let returned = info
                .participants
                .as_ref()
                .map_or_else(|| "Unavailable".into(), |nodes| nodes.len().to_string());
            ui.label(format!(
                "Count before {before} · Returned {returned} · Count after {after}"
            ));
            let warning = match info.count_status() {
                DlrParticipantCountStatus::CountsMatch => None,
                DlrParticipantCountStatus::CountsChanged => {
                    Some("Reported count changed during the read.")
                }
                DlrParticipantCountStatus::CountMismatch => {
                    Some("Returned list length differs from the reported count.")
                }
                DlrParticipantCountStatus::Unavailable => {
                    Some("Could not verify the participant count.")
                }
            };
            if let Some(warning) = warning {
                ui.colored_label(AMBER, warning);
            }
            if let Some(nodes) = &info.participants
                && !nodes.is_empty()
            {
                show_columns(ui, "IPv4 address", "MAC address");
                let row_height = ui.text_style_height(&egui::TextStyle::Body);
                egui::ScrollArea::vertical()
                    .id_salt("dlr-participant-addresses")
                    .max_height(190.0)
                    .show_rows(ui, row_height, nodes.len(), |ui, rows| {
                        for node in &nodes[rows] {
                            let ipv4 = node.ipv4.map_or_else(|| "—".into(), |ip| ip.to_string());
                            show_columns(ui, &ipv4, node.mac_address.as_deref().unwrap_or("—"));
                        }
                    });
            }
            ui.collapsing("Participant replies", |ui| {
                for (index, result) in info.batch.results.iter().enumerate() {
                    ui.push_id(index, |ui| batch::show_result(ui, result));
                }
            });
        });
}

fn show_columns(ui: &mut Ui, ipv4: &str, mac: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 0.0], egui::Label::new(ipv4));
        ui.label(mac);
    });
}
