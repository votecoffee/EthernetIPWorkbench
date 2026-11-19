use super::*;

fn edit_selection(ui: &mut Ui, selection: &mut Selection) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Byte offset");
        ui.add(egui::DragValue::new(&mut selection.offset));
        egui::ComboBox::from_id_salt("field-kind")
            .selected_text(format!("{:?}", selection.kind))
            .show_ui(ui, |ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut selection.kind, kind, format!("{kind:?}"));
                }
            });
        if selection.kind == Kind::Utf8 {
            selection.bit = None;
            selection.swap = ByteSwap::None;
            ui.label("Text bytes");
            ui.add(
                egui::DragValue::new(selection.string_length.get_or_insert(1))
                    .range(1..=model::MAX_LARGE_ASSEMBLY_BYTES),
            );
        } else {
            selection.string_length = None;
            let mut bit = selection.bit.is_some();
            if ui.checkbox(&mut bit, "Single bit").changed() {
                selection.bit = bit.then_some(0);
            }
            if let Some(bit) = &mut selection.bit {
                selection.swap = ByteSwap::None;
                ui.add(egui::DragValue::new(bit).range(0..=7));
            } else if !matches!(selection.kind, Kind::Bool | Kind::U8 | Kind::I8) {
                let wide = !matches!(selection.kind, Kind::U16 | Kind::I16);
                if !wide && selection.swap == ByteSwap::WordOrder {
                    selection.swap = ByteSwap::None;
                }
                ui.checkbox(&mut selection.big_endian, "Big endian");
                egui::ComboBox::from_id_salt("field-swap")
                    .selected_text(selection.swap.label())
                    .show_ui(ui, |ui| {
                        for swap in ByteSwap::ALL {
                            if wide || swap != ByteSwap::WordOrder {
                                ui.selectable_value(&mut selection.swap, swap, swap.label());
                            }
                        }
                    });
            } else {
                selection.swap = ByteSwap::None;
            }
        }
    });
}

fn edit_fields(
    ui: &mut Ui,
    direction: &str,
    fields: &mut Vec<CaptureField>,
    mut logged: Option<&mut Vec<String>>,
    size: u16,
) -> bool {
    let mut remove = None;
    for (index, field) in fields.iter_mut().enumerate() {
        ui.push_id((direction, index), |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                let previous = field.name.clone();
                if ui.text_edit_singleline(&mut field.name).changed()
                    && let Some(names) = logged.as_deref_mut()
                {
                    for name in names.iter_mut().filter(|name| **name == previous) {
                        name.clone_from(&field.name);
                    }
                }
                if let Some(names) = logged.as_deref_mut() {
                    let mut included = names.contains(&field.name);
                    if ui.checkbox(&mut included, "CSV").changed() {
                        if included {
                            names.push(field.name.clone());
                        } else {
                            names.retain(|name| name != &field.name);
                        }
                    }
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
            edit_selection(ui, &mut field.selection);
            if let Err(error) = field.selection.validate(usize::from(size)) {
                ui.colored_label(RED, error);
            }
            ui.separator();
        });
    }
    if let Some(index) = remove {
        let field = fields.remove(index);
        if let Some(names) = logged {
            names.retain(|name| name != &field.name);
        }
        return true;
    }
    if fields.len() < model::MAX_MAPPED_FIELDS
        && ui.button(format!("Add {direction} field")).clicked()
    {
        let name = (1..)
            .map(|n| format!("{direction}_{n}"))
            .find(|name| {
                !fields
                    .iter()
                    .any(|field| field.name.eq_ignore_ascii_case(name))
            })
            .unwrap();
        fields.push(CaptureField {
            name,
            selection: Selection {
                kind: Kind::U8,
                ..Selection::default()
            },
        });
        return true;
    }
    false
}

