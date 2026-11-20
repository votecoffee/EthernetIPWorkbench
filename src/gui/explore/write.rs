use super::*;
use assembly_scanner::explicit::ReadbackOutcome;

pub(super) struct Draft {
    request: AttributeWrite,
    bytes: String,
    pub(super) identity: Option<Identity>,
    readback: bool,
}

enum Action {
    Verify,
    Send(IdentityPin),
    Cancel,
}

impl Explorer {
    pub(super) fn stage_attribute_write(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        let prepare = || {
            let (path, route) = self.request()?;
            let request = AttributeWrite {
                adapter_or_bridge: adapter
                    .parse()
                    .map_err(|_| "Set the adapter IPv4 address in Device")?,
                local_ip: local.ok_or("Set the local NIC IPv4 address in Device")?,
                path,
                route,
                value: Vec::new(),
            };
            request.validate()?;
            Ok::<_, String>(Draft {
                request,
                bytes: String::new(),
                identity: None,
                readback: false,
            })
        };
        match prepare() {
            Ok(draft) => {
                self.notice = None;
                self.write_draft = Some(draft);
            }
            Err(error) => self.notice = Some(error),
        }
    }

    pub(super) fn show_attribute_write(&mut self, ui: &mut Ui) {
        if let Some(result) = &self.write_result {
            show_result(ui, &result.0, &result.1);
        }
        let ready = !self.busy();
        let Some(draft) = &mut self.write_draft else {
            return;
        };
        let response =
            egui::Modal::new(egui::Id::new("cip-attribute-write")).show(ui.ctx(), |ui| {
                ui.heading("Review attribute write");
                show_address(ui, &draft.request);
                ui.label("Attribute data in hex");
                let edit = egui::TextEdit::multiline(&mut draft.bytes)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(360.0)
                    .desired_rows(2)
                    .hint_text("Space-separated hex bytes");
                if ui.add_enabled(ready, edit).changed() {
                    draft.identity = None;
                }
                ui.add_enabled(ready, egui::Checkbox::new(&mut draft.readback, "Read back after write"))
                    .on_hover_text("Compare the returned bytes from this attribute after the target acknowledges the write.");
                if let Some(identity) = &draft.identity {
                    show_identity(ui, identity);
                    ui.label(format!("{} bytes to send", draft.request.value.len()));
                    show_reply_payload(ui, &draft.request.value, &Ok(None));
                }
                if let Some(notice) = &self.notice {
                    ui.colored_label(AMBER, notice);
                }
                if !ready {
                    ui.spinner();
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(ready, egui::Button::new("Verify target identity"))
                        .clicked()
                    {
                        return Some(Action::Verify);
                    }
                    if ui
                        .add_enabled(
                            ready && draft.identity.is_some(),
                            egui::Button::new("Send reviewed write"),
                        )
                        .clicked()
                        && let Some(identity) = &draft.identity
                    {
                        return Some(Action::Send(identity.pin()));
                    }
                    ui.button("Cancel").clicked().then_some(Action::Cancel)
                })
                .inner
            });
        let should_close = response.should_close();
        let action = response.inner;
        if should_close || matches!(action, Some(Action::Cancel)) {
            self.write_draft = None;
            self.notice = None;
            return;
        }
        match action {
            Some(Action::Verify) => {
                draft.identity = None;
                match model::parse_hex(&draft.bytes) {
                    Ok(value) => {
                        draft.request.value = value;
                        let request = draft.request.clone();
                        self.start(move || {
                            request.read_target_identity().map(Response::WriteIdentity)
                        });
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
            Some(Action::Send(pin)) => {
                let request = draft.request.clone();
                let readback = draft.readback;
                self.write_draft = None;
                self.write_result = None;
                self.start(move || {
                    let report = if readback {
                        request.apply_with_readback(pin)?
                    } else {
                        request.apply(pin)?
                    };
                    Ok(Response::WriteResult(Box::new((request, report))))
                });
            }
            _ => {}
        }
    }
}

fn show_address(ui: &mut Ui, request: &AttributeWrite) {
    ui.label(format!(
        "Adapter or bridge {} · Local NIC {}",
        request.adapter_or_bridge, request.local_ip
    ));
    if let Some(route) = &request.route {
        ui.label(format!("Route {route}"));
    }
    let path = request.path;
    if let Some(attribute) = path.attribute {
        ui.label(format!(
            "Class 0x{:X} · Instance 0x{:X} · Attribute 0x{attribute:X}",
            path.class, path.instance
        ));
    }
}

fn show_identity(ui: &mut Ui, identity: &Identity) {
    ui.label(format!(
        "{} · identity {}:{}:{}:{}",
        identity.name, identity.vendor, identity.device_type, identity.code, identity.serial
    ));
}

fn show_result(ui: &mut Ui, request: &AttributeWrite, report: &WriteReport) {
    if matches!(report.outcome, WriteOutcome::Acknowledged { .. }) {
        ui.label(report.outcome.to_string());
    } else {
        ui.colored_label(AMBER, report.outcome.to_string());
    }
    if let Some(readback) = &report.readback {
        if matches!(readback, ReadbackOutcome::Matched { .. }) {
            ui.label(readback.to_string());
        } else {
            ui.colored_label(AMBER, readback.to_string());
        }
    }
    ui.collapsing("Attribute write result", |ui| {
        show_address(ui, request);
        let IdentityPin {
            key: (vendor, device_type, code),
            serial,
        } = report.expected_identity;
        ui.label(format!(
            "Reviewed identity {vendor}:{device_type}:{code}:{serial}"
        ));
        if let Some(identity) = &report.observed_identity {
            show_identity(ui, identity);
        }
        ui.label(format!("{} staged bytes", request.value.len()));
        show_reply_payload(ui, &request.value, &Ok(None));
        if let Some(
            ReadbackOutcome::Matched { data_hex } | ReadbackOutcome::Mismatch { data_hex },
        ) = &report.readback
        {
            ui.label("Readback bytes");
            egui::ScrollArea::vertical()
                .id_salt("attribute-readback")
                .max_height(120.0)
                .show(ui, |ui| ui.monospace(data_hex));
        }
    });
}
