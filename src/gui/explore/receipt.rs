use super::{write, *};
use assembly_scanner::explicit::{SavedWriteOutcome, SavedWriteReceipt};

#[derive(Default)]
pub(super) struct Panel {
    path: String,
    pub(super) opened: Option<Opened>,
    pub(super) check: Option<Box<WriteCurrentCheck>>,
}

pub(super) struct Opened {
    path: PathBuf,
    receipt: Arc<SavedWriteReceipt>,
}

enum Action {
    Open,
    Check,
}

impl Explorer {
    pub(super) fn show_write_receipts(&mut self, ui: &mut Ui) {
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
                        Ok(Response::CheckedReceiptFile(Box::new(check)))
                    });
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
        if let Some(result) = &self.check {
            write::show_current_check(ui, result);
        }
        check.then_some(Action::Check)
    }
}