impl Device {
    pub(super) fn mappings(&mut self, ui: &mut Ui) {
        ui.label("Define the primary readback and named values. CSV checkboxes select which input values are logged.");
        ui.add_enabled_ui(!self.link_busy() && !self.capturing, |ui| {
            let s = &mut self.settings;
            title(ui, "Assembly mappings");
            egui::CollapsingHeader::new("Primary input readback")
                .default_open(true)
                .show(ui, |ui| {
                    edit_selection(ui, &mut s.selection);
                    if let Err(e) = s.selection.validate(s.input_size as usize) {
                        ui.colored_label(RED, e);
                    }
                });
            egui::CollapsingHeader::new("Named inputs and CSV selection")
                .default_open(true)
                .show(ui, |ui| {
                    let mut all = s.log_fields.is_none();
                    if ui.checkbox(&mut all, "Log all mapped inputs").changed() {
                        s.log_fields = if all {
                            None
                        } else if s.capture_fields.is_empty() {
                            Some(vec!["value".into()])
                        } else {
                            Some(
                                s.capture_fields
                                    .iter()
                                    .map(|field| field.name.clone())
                                    .collect(),
                            )
                        };
                    }
                    let had_fields = !s.capture_fields.is_empty();
                    if !had_fields {
                        ui.small("With no named inputs, CSV uses the primary readback.");
                        if let Some(names) = &mut s.log_fields {
                            let mut included = names.iter().any(|name| name == "value");
                            if ui.checkbox(&mut included, "Log primary value").changed() {
                                *names = if included {
                                    vec!["value".into()]
                                } else {
                                    Vec::new()
                                };
                            }
                        }
                    }
                    edit_fields(
                        ui,
                        "input",
                        &mut s.capture_fields,
                        s.log_fields.as_mut(),
                        s.input_size,
                    );
                    if had_fields == s.capture_fields.is_empty() {
                        if let Some(names) = &mut s.log_fields {
                            *names = if s.capture_fields.is_empty() {
                                vec!["value".into()]
                            } else {
                                s.capture_fields
                                    .iter()
                                    .map(|field| field.name.clone())
                                    .collect()
                            };
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("CSV rows");
                        ui.selectable_value(
                            &mut s.capture_mode,
                            CaptureMode::EveryPacket,
                            "Every packet",
                        );
                        ui.selectable_value(
                            &mut s.capture_mode,
                            CaptureMode::OnChange,
                            "On change",
                        );
                    });
                    match s.csv_fields() {
                        Ok(fields) => {
                            let names = fields
                                .iter()
                                .map(|field| field.name.as_str())
                                .collect::<Vec<_>>();
                            ui.small(format!("CSV columns · {}", names.join(", ")));
                        }
                        Err(e) => {
                            ui.colored_label(AMBER, e);
                        }
                    }
                });
            if !s.input_only {
                ui.collapsing("Named output mappings", |ui| {
                    if edit_fields(ui, "output", &mut s.output_fields, None, s.output_size) {
                        self.output_entries.clear();
                    }
                });
            }
            ui.collapsing("Preview input payload", |ui| self.show_input_preview(ui));
        });
    }
}

impl Device {
    fn show_input_preview(&mut self, ui: &mut Ui) {
        ui.small("Paste assembly bytes in hex, with spaces between bytes. Exclude sequence and Run/Idle headers.");
        ui.add(
            egui::TextEdit::multiline(&mut self.preview_hex)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .code_editor(),
        );
        if self.preview_hex.trim().is_empty() {
            return;
        }
        let preview = model::parse_hex(&self.preview_hex)
            .and_then(|bytes| self.settings.preview_input(&bytes));
        match preview {
            Ok(preview) => {
                egui::ScrollArea::both()
                    .id_salt("input-preview-scroll")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        egui::Grid::new("input-preview")
                            .striped(true)
                            .show(ui, |ui| {
                                for heading in ["Field", "Offset", "Bytes", "Value", "CSV"] {
                                    ui.strong(heading);
                                }
                                ui.end_row();
                                for field in preview.fields {
                                    ui.label(field.name);
                                    ui.monospace(field.selection.offset.to_string());
                                    ui.monospace(field.raw_hex);
                                    ui.monospace(field.value);
                                    ui.label(if field.included_in_csv { "Yes" } else { "No" });
                                    ui.end_row();
                                }
                            });
                    });
            }
            Err(error) => {
                ui.colored_label(AMBER, error);
            }
        }
    }
}
