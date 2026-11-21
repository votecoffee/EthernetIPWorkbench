use super::{
    AMBER, AttributeListReport, Explorer, Ipv4Addr, ReadPath, Response, Ui, egui, explicit,
};

impl Explorer {
    pub(super) fn show_attribute_list(
        &mut self,
        ui: &mut Ui,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) {
        ui.collapsing("Attribute list read", |ui| {
            ui.label("Use the class, instance, and optional route above.");
            ui.horizontal_wrapped(|ui| {
                ui.label("Attribute IDs and reply byte counts");
                ui.add(
                    egui::TextEdit::singleline(&mut self.attribute_list_spec)
                        .desired_width(220.0)
                        .hint_text("1:2 2:2 4:2"),
                );
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Read attribute list"))
                    .on_hover_text("Enter each attribute ID followed by a colon and its documented reply length in bytes.")
                    .clicked()
                {
                    let request = (|| {
                        let (adapter, local, class, route) =
                            self.inspection_request(adapter, local, &self.class)?;
                        let instance = explicit::parse_id(&self.instance)?;
                        let fields = explicit::parse_attribute_widths(&self.attribute_list_spec)?;
                        Ok::<_, String>((adapter, local, class, instance, fields, route))
                    })();
                    self.attribute_list = None;
                    match request {
                        Ok((adapter, local, class, instance, fields, route)) => self.start(move || {
                            explicit::read_attribute_list(adapter, local, class, instance, fields, route)
                                .map(Response::AttributeList)
                        }),
                        Err(error) => self.notice = Some(error),
                    }
                }
            });
            if let Some(report) = &self.attribute_list {
                show_report(ui, report);
            }
        });
    }
}

fn show_report(ui: &mut Ui, report: &AttributeListReport) {
    ui.small(format!(
        "{} · Local NIC {} · Class 0x{:X} · Instance 0x{:X}",
        report.adapter_or_bridge, report.local_ip, report.path.class, report.path.instance
    ));
    if let Some(route) = &report.route {
        ui.small(format!("Route {route}"));
    }
    let reply = match &report.reply {
        Ok(reply) => reply,
        Err(error) => {
            ui.colored_label(AMBER, error.to_string());
            return;
        }
    };
    if reply.general_status != 0 {
        ui.colored_label(
            AMBER,
            format!("Attribute list status 0x{:02X}", reply.general_status),
        );
    }
    if !reply.extended_status.is_empty() {
        ui.small(format!("Extended status {:04X?}", reply.extended_status));
    }
    match &reply.entries {
        Ok(Some(entries)) => {
            for entry in entries {
                if let Some(bytes) = &entry.data {
                    let path = ReadPath {
                        attribute: Some(entry.attribute.into()),
                        ..report.path
                    };
                    ui.label(format!("Attribute 0x{:X} · Reply", entry.attribute));
                    super::show_reply_payload(ui, bytes, &path.decode_reply(bytes, None));
                } else {
                    ui.colored_label(
                        AMBER,
                        format!(
                            "Attribute 0x{:X} · Status 0x{:02X}",
                            entry.attribute, entry.status
                        ),
                    );
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            ui.colored_label(AMBER, error);
        }
    }
    if !reply.body.is_empty() {
        ui.collapsing("Attribute list reply bytes", |ui| {
            super::show_reply_payload(ui, &reply.body, &Ok(None))
        });
    }
}
