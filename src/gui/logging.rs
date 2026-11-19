use super::*;

impl Device {
    pub(super) fn logging(&mut self, ui: &mut Ui) {
        title(ui, "CSV logging");
        ui.add_enabled_ui(!self.capturing, |ui| {
            egui::Grid::new("capture").num_columns(2).show(ui, |ui| {
                ui.label("Output directory");
                ui.text_edit_singleline(&mut self.preferences.directory);
                ui.end_row();
                ui.label("Stop after seconds (0 = manual)");
                ui.add(
                    egui::DragValue::new(&mut self.preferences.gui_capture_seconds)
                        .range(0..=capture::MAX_CAPTURE_SECONDS)
                        .clamp_existing_to_range(false),
                );
                ui.end_row();
            });
            ui.collapsing("Buffer and file rotation", |ui| {
                egui::Grid::new("capture-advanced")
                    .num_columns(2)
                    .show(ui, |ui| {
                        number(ui, "Buffer samples", &mut self.preferences.capacity);
                        number(
                            ui,
                            "Rotate after bytes (0 = off)",
                            &mut self.preferences.rotate_bytes,
                        );
                        number(
                            ui,
                            "Rotate after seconds (0 = off)",
                            &mut self.preferences.rotate_seconds,
                        );
                    });
            });
        });
        ui.horizontal_wrapped(|ui| {
            let fields = self.settings.csv_fields();
            ui.label(fields.map_or_else(
                |error| error,
                |fields| {
                    format!(
                        "Logged values · {}",
                        fields
                            .iter()
                            .map(|field| field.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ));
            if ui.button("Choose logged values").clicked() {
                self.tab = Tab::Mappings;
            }
        });
        if self.capturing {
            if ui.button("Stop capture and flush").clicked() {
                self.stop_capture();
            }
        } else if ui.button("Start CSV capture").clicked() {
            self.start_capture();
        }
        if let Some(capture) = &self.capture {
            let s = &capture.sink;
            if self.capturing && s.duration_seconds().is_some() {
                if let Some(deadline) = s.deadline() {
                    ui.small(format!(
                        "{:.2} s until capture stops",
                        deadline
                            .saturating_duration_since(Instant::now())
                            .as_secs_f64()
                    ));
                } else {
                    ui.small("Timer starts with the first input packet");
                }
            } else if self.capture_run.duration_reached {
                ui.small("Capture time limit reached");
            }
            let seen = s.seen.load(Relaxed);
            let unchanged = s.unchanged.load(Relaxed);
            let written = s.written.load(Relaxed);
            let overrun = s.overrun.load(Relaxed);
            ui.label(format!(
                "{}  ·  Seen {seen}  ·  Unchanged {unchanged}  ·  Written {written}",
                if self.capturing { "Armed" } else { "Stopped" }
            ));
            ui.colored_label(
                if overrun > 0 { RED } else { Color32::GRAY },
                format!("Local buffer overruns {overrun}"),
            );
            if let Some(e) = &*s.error.lock().unwrap() {
                ui.colored_label(
                    RED,
                    format!(
                        "Disk error — {e}  ·  Unconfirmed rows {}",
                        seen.saturating_sub(written + overrun + unchanged)
                    ),
                );
            }
            ui.small(s.prefix.display().to_string());
        }
        if self.capture.is_some()
            && ui
                .add_enabled(
                    !self.capturing && !self.link_busy(),
                    egui::Button::new("Save run report"),
                )
                .on_disabled_hover_text("Disconnect and stop capture before saving the report")
                .clicked()
        {
            match self.save_run_report() {
                Ok(path) => {
                    self.error = None;
                    self.notice = Some(format!("Saved run report to {}", path.display()));
                }
                Err(error) => {
                    self.notice = None;
                    self.error = Some(error);
                }
            }
        }
    }
}
