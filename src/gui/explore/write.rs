use super::*;
use assembly_scanner::explicit::{ReadbackOutcome, SavedWriteReceipt, WriteDraft};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::json;

pub(super) struct Draft {
    plan: WriteDraft,
    bytes: String,
    typed: bool,
    typed_value: String,
    format: Selection,
    pub(super) identity: Option<Identity>,
    connected: bool,
    class3_size: Class3Size,
    current_input: Option<String>,
    pub(super) inspection: Option<WriteInspection>,
    pub(super) saved_file: Option<PathBuf>,
}

pub(super) struct Receipt {
    draft: WriteDraft,
    report: WriteReport,
    completed_utc: DateTime<Utc>,
}

impl Receipt {
    fn completed_utc(&self) -> String {
        self.completed_utc
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn save(&self, directory: &Path) -> Result<PathBuf, String> {
        let mut document = self.draft.report_document();
        document["result"] = json!(self.report);
        document["completed_utc"] = json!(self.completed_utc());
        let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
        save_json_report(directory, "attribute-write", &text)
    }
}

enum Action {
    Verify,
    InspectCurrent,
    Save,
    Send(IdentityPin),
    Cancel,
}

enum ReceiptAction {
    Save,
    CheckCurrent,
    SaveCheck(Arc<WriteCurrentCheck>),
}

impl Draft {
    fn new(plan: WriteDraft, class3_default: Class3Size) -> Self {
        Self {
            bytes: model::hex(&plan.request.value),
            typed: false,
            typed_value: String::new(),
            format: Selection::default(),
            identity: None,
            inspection: None,
            connected: plan.class3_size.is_some(),
            class3_size: plan.class3_size.unwrap_or(class3_default),
            current_input: plan.expected_current.as_deref().map(model::hex),
            plan,
            saved_file: None,
        }
    }

    fn encode_value(&self) -> Result<Vec<u8>, String> {
        if self.typed {
            self.format.encode_value(&self.typed_value)
        } else {
            model::parse_hex(&self.bytes)
        }
    }

    fn staged_value(&self) -> Result<WriteDraft, String> {
        let mut plan = self.plan.clone();
        plan.request.value = self.encode_value()?;
        plan.class3_size = self.connected.then_some(self.class3_size);
        Ok(plan)
    }

    fn compile(&self) -> Result<WriteDraft, String> {
        let mut plan = self.staged_value()?;
        plan.expected_current = self
            .current_input
            .as_deref()
            .map(model::parse_hex)
            .transpose()
            .map_err(|error| format!("Expected current bytes · {error}"))?;
        Ok(plan)
    }
}

impl Explorer {
    pub(super) fn stage_attribute_write(
        &mut self,
        adapter: &str,
        local: Option<Ipv4Addr>,
        class3_size: Class3Size,
    ) {
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
            Ok::<_, String>(Draft::new(
                WriteDraft {
                    request,
                    class3_size: None,
                    expected_current: None,
                    readback: false,
                },
                class3_size,
            ))
        };
        match prepare() {
            Ok(draft) => {
                self.notice = None;
                self.write_draft = Some(draft);
            }
            Err(error) => self.notice = Some(error),
        }
    }

