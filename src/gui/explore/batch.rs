use super::*;
use assembly_scanner::explicit::{SavedReadOutcome, SavedReadResult};

pub(super) fn show(
    ui: &mut Ui,
    batch: &SavedReadBatch,
    saved_report: Option<&Path>,
    can_save: bool,
) -> bool {
    ui.collapsing("Saved read results", |ui| {
        ui.small(format!(
            "{} · Local NIC {}",
            batch.adapter_or_bridge, batch.local_ip
        ));
        let save = ui
            .add_enabled(can_save, egui::Button::new("Save JSON"))
            .on_hover_text("Save the batch to the output directory in Logging.")
            .clicked();
        if let Some(path) = saved_report {
            ui.small(format!("Saved {}", path.display()));
        }
        egui::ScrollArea::vertical()
            .id_salt("batch-replies")
            .max_height(260.0)
            .show(ui, |ui| {
                for (index, result) in batch.results.iter().enumerate() {
                    ui.push_id(index, |ui| show_result(ui, result));
                }
            });
        save
    })
    .body_returned
    .unwrap_or(false)
}

pub(super) fn save_report(batch: &SavedReadBatch, directory: &Path) -> Result<PathBuf, String> {
    if directory.as_os_str().is_empty() {
        return Err("Set an output directory in Logging".into());
    }
    let timestamp = chrono::Utc::now()
        .format("saved-reads-%Y%m%dT%H%M%S%.9fZ.json")
        .to_string();
    let path = directory.join(timestamp);
    let mut text = batch.report_json()?;
    text.push('\n');
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    settings_file::write_new(&path, &text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

fn show_result(ui: &mut Ui, result: &SavedReadResult) {
    let status = match &result.outcome {
        SavedReadOutcome::Reply(_) => "Reply",
        SavedReadOutcome::Rejected(_) => "CIP rejection",
        SavedReadOutcome::ExchangeFailed(_) => "Exchange failed",
        SavedReadOutcome::NotAttempted => "Not attempted",
    };
    let request = &result.request;
    ui.collapsing(format!("{} · {status}", request.name), |ui| {
        let path = request.path;
        let attribute = path
            .attribute
            .map_or_else(|| "all".into(), |id| format!("0x{id:X}"));
        ui.small(format!(
            "Class 0x{:X} · Instance 0x{:X} · Attribute {attribute}",
            path.class, path.instance,
        ));
        if let Some(route) = &request.route {
            ui.small(format!("Route {route}"));
        }
        match &result.outcome {
            SavedReadOutcome::Reply(bytes) => {
                ui.small(format!("{} bytes", bytes.len()));
                show_reply_payload(ui, bytes, &path.decode_reply(bytes, request.selection));
            }
            SavedReadOutcome::Rejected(error) | SavedReadOutcome::ExchangeFailed(error) => {
                ui.colored_label(AMBER, error.to_string());
            }
            SavedReadOutcome::NotAttempted => {
                ui.small("The explicit session ended before this request.");
            }
        }
    });
}
