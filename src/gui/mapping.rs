use super::*;

pub(super) fn edit_selection(ui: &mut Ui, selection: &mut Selection) {
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
            let profile = self.selected.and_then(|i| self.profiles.get(i));
            let s = &mut self.settings;
            let minimum = s.input_min_size.unwrap_or(s.input_size);
            title(ui, "Assembly mappings");
            if s.input_min_size.is_some() {
                ui.small(format!(
                    "Input mappings must fit in the shortest accepted payload ({minimum} bytes)."
                ));
            }
            egui::CollapsingHeader::new("Primary input readback")
                .default_open(true)
                .show(ui, |ui| {
                    edit_selection(ui, &mut s.selection);
                    if let Err(e) = s.selection.validate(usize::from(minimum)) {
                        ui.colored_label(RED, e);
                    }
                });
            if let Some(profile) = profile
                && profile.settings.input_instance == s.input_instance
                && let Some(layout) = &profile.input_layout
            {
                egui::CollapsingHeader::new("EDS input mapping suggestions").show(ui, |ui| {
                    let primary = primary_mapping(s, minimum);
                    if let Some(fields) = choose_suggestions(
                        ui,
                        layout,
                        &s.capture_fields,
                        minimum,
                        model::MAX_MAPPED_FIELDS - usize::from(primary.is_some()),
                        "input",
                    ) {
                        append_suggestions(s, primary, fields);
                    }
                });
            }
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
                        minimum,
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
                if let Some(profile) = profile
                    && profile.settings.output_instance == s.output_instance
                    && let Some(layout) = &profile.output_layout
                {
                    ui.collapsing("EDS output mapping suggestions", |ui| {
                        ui.small("Add fields for typed staging in Outputs. This action only adds mappings.");
                        if let Some(fields) = choose_suggestions(
                            ui,
                            layout,
                            &s.output_fields,
                            s.output_size,
                            model::MAX_MAPPED_FIELDS,
                            "output",
                        ) {
                            s.output_fields.extend(fields);
                        }
                    });
                }
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

fn choose_suggestions(
    ui: &mut Ui,
    layout: &eds::AssemblyLayout,
    existing: &[CaptureField],
    payload_size: u16,
    max_fields: usize,
    direction: &str,
) -> Option<Vec<CaptureField>> {
    ui.small(format!(
        "{} · Raw CIP values. Review names, scaling, and byte order against the device manual.",
        layout.format
    ));
    if layout.skipped_members > 0 {
        ui.small(format!(
            "{} typed members need manual mapping.",
            layout.skipped_members
        ));
    }
    let available = max_fields.saturating_sub(existing.len());
    let candidates: Vec<_> = layout
        .fields
        .iter()
        .filter(|field| {
            field.selection.validate(usize::from(payload_size)).is_ok()
                && !existing
                    .iter()
                    .any(|current| current.name.eq_ignore_ascii_case(&field.name))
        })
        .take(available)
        .collect();
    if candidates.is_empty() {
        ui.small(format!(
            "No additional suggestions fit the current {direction} mapping."
        ));
        return None;
    }
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            for field in &candidates {
                let bit = field
                    .selection
                    .bit
                    .map_or_else(String::new, |bit| format!(", bit {bit}"));
                ui.label(format!(
                    "{} · byte {}{bit} · {:?}",
                    field.name, field.selection.offset, field.selection.kind,
                ));
            }
        });
    if ui
        .button(format!("Add to named {direction}s ({})", candidates.len()))
        .clicked()
    {
        return Some(candidates.into_iter().cloned().collect());
    }
    None
}

fn primary_mapping(settings: &Settings, minimum: u16) -> Option<CaptureField> {
    (settings.capture_fields.is_empty()
        && settings.selection.validate(usize::from(minimum)).is_ok())
    .then(|| CaptureField {
        name: "value".into(),
        selection: settings.selection,
    })
}

fn append_suggestions(
    settings: &mut Settings,
    primary: Option<CaptureField>,
    candidates: Vec<CaptureField>,
) {
    if settings.capture_fields.is_empty() && primary.is_none() {
        // A primary readback that does not fit could not previously be logged.
        settings.log_fields = Some(Vec::new());
    }
    settings.capture_fields.extend(primary);
    // Preserve the previous CSV schema until the operator selects more columns.
    settings.log_fields.get_or_insert_with(|| {
        settings
            .capture_fields
            .iter()
            .map(|field| field.name.clone())
            .collect()
    });
    settings.capture_fields.extend(candidates);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appending_eds_suggestions_preserves_the_current_csv_columns() {
        let mut settings = Settings {
            input_size: 8,
            ..Settings::default()
        };
        let primary = primary_mapping(&settings, 8);
        let logged = settings.csv_fields().unwrap();
        let candidate = CaptureField {
            name: "Net_weight".into(),
            selection: Selection {
                offset: 4,
                kind: Kind::I16,
                ..Selection::default()
            },
        };
        append_suggestions(&mut settings, primary, vec![candidate.clone()]);
        assert_eq!(settings.csv_fields().unwrap(), logged);
        assert_eq!(settings.input_mapping().unwrap()[1], candidate);
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
