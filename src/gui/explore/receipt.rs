use super::{write, *};
use assembly_scanner::explicit::{SavedWriteOutcome, SavedWriteReceipt};

#[derive(Default)]
pub(super) struct Panel {
    path: String,
    pub(super) opened: Option<Opened>,
    pub(super) check: Option<FollowUp>,
}

pub(super) struct Opened {
    path: PathBuf,
    receipt: Arc<SavedWriteReceipt>,
}

pub(super) enum Source {
    Current,
    Reopened,
}

pub(super) struct FollowUp {
    check: Arc<WriteCurrentCheck>,
    pub(super) saved_path: Option<PathBuf>,
}

impl FollowUp {
    pub(super) fn new(check: Arc<WriteCurrentCheck>) -> Self {
        Self {
            check,
            saved_path: None,
        }
    }

    pub(super) fn show(
        &self,
        ui: &mut Ui,
        ready: bool,
        label: &str,
    ) -> Option<Arc<WriteCurrentCheck>> {
        write::show_current_check(ui, &self.check);
        let save = ui
            .add_enabled(ready, egui::Button::new(label))
            .on_hover_text("Save the follow-up to the output directory in Logging.")
            .clicked();
        if let Some(path) = &self.saved_path {
            ui.small(format!("Saved current check {}", path.display()));
        }
        save.then(|| Arc::clone(&self.check))
    }
}

pub(super) fn save_check(
    directory: &Path,
    receipt: &SavedWriteReceipt,
    result: &WriteCurrentCheck,
    source: Source,
) -> Result<Response, String> {
    let text = serde_json::to_string_pretty(&receipt.follow_up_document(result))
        .map_err(|error| error.to_string())?;
    save_json_report(directory, "write-current-check", &text)
        .map(|path| Response::SavedWriteCheck(source, path))
}

enum Action {
    Open,
    Check,
    SaveCheck(Arc<WriteCurrentCheck>),
}

impl Explorer {
    pub(super) fn show_write_receipts(&mut self, ui: &mut Ui, directory: &str) {
        let ready = !self.busy() && self.write_draft.is_none();
        let panel = &mut self.receipt_files;
        let action = ui
            .collapsing("Saved write receipts", |ui| panel.show(ui, ready))
            .body_returned
            .flatten();

        match action {
            Some(Action::Open) => {
                let path = PathBuf::from(panel.path.trim());
                panel.opened = None;
                panel.check = None;
                self.start(move || {
                    let receipt = Arc::new(SavedWriteReceipt::load(&path)?);
                    Ok(Response::OpenedReceiptFile(Opened { path, receipt }))
                });
            }
            Some(Action::Check) => {
                if let Some(opened) = &panel.opened {
                    let receipt = Arc::clone(&opened.receipt);
                    panel.check = None;
                    self.start(move || {
                        let check = receipt.check_current()?;
                        Ok(Response::CheckedReceiptFile(Arc::new(check)))
                    });
                }
            }
            Some(Action::SaveCheck(check)) => {
                if let Some(opened) = &panel.opened {
                    let receipt = Arc::clone(&opened.receipt);
                    let directory = PathBuf::from(directory);
                    self.start(move || save_check(&directory, &receipt, &check, Source::Reopened));
                }
            }
            None => {}
        }
    }
}

impl Panel {
    fn show(&mut self, ui: &mut Ui, ready: bool) -> Option<Action> {
        if ui
            .add_enabled_ui(ready, |ui| {
                write::file_control(
                    ui,
                    &mut self.path,
                    "Path to a write receipt JSON",
                    "Open write receipt",
                )
            })
            .inner
        {
            return Some(Action::Open);
        }
        let opened = self.opened.as_ref()?;
        let receipt = &opened.receipt;
        ui.small(format!("Opened {}", opened.path.display()));
        let outcome = receipt.outcome();
        let label = format!("Recorded write outcome · {outcome}");
        if outcome == SavedWriteOutcome::Acknowledged {
            ui.label(label);
        } else {
            ui.colored_label(AMBER, label);
        }
        if let Some(time) = receipt.completed_utc() {
            ui.small(format!("Completed {time}"));
        }
        let request = receipt.request();
        write::show_address(ui, request);
        if let Some(size) = receipt.class3_size() {
            ui.label(format!("Class 3 · {} bytes requested", size.bytes()));
        } else {
            ui.label("Unconnected explicit messaging");
        }
        write::show_reviewed_identity(ui, receipt.expected_identity());
        ui.label(format!("{} staged bytes", request.value.len()));
        show_reply_payload(ui, &request.value, &Ok(None));
        let check = ui
            .add_enabled(ready, egui::Button::new("Check receipt current value"))
            .on_hover_text(
                "Read the recorded device and compare its current attribute with the staged bytes.",
            )
            .clicked();
        if let Some(result) = &self.check
            && let Some(save) = result.show(ui, ready, "Save receipt check JSON")
        {
            return Some(Action::SaveCheck(save));
        }
        check.then_some(Action::Check)
    }
}
