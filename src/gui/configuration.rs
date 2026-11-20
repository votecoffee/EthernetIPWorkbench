use super::*;

pub(super) fn edit(
    ui: &mut Ui,
    layout: &eds::AssemblyLayout,
    settings: &mut Settings,
    entries: &mut Vec<String>,
) -> Option<Result<String, String>> {
    let mut result = None;
    ui.collapsing("Stage typed configuration values", |ui| {
        ui.small(format!(
            "{} · Stage edits the configuration buffer for connection review.",
            layout.format
        ));
        if layout.skipped_members != 0 {
            ui.small(format!(
                "{} members have no typed editor.",
                layout.skipped_members
            ));
        }
        let bytes = settings.configuration();
        if let Err(error) = &bytes {
            ui.colored_label(AMBER, error);
            ui.small("Enter the complete configuration buffer in the hex field above.");
        }
        entries.resize_with(layout.fields.len(), String::new);
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                egui::Grid::new("typed-configuration")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Field");
                        ui.strong("Staged");
                        ui.strong("New value");
                        ui.label("");
                        ui.end_row();
                        for (index, (field, entry)) in
                            layout.fields.iter().zip(entries.iter_mut()).enumerate()
                        {
                            let selection = field.selection;
                            let bit = selection
                                .bit
                                .map_or_else(String::new, |bit| format!(", bit {bit}"));
                            ui.label(&field.name).on_hover_text(format!(
                                "Byte {}{bit} · {:?}",
                                selection.offset, selection.kind
                            ));
                            let staged =
                                bytes.as_ref().ok().and_then(|data| selection.decode(data));
                            ui.monospace(
                                staged.map_or_else(|| "—".into(), |value| value.to_string()),
                            );
                            ui.push_id(index, |ui| {
                                ui.add(egui::TextEdit::singleline(entry).desired_width(100.0));
                                if ui
                                    .add_enabled(staged.is_some(), egui::Button::new("Stage"))
                                    .clicked()
                                {
                                    result = Some(
                                        settings
                                            .stage_configuration(selection, entry)
                                            .map(|()| field.name.clone())
                                            .map_err(|error| format!("{}: {error}", field.name)),
                                    );
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    });
    result
}
