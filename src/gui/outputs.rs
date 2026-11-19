use super::*;

impl Device {
    pub(super) fn outputs(&mut self, ui: &mut Ui) {
        title(ui, "Stage and apply outputs");
        if self.settings.input_only {
            ui.label(format!(
                "{} sends a heartbeat with no output payload.",
                self.settings.role().label()
            ));
            if ui.button("Review device connection settings").clicked() {
                self.tab = Tab::Device;
            }
            return;
        }
        ui.label("Stage a buffer, review its values, then authorize Apply staged outputs for an open owner connection.");
        ui.label("Staged output hex bytes");
        if ui
            .add_enabled(
                !self.settings.input_only,
                egui::TextEdit::multiline(&mut self.settings.output_hex)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .code_editor(),
            )
            .changed()
        {
            self.writes_confirmed = false;
        }
        self.typed_outputs(ui);
        let variable = self.settings.output_min_size.is_some();
        let staged_length = if let Some(min) = self.settings.output_min_size {
            match self.settings.output() {
                Ok(bytes) => {
                    ui.label(format!(
                        "Staged length · {} bytes · Allowed {min}–{}",
                        bytes.len(),
                        self.settings.output_size
                    ));
                    true
                }
                Err(error) => {
                    ui.colored_label(RED, error);
                    false
                }
            }
        } else {
            true
        };
        if let Some(c) = &self.connection {
            if !self.settings.input_only {
                ui.checkbox(
                    &mut self.writes_confirmed,
                    "These staged output bytes are authorized",
                );
            }
            if ui
                .add_enabled(
                    !self.settings.input_only
                        && self.status.state == "Receiving"
                        && staged_length
                        && self.writes_confirmed,
                    egui::Button::new("Apply staged outputs"),
                )
                .clicked()
            {
                self.writes_confirmed = false;
                match self
                    .settings
                    .output()
                    .and_then(|bytes| c.apply_output(bytes))
                {
                    Ok(()) => {
                        self.notice = Some("Output buffer applied to the cyclic producer".into());
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            let applied = c.output_bytes();
            if variable || !applied.is_empty() {
                ui.label(format!(
                    "Active output buffer · {} bytes · {}",
                    applied.len(),
                    model::hex(&applied)
                ));
            }
        }
    }

    fn typed_outputs(&mut self, ui: &mut Ui) {
        if self.settings.input_only || self.settings.output_fields.is_empty() {
            return;
        }
        self.output_entries
            .resize_with(self.settings.output_fields.len(), String::new);
        ui.collapsing("Stage typed output values", |ui| {
            ui.small("Stage edits the hex buffer. Use Apply staged outputs to send it on an open connection.");
            let mut staged = self.settings.output();
            if staged.is_err() {
                ui.label("Enter a hex buffer within the allowed length or initialize it to the maximum size.");
                if self.settings.output_size as usize <= model::MAX_LARGE_ASSEMBLY_BYTES
                    && ui.button("Stage zero-filled buffer").clicked()
                {
                    let bytes = vec![0; self.settings.output_size as usize];
                    self.settings.output_hex = model::hex(&bytes);
                    self.writes_confirmed = false;
                    staged = Ok(bytes);
                }
            }
            let Ok(mut bytes) = staged else { return; };
            egui::Grid::new("typed-output").striped(true).show(ui, |ui| {
                ui.strong("Output"); ui.strong("Staged"); ui.strong("New value"); ui.label("");
                ui.end_row();
                for index in 0..self.settings.output_fields.len() {
                    let field = &self.settings.output_fields[index];
                    ui.label(&field.name);
                    let decoded = field.selection.decode(&bytes);
                    let fits = decoded.is_some();
                    ui.monospace(decoded.map_or_else(|| "Unavailable at staged length".into(), |v| v.to_string()));
                    ui.push_id(index, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.output_entries[index]).desired_width(100.0));
                        if ui.add_enabled(fits, egui::Button::new("Stage")).clicked() {
                            let name = self.settings.output_fields[index].name.clone();
                            match self.settings.stage_output(index, &self.output_entries[index]) {
                                Ok(()) => {
                                    bytes = self.settings.output().expect("validated staged buffer");
                                    self.writes_confirmed = false;
                                    self.notice = Some(format!("Staged {name}"));
                                    self.error = None;
                                }
                                Err(error) => self.error = Some(format!("{name}: {error}")),
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
}