    pub(super) fn show_open_write_draft(&mut self, ui: &mut Ui, class3_default: Class3Size) {
        let idle = !self.busy() && self.write_draft.is_none();
        let open = ui
            .collapsing("Write drafts", |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    file_control(
                        ui,
                        &mut self.write_draft_path,
                        "Path to a write draft TOML",
                        "Open write draft",
                    )
                })
                .inner
            })
            .body_returned
            .unwrap_or(false);
        if open {
            let path = PathBuf::from(self.write_draft_path.trim());
            self.start(move || {
                let draft = Draft::new(WriteDraft::load(&path)?, class3_default);
                Ok(Response::OpenedWriteDraft(Box::new(draft)))
            });
        }
    }

    pub(super) fn show_attribute_write(&mut self, ui: &mut Ui, directory: &str) {
        if let Some(receipt) = &self.write_result
            && let Some(action) = show_result(
                ui,
                receipt,
                self.saved_write_receipt.as_deref(),
                self.write_current_check.as_ref(),
                !self.busy() && self.write_draft.is_none(),
            )
        {
            let receipt = Arc::clone(receipt);
            match action {
                ReceiptAction::Save => {
                    let directory = PathBuf::from(directory);
                    self.start(move || receipt.save(&directory).map(Response::SavedWriteReceipt));
                }
                ReceiptAction::CheckCurrent => {
                    self.write_current_check = None;
                    self.start(move || {
                        let check = receipt
                            .draft
                            .check_current(receipt.report.expected_identity)?;
                        Ok(Response::CheckedWriteReceipt(Arc::new(check)))
                    });
                }
                ReceiptAction::SaveCheck(check) => {
                    let directory = PathBuf::from(directory);
                    self.start(move || {
                        let original = SavedWriteReceipt::from_report(
                            &receipt.draft,
                            &receipt.report,
                            receipt.completed_utc(),
                        );
                        receipt::save_check(&directory, &original, &check, receipt::Source::Current)
                    });
                }
            }
        }
        let ready = !self.busy();
        let Some(draft) = &mut self.write_draft else {
            return;
        };
        let response =
            egui::Modal::new(egui::Id::new("cip-attribute-write")).show(ui.ctx(), |ui| {
                ui.heading("Review attribute write");
                show_address(ui, &draft.plan.request);
                let inspect = ui.add_enabled_ui(ready, |ui| {
                    edit_transport(ui, draft);
                    edit_value(ui, draft);
                    edit_current_value(ui, draft)
                }).inner;
                ui.add_enabled(ready, egui::Checkbox::new(&mut draft.plan.readback, "Read back after write"))
                    .on_hover_text("Compare the returned bytes from this attribute after the target acknowledges the write.");
                if let Some(identity) = &draft.identity {
                    show_identity(ui, identity);
                    if let Some(expected) = &draft.plan.expected_current {
                        show_hex(ui, "Expected current bytes", &model::hex(expected));
                    }
                    ui.label(format!("{} bytes to send", draft.plan.request.value.len()));
                    show_reply_payload(ui, &draft.plan.request.value, &Ok(None));
                }
                let save = ui.collapsing("Save draft file", |ui| {
                    if let Some(path) = &draft.saved_file {
                        ui.small(format!("Saved {}", path.display()));
                    }
                    ui.add_enabled_ui(ready, |ui| {
                        file_control(ui, &mut self.write_draft_path, "Path to a write draft TOML", "Save write draft")
                    }).inner
                }).body_returned.unwrap_or(false);
                if let Some(notice) = &self.notice {
                    ui.colored_label(AMBER, notice);
                }
                if !ready {
                    ui.spinner();
                }
                if save {
                    return Some(Action::Save);
                }
                if inspect {
                    return Some(Action::InspectCurrent);
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
                match draft.compile() {
                    Ok(plan) => {
                        draft.plan = plan.clone();
                        self.start(move || {
                            plan.read_target_identity().map(Response::WriteIdentity)
                        });
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
            Some(Action::InspectCurrent) => {
                draft.inspection = None;
                draft.identity = None;
                match draft.staged_value() {
                    Ok(plan) => self.start(move || {
                        plan.inspect_current()
                            .map(|report| Response::WriteInspection(Box::new(report)))
                    }),
                    Err(error) => self.notice = Some(error),
                }
            }
            Some(Action::Save) => {
                let path = PathBuf::from(self.write_draft_path.trim());
                match draft.compile() {
                    Ok(plan) => self.start(move || {
                        plan.save_new(&path)?;
                        Ok(Response::SavedWriteDraft(path))
                    }),
                    Err(error) => self.notice = Some(error),
                }
            }
            Some(Action::Send(pin)) => {
                let plan = draft.plan.clone();
                self.write_draft = None;
                self.start(move || {
                    let report = plan.apply(pin)?;
                    Ok(Response::WriteResult(Arc::new(Receipt {
                        draft: plan,
                        report,
                        completed_utc: Utc::now(),
                    })))
                });
            }
            _ => {}
        }
    }
}

pub(super) fn file_control(ui: &mut Ui, path: &mut String, hint: &str, button: &str) -> bool {
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(280.0)
                .hint_text(hint),
        );
        ui.add_enabled(!path.trim().is_empty(), egui::Button::new(button))
            .clicked()
    })
    .inner
}

fn edit_transport(ui: &mut Ui, draft: &mut Draft) {
    if ui.checkbox(&mut draft.connected, "Use Class 3").changed() {
        draft.identity = None;
        draft.inspection = None;
    }
    if draft.connected {
        ui.horizontal(|ui| {
            ui.label("Connection size");
            let mut bytes = draft.class3_size.bytes();
            if ui
                .add(
                    egui::DragValue::new(&mut bytes)
                        .range(Class3Size::RANGE)
                        .suffix(" bytes"),
                )
                .changed()
                && let Ok(size) = Class3Size::try_from(bytes)
            {
                draft.class3_size = size;
                draft.identity = None;
                draft.inspection = None;
            }
        });
    }
}

