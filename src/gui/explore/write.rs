use super::*;
use assembly_scanner::explicit::{ReadbackOutcome, WriteOptions};

pub(super) struct Draft {
    request: AttributeWrite,
    bytes: String,
    typed: bool,
    typed_value: String,
    format: Selection,
    pub(super) identity: Option<Identity>,
    readback: bool,
    current_input: Option<String>,
    expected_current: Option<Vec<u8>>,
}

enum Action {
    Verify,
    Send(IdentityPin),
    Cancel,
}

impl Draft {
    fn encode_value(&self) -> Result<Vec<u8>, String> {
        if self.typed {
            self.format.encode_value(&self.typed_value)
        } else {
            model::parse_hex(&self.bytes)
        }
    }
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
                typed: false,
                typed_value: String::new(),
                format: Selection::default(),
                identity: None,
                readback: false,
                current_input: None,
                expected_current: None,
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
                ui.add_enabled_ui(ready, |ui| {
                    edit_value(ui, draft);
                    edit_current_value(ui, draft);
                });
                ui.add_enabled(ready, egui::Checkbox::new(&mut draft.readback, "Read back after write"))
                    .on_hover_text("Compare the returned bytes from this attribute after the target acknowledges the write.");
                if let Some(identity) = &draft.identity {
                    show_identity(ui, identity);
                    if let Some(expected) = &draft.expected_current {
                        show_hex(ui, "Expected current bytes", &model::hex(expected));
                    }
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
                let values = draft.encode_value().and_then(|value| {
                    let expected = draft
                        .current_input
                        .as_deref()
                        .map(model::parse_hex)
                        .transpose()
                        .map_err(|error| format!("Expected current bytes · {error}"))?;
                    Ok((value, expected))
                });
                match values {
                    Ok((value, expected)) => {
                        draft.request.value = value;
                        draft.expected_current = expected;
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
                let expected_current = draft.expected_current.clone();
                self.write_draft = None;
                self.write_result = None;
                self.start(move || {
                    let report = request.apply_with_options(
                        pin,
                        WriteOptions {
                            expected_current: expected_current.as_deref(),
                            readback,
                        },
                    )?;
                    Ok(Response::WriteResult(Box::new((request, report))))
                });
            }
            _ => {}
        }
    }
}

fn edit_current_value(ui: &mut Ui, draft: &mut Draft) {
    let mut required = draft.current_input.is_some();
    if ui
        .checkbox(&mut required, "Require current value")
        .changed()
    {
        draft.current_input = required.then(String::new);
        draft.identity = None;
    }
    if let Some(value) = &mut draft.current_input
        && ui
            .add(
                egui::TextEdit::singleline(value)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(360.0)
                    .hint_text("Expected current value in hex"),
            )
            .changed()
    {
        draft.identity = None;
    }
}

fn edit_value(ui: &mut Ui, draft: &mut Draft) {
    let changed_mode = ui.checkbox(&mut draft.typed, "Enter typed value").changed();
    let changed_value = if draft.typed {
        let previous = draft.format;
        super::super::mapping::edit_value_format(ui, &mut draft.format);
        ui.label("Attribute value");
        let changed = ui
            .add(
                egui::TextEdit::singleline(&mut draft.typed_value)
                    .desired_width(360.0)
                    .hint_text("Typed attribute value"),
            )
            .changed();
        changed || previous != draft.format
    } else {
        ui.label("Attribute data in hex");
        ui.add(
            egui::TextEdit::multiline(&mut draft.bytes)
                .font(egui::TextStyle::Monospace)
                .desired_width(360.0)
                .desired_rows(2)
                .hint_text("Space-separated hex bytes"),
        )
        .changed()
    };
    if changed_mode || changed_value {
        draft.identity = None;
    }
    if draft.typed && draft.identity.is_none() {
        match draft.encode_value() {
            Ok(bytes) => {
                ui.label(format!("Encoded value · {} bytes", bytes.len()));
                show_reply_payload(ui, &bytes, &Ok(None));
            }
            Err(error) if !draft.typed_value.is_empty() => {
                ui.colored_label(AMBER, error);
            }
            Err(_) => {}
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
    if let Some(check) = &report.current_value {
        if matches!(check.outcome, ReadbackOutcome::Matched { .. }) {
            ui.label(check.to_string());
        } else {
            ui.colored_label(AMBER, check.to_string());
        }
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
        if let Some(check) = &report.current_value {
            show_hex(ui, "Expected current bytes", &check.expected_hex);
            if let Some(data_hex) = check.outcome.data_hex() {
                show_hex(ui, "Current bytes before write", data_hex);
            }
        }
        if let Some(data_hex) = report.readback.as_ref().and_then(ReadbackOutcome::data_hex) {
            show_hex(ui, "Readback bytes", data_hex);
        }
    });
}

fn show_hex(ui: &mut Ui, label: &str, data_hex: &str) {
    ui.label(label);
    egui::ScrollArea::vertical()
        .id_salt(label)
        .max_height(120.0)
        .show(ui, |ui| ui.monospace(data_hex));
}
