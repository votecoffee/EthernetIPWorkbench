use super::{
    AMBER, DlrParticipantReport, Explorer, Ipv4Addr, Response, Ui, batch, egui, explicit,
    show_reply_payload,
};
use explicit::{DlrParticipantCountStatus, DlrParticipantPage};

impl Explorer {
    pub(super) fn dlr_participants_action(
        &mut self,
        ui: &mut Ui,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) {
        ui.checkbox(
            &mut self.dlr_participants_paged,
            "Read participants in pages",
        )
        .on_hover_text(
            "Use Get_Member for supervisors that limit the size of a single list reply.",
        );
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
        let read = if self.dlr_participants_paged {
            explicit::read_dlr_participants_paged
        } else {
            explicit::read_dlr_participants
        };
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                read(adapter, local, instance, route).map(Response::DlrParticipants)
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
            if let Some(pages) = &info.pages {
                ui.small(format!("Get_Member pages requested · {}", pages.len()));
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
            if let Some(issue) = &info.list_issue {
                ui.colored_label(AMBER, issue);
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
                if let Some(pages) = &info.pages {
                    for page in pages {
                        show_page(ui, page);
                    }
                }
            });
        });
}

fn show_page(ui: &mut Ui, page: &DlrParticipantPage) {
    let last = page.first_member + page.requested_count - 1;
    ui.collapsing(
        format!("Get_Member {}–{last}", page.first_member),
        |ui| match &page.reply {
            Ok(bytes) => {
                let decoded = page
                    .returned_count()
                    .map(|count| count.map(|count| format!("{count} participant records")));
                show_reply_payload(ui, bytes, &decoded);
            }
            Err(error) => {
                ui.colored_label(AMBER, error.to_string());
            }
        },
    );
}

fn show_columns(ui: &mut Ui, ipv4: &str, mac: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 0.0], egui::Label::new(ipv4));
        ui.label(mac);
    });
}