fn edit_current_value(ui: &mut Ui, draft: &mut Draft) -> bool {
    let mut required = draft.current_input.is_some();
    if ui
        .checkbox(&mut required, "Require current value")
        .changed()
    {
        draft.current_input = required.then(String::new);
        draft.identity = None;
        draft.inspection = None;
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
    if !required {
        return false;
    }
    let read = ui.button("Read current from target").clicked();
    if let Some(report) = &draft.inspection {
        show_identity(ui, &report.identity);
        if let Some(close) = &report.forward_close {
            let label = format!("Inspection Forward_Close · {close}");
            if close.is_confirmed() {
                ui.small(label);
            } else {
                ui.colored_label(AMBER, label);
            }
        }
        match &report.current {
            Ok(bytes) => {
                let value = model::hex(bytes);
                show_hex(ui, "Current attribute bytes", &value);
                if ui
                    .add_enabled(
                        report.is_success(),
                        egui::Button::new("Use as expected value"),
                    )
                    .clicked()
                {
                    draft.current_input = Some(value);
                    draft.identity = None;
                    draft.inspection = None;
                }
            }
            Err(error) => {
                ui.colored_label(AMBER, error.to_string());
            }
        }
    }
    read
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
        egui::ScrollArea::vertical()
            .id_salt("edit-write-bytes")
            .max_height(100.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut draft.bytes)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(360.0)
                        .desired_rows(2)
                        .hint_text("Space-separated hex bytes"),
                )
                .changed()
            })
            .inner
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

pub(super) fn show_address(ui: &mut Ui, request: &AttributeWrite) {
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

pub(super) fn show_reviewed_identity(ui: &mut Ui, identity: IdentityPin) {
    let IdentityPin {
        key: (vendor, device_type, code),
        serial,
    } = identity;
    ui.label(format!(
        "Reviewed identity {vendor}:{device_type}:{code}:{serial}"
    ));
}

fn show_result(
    ui: &mut Ui,
    receipt: &Receipt,
    saved: Option<&Path>,
    current: Option<&receipt::FollowUp>,
    ready: bool,
) -> Option<ReceiptAction> {
    let report = &receipt.report;
    let request = &receipt.draft.request;
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
    if let Some(close) = &report.forward_close {
        let detail = format!("Forward_Close · {close}");
        if close.is_confirmed() {
            ui.label(detail);
        } else {
            ui.colored_label(AMBER, detail);
        }
    }
    ui.collapsing("Attribute write result", |ui| {
        let mut action = ui
            .horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(ready, egui::Button::new("Save write receipt JSON"))
                    .on_hover_text("Save the result to the output directory in Logging.")
                    .clicked()
                {
                    return Some(ReceiptAction::Save);
                }
                ui.add_enabled(ready, egui::Button::new("Check current value"))
                    .on_hover_text(
                        "Read the same device and compare its current bytes with the staged value.",
                    )
                    .clicked()
                    .then_some(ReceiptAction::CheckCurrent)
            })
            .inner;
        if let Some(path) = saved {
            ui.small(format!("Saved {}", path.display()));
        }
        if let Some(check) = current
            && let Some(save) = check.show(ui, ready, "Save current check JSON")
        {
            action = Some(ReceiptAction::SaveCheck(save));
        }
        ui.small(format!("Completed {}", receipt.completed_utc()));
        show_address(ui, request);
        if let Some(size) = receipt.draft.class3_size {
            ui.label(format!("Class 3 · {} bytes requested", size.bytes()));
        }
        show_reviewed_identity(ui, report.expected_identity);
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
        action
    })
    .body_returned
    .flatten()
}

pub(super) fn show_current_check(ui: &mut Ui, check: &WriteCurrentCheck) {
    ui.small(format!("Checked {}", check.checked_utc));
    show_identity(ui, &check.observed_identity);
    match &check.comparison {
        ReadbackOutcome::Matched { .. } => {
            ui.label("Current value matches the staged bytes");
        }
        ReadbackOutcome::Mismatch { .. } => {
            ui.colored_label(AMBER, "Current value differs from the staged bytes");
        }
        ReadbackOutcome::Skipped => {
            ui.colored_label(
                AMBER,
                "Current value check skipped · Device identity differs",
            );
        }
        outcome => {
            ui.colored_label(AMBER, outcome.to_string());
        }
    }
    if let Some(data) = check.comparison.data_hex() {
        show_hex(ui, "Current bytes after write attempt", data);
    }
    if let Some(close) = &check.forward_close {
        let label = format!("Current check Forward_Close · {close}");
        if close.is_confirmed() {
            ui.small(label);
        } else {
            ui.colored_label(AMBER, label);
        }
    }
}

fn show_hex(ui: &mut Ui, label: &str, data_hex: &str) {
    ui.label(label);
    egui::ScrollArea::vertical()
        .id_salt(label)
        .max_height(120.0)
        .show(ui, |ui| ui.monospace(data_hex));
}
