use super::{AMBER, ConnectionDataReport, Explorer, Ipv4Addr, Response, Ui, egui, explicit};

impl Explorer {
    pub(super) fn show_connection_data(
        &mut self,
        ui: &mut Ui,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) {
        ui.collapsing("Connection data", |ui| {
            ui.label("Enter a connection number or the connection serial, vendor ID, and originator serial from Forward_Open.");
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.connection_data_query)
                        .desired_width(240.0)
                        .hint_text("Number or serial:vendor:originator-serial"),
                );
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Read connection data"))
                    .on_hover_text("Query the target Connection Manager instance 1 using the optional CIP route above.")
                    .clicked()
                {
                    self.connection_data = None;
                    match (self.inspection_target(adapter, local), self.connection_data_query.parse()) {
                        (Ok((adapter, local, route)), Ok(query)) => self.start(move || {
                            explicit::read_connection_data(adapter, local, query, route)
                                .map(Response::ConnectionData)
                        }),
                        (Err(error), _) | (_, Err(error)) => self.notice = Some(error),
                    }
                }
            });
            if let Some(report) = &self.connection_data {
                show_report(ui, report);
            }
        });
    }
}

fn show_report(ui: &mut Ui, report: &ConnectionDataReport) {
    ui.small(format!(
        "{} · Local NIC {} · {}",
        report.adapter_or_bridge, report.local_ip, report.query
    ));
    if let Some(route) = &report.route {
        ui.small(format!("Route {route}"));
    }
    match report.decoded() {
        Ok(Some(data)) => {
            ui.label(format!(
                "Connection {} · State 0x{:04X}",
                data.number, data.state_code
            ));
            ui.label(explicit::ConnectionDataQuery::Triad(data.triad).to_string());
            ui.label(format!(
                "Reported originator port {} · Target port {}",
                data.originator_port, data.target_port
            ));
            for (label, direction) in [("O→T", data.o_to_t), ("T→O", data.t_to_o)] {
                ui.label(format!(
                    "{label} · Reported RPI {} µs · API {} µs · Timeout multiplier code {}",
                    direction.reported_rpi_us,
                    direction.reported_api_us,
                    direction.timeout_multiplier_code
                ));
                ui.monospace(format!(
                    "Originator ID 0x{:08X} · Target ID 0x{:08X}",
                    direction.originator_connection_id, direction.target_connection_id
                ));
            }
        }
        Ok(None) => {}
        Err(error) => {
            ui.colored_label(AMBER, error);
        }
    }
    match &report.reply {
        Ok(bytes) => {
            ui.collapsing("Connection reply bytes", |ui| {
                super::show_reply_payload(ui, bytes, &Ok(None))
            });
        }
        Err(error) => {
            ui.colored_label(AMBER, error.to_string());
        }
    }
}
