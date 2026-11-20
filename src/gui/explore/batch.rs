use super::*;
use assembly_scanner::explicit::{SavedReadOutcome, SavedReadResult};

pub(super) fn show(ui: &mut Ui, batch: &SavedReadBatch) {
    ui.collapsing("Saved read results", |ui| {
        ui.small(format!(
            "{} · Local NIC {}",
            batch.adapter_or_bridge, batch.local_ip
        ));
        egui::ScrollArea::vertical()
            .id_salt("batch-replies")
            .max_height(260.0)
            .show(ui, |ui| {
                for (index, result) in batch.results.iter().enumerate() {
                    ui.push_id(index, |ui| show_result(ui, result));
                }
            });
    });
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
