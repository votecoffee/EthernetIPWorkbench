use super::*;
use assembly_scanner::explicit::{SavedReadOutcome, SavedReadResult};

fn mode_label(mode: SavedReadMode) -> &'static str {
    match mode {
        SavedReadMode::Sequential => "Unconnected sequential",
        SavedReadMode::Packet => "Unconnected Multiple Service Packet",
        SavedReadMode::Connected => "Class 3 sequential",
        SavedReadMode::ConnectedPacket => "Class 3 Multiple Service Packet",
    }
}

pub(super) fn controls(
    ui: &mut Ui,
    mode: &mut SavedReadMode,
    class3_size: &mut Class3Size,
    reads: &[SavedRead],
    ready: bool,
) -> bool {
    let mut run = false;
    let mut route_error = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Read mode");
        ui.add_enabled_ui(ready, |ui| {
            egui::ComboBox::from_id_salt("saved-read-mode")
                .selected_text(mode_label(*mode))
                .show_ui(ui, |ui| {
                    for choice in [
                        SavedReadMode::Sequential,
                        SavedReadMode::Packet,
                        SavedReadMode::Connected,
                        SavedReadMode::ConnectedPacket,
                    ] {
                        ui.selectable_value(mode, choice, mode_label(choice));
                    }
                });
        });
        route_error = mode.validate_routes(reads).err();
        run = ui
            .add_enabled(
                ready && !reads.is_empty() && route_error.is_none(),
                egui::Button::new("Read all saved"),
            )
            .clicked();
    });
    if let Some(error) = route_error {
        ui.colored_label(
            AMBER,
            format!("{error}. Choose Unconnected sequential to run this list."),
        );
    }
    if matches!(
        mode,
        SavedReadMode::Connected | SavedReadMode::ConnectedPacket
    ) {
        ui.add_enabled_ui(ready, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Class 3 connection size");
                let mut bytes = class3_size.bytes();
                if ui
                    .add(
                        egui::DragValue::new(&mut bytes)
                            .range(Class3Size::RANGE)
                            .suffix(" bytes"),
                    )
                    .on_hover_text("Includes the two-byte sequence in each direction.")
                    .changed()
                    && let Ok(size) = Class3Size::try_from(bytes)
                {
                    *class3_size = size;
                }
                ui.small(class3_open_name(*class3_size));
            });
        });
    }
    run
}

fn class3_open_name(size: Class3Size) -> &'static str {
    if size.uses_large_forward_open() {
        "Large Forward_Open"
    } else {
        "Forward_Open"
    }
}

pub(super) fn show(
    ui: &mut Ui,
    report: &SavedReadReport,
    saved_report: Option<&Path>,
    can_save: bool,
) -> bool {
    ui.collapsing("Saved read results", |ui| {
        let batch = &report.batch;
        ui.small(format!(
            "{} · {} · Local NIC {}",
            mode_label(report.mode),
            batch.adapter_or_bridge,
            batch.local_ip
        ));
        if let Some(route) = &report.connection_route {
            ui.small(format!("Connection route · {route}"));
        }
        if let Some(size) = report.class3_size {
            ui.small(format!(
                "{} · {} bytes requested",
                class3_open_name(size),
                size.bytes()
            ));
        }
        if let Some(error) = &report.packet_error {
            ui.colored_label(AMBER, format!("Multiple Service Packet failed · {error}"));
        }
        if let Some(close) = &report.forward_close {
            let message = format!("Forward_Close · {close}");
            if close.is_confirmed() {
                ui.small(message);
            } else {
                ui.colored_label(AMBER, message);
            }
        }
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

pub(super) fn save_report(report: &SavedReadReport, directory: &Path) -> Result<PathBuf, String> {
    if directory.as_os_str().is_empty() {
        return Err("Set an output directory in Logging".into());
    }
    let timestamp = chrono::Utc::now()
        .format("saved-reads-%Y%m%dT%H%M%S%.9fZ.json")
        .to_string();
    let path = directory.join(timestamp);
    let mut text = report.report_json()?;
    text.push('\n');
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    settings_file::write_new(&path, &text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

pub(super) fn show_result(ui: &mut Ui, result: &SavedReadResult) {
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
