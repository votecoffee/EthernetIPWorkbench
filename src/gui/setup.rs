use super::*;

impl Device {
    pub(super) fn profile_controls(&mut self, ui: &mut Ui) {
        ui.label("Load a named setup or save this device's settings.");
        ui.add_enabled_ui(!self.link_busy() && !self.capturing, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Settings TOML");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.toml_path)
                            .desired_width(200.0)
                            .hint_text("Path to a TOML profile"),
                    )
                    .changed()
                {
                    self.replace_profile = None;
                }
                let named = !self.toml_path.trim().is_empty();
                if ui.add_enabled(named, egui::Button::new("Load")).clicked() {
                    self.load_profile();
                }
                if ui.add_enabled(named, egui::Button::new("Save")).clicked() {
                    self.save_profile(None);
                }
            });
            if let Some(path) = self.replace_profile.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(AMBER, format!("Replace {}?", path.display()));
                    if ui.button("Replace profile").clicked() {
                        self.save_profile(Some(path));
                    } else if ui.button("Cancel").clicked() {
                        self.replace_profile = None;
                    }
                });
            }
        });
        if self.link_busy() || self.capturing {
            ui.small("Disconnect and stop capture to load or save settings.");
        }
    }

    pub(super) fn setup(&mut self, ui: &mut Ui) {
        title(ui, "Device setup");
        ui.label("Choose an EDS connection or enter the assembly settings, then read identity before connecting.");
        ui.add_enabled_ui(!self.link_busy() && !self.capturing, |ui| {
            ui.horizontal(|ui| {
                ui.label("EDS file");
                ui.text_edit_singleline(&mut self.eds_path);
                if ui.button("Import").clicked() {
                    match eds::load(Path::new(&self.eds_path)) {
                        Ok(profiles) => {
                            self.profiles = profiles;
                            let supported_input = self.profiles.iter().position(|p| {
                                p.settings.role() == model::ConnectionRole::InputOnly && p.unsupported.is_empty()
                            });
                            let supported_listener = || self.profiles.iter().position(|p| {
                                p.settings.listen_only && p.unsupported.is_empty()
                            });
                            self.select_profile(Some(supported_input.or_else(supported_listener).unwrap_or(0)));
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            });
            let mut chosen = self.selected;
            egui::ComboBox::from_id_salt("eds-profiles")
                .selected_text(chosen.map_or_else(|| "Manual settings".into(), |i| format!("{} · {}", self.profiles[i].id, self.profiles[i].name)))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "Manual settings");
                    for (i, p) in self.profiles.iter().enumerate() {
                        ui.selectable_value(&mut chosen, Some(i), format!("{} · {}", p.id, p.name));
                    }
                });
            if chosen != self.selected { self.select_profile(chosen); }
            if let Some(i) = self.selected {
                let p = &self.profiles[i];
                ui.collapsing("EDS source values and fields to review", |ui| {
                    for (key, value) in &p.fields {
                        ui.label(format!("{key} = {value}"));
                        if p.size_from_assembly_definition(key) {
                            ui.colored_label(AMBER, "Assembly definition width. Verify the configured device payload.");
                        }
                    }
                    for key in ["O->T RPI", "T->O RPI", "T->O size"] {
                        if !p.fields.contains_key(key) { ui.colored_label(AMBER, format!("{key} needs input")); }
                    }
                    ui.label("Enter device and NIC addresses, selected input, and any output or configuration bytes. The fields below override the imported values.");
                });
                for issue in &p.unsupported { ui.colored_label(AMBER, issue); }
            }
            egui::Grid::new("network").num_columns(2).show(ui, |ui| {
                ui.label("Adapter IPv4");
                let device_changed = ui.text_edit_singleline(&mut self.device).changed();
                ui.end_row();
                ui.label("Local NIC IPv4");
                let local_changed = ui.text_edit_singleline(&mut self.local).changed();
                ui.end_row();
                if device_changed || local_changed {
                    self.reset_endpoint_status();
                    self.bench_confirmed = false;
                    self.writes_confirmed = false;
                }
                number(ui, "Local UDP port (0 = automatic)", &mut self.settings.local_udp_port);
            });
            let s = &mut self.settings;
            let write_settings = |s: &Settings| (
                s.role(), s.output_instance, (s.output_size, s.output_min_size), s.output_run_idle,
                s.config_instance, s.config_size, s.timeout_multiplier_code,
                s.large_forward_open, s.electronic_key,
                s.input_size, s.input_min_size,
                s.input_multicast,
            );
            let before = write_settings(s);
            ui.separator();
            let prior_role = s.role();
            let mut role = prior_role;
            egui::ComboBox::from_label("Connection role")
                .selected_text(role.label())
                .show_ui(ui, |ui| {
                    for choice in [model::ConnectionRole::InputOnly, model::ConnectionRole::ListenOnly,
                        model::ConnectionRole::ExclusiveOwner] {
                        ui.selectable_value(&mut role, choice, choice.label());
                    }
                });
            if role != prior_role {
                s.set_role(role);
                if s.input_only {
                    s.output_min_size = None;
                }
                if matches!(role, model::ConnectionRole::ListenOnly) || matches!(prior_role, model::ConnectionRole::ListenOnly) {
                    // An adapter distinguishes the roles by their connection points.
                    s.output_instance = 0;
                }
            }
            if s.listen_only {
                ui.small("Use the adapter's listen-only heartbeat connection point. A controlling multicast connection must already be running.");
            }
            egui::Grid::new("assembly").num_columns(3).striped(true).show(ui, |ui| {
                ui.label(""); ui.strong("T->O input"); ui.strong("O->T output"); ui.end_row();
                ui.label("Connection point");
                ui.add(egui::DragValue::new(&mut s.input_instance));
                ui.add(egui::DragValue::new(&mut s.output_instance)); ui.end_row();
                ui.label(if s.input_min_size.is_some() || s.output_min_size.is_some() {
                    "Assembly bytes (max if variable)"
                } else {
                    "Assembly bytes"
                });
                ui.add(egui::DragValue::new(&mut s.input_size));
                ui.add(egui::DragValue::new(&mut s.output_size)); ui.end_row();
                ui.label("Requested RPI (µs)");
                ui.add(egui::DragValue::new(&mut s.input_rpi_us).speed(10));
                ui.add(egui::DragValue::new(&mut s.output_rpi_us).speed(10)); ui.end_row();
                ui.label("32-bit Run/Idle");
                ui.checkbox(&mut s.input_run_idle, "");
                ui.checkbox(&mut s.output_run_idle, ""); ui.end_row();
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut s.input_multicast, "Multicast input (T->O)")
                    .on_hover_text("The adapter selects the input group and port. Outputs remain unicast.");
            });
            ui.horizontal_wrapped(|ui| {
                let mut variable = s.input_min_size.is_some();
                if ui.checkbox(&mut variable, "Variable-size input (T->O)").changed() {
                    s.input_min_size = variable.then_some(s.input_size.max(1));
                }
                if let Some(min) = &mut s.input_min_size {
                    ui.label("Minimum input bytes");
                    ui.add(egui::DragValue::new(min).range(1..=s.input_size.max(1)).clamp_existing_to_range(false));
                }
            });
            ui.collapsing("Configuration and advanced timing", |ui| {
                if !s.input_only || s.output_min_size.is_some() {
                    ui.horizontal_wrapped(|ui| {
                        let mut variable = s.output_min_size.is_some();
                        if ui.checkbox(&mut variable, "Variable-size output (O->T)").changed() {
                            s.output_min_size = variable.then_some(s.output_size);
                        }
                        if let Some(min) = &mut s.output_min_size {
                            ui.label("Minimum output bytes");
                            ui.add(egui::DragValue::new(min).range(0..=s.output_size).clamp_existing_to_range(false));
                        }
                    });
                }
                ui.checkbox(&mut s.large_forward_open, "Use Large Forward_Open");
                let mut keyed = s.electronic_key.is_some();
                if ui.checkbox(&mut keyed, "Use electronic key (exact mode)").changed() {
                    s.electronic_key = keyed.then(|| {
                        self.status.identity.as_ref().map_or_else(model::ElectronicKey::default, |id| model::ElectronicKey {
                            vendor: id.vendor,
                            device_type: id.device_type,
                            product: id.code,
                            major: id.major,
                            minor: id.minor,
                        })
                    });
                }
                if let Some(key) = s.electronic_key.as_mut() {
                    egui::Grid::new("electronic-key").num_columns(2).show(ui, |ui| {
                        number(ui, "Vendor", &mut key.vendor);
                        number(ui, "Device type", &mut key.device_type);
                        number(ui, "Product code", &mut key.product);
                        number(ui, "Major revision", &mut key.major);
                        number(ui, "Minor revision", &mut key.minor);
                    });
                }
                egui::Grid::new("connection-priority").num_columns(3).show(ui, |ui| {
                    ui.label("CIP priority (0–3)");
                    ui.add(egui::DragValue::new(&mut s.input_priority).range(0..=3));
                    ui.add(egui::DragValue::new(&mut s.output_priority).range(0..=3));
                    ui.end_row();
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("CIP timeout code (0–7)");
                    ui.add(egui::DragValue::new(&mut s.timeout_multiplier_code)
                        .range(0..=model::MAX_TIMEOUT_MULTIPLIER_CODE)
                        .clamp_existing_to_range(false));
                    if let Some(factor) = s.connection_timeout(None).factor {
                        ui.small(format!("×{factor}"));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let mut configured = s.config_instance.is_some();
                    if ui.checkbox(&mut configured, "Configuration instance").changed() {
                        s.config_instance = configured.then_some(1);
                    }
                    if let Some(instance) = &mut s.config_instance {
                        ui.add(egui::DragValue::new(instance));
                    }
                    ui.label("Bytes");
                    ui.add(egui::DragValue::new(&mut s.config_size));
                });
                ui.label("Configuration hex bytes (sent in Forward_Open)");
                if ui.add(egui::TextEdit::multiline(&mut s.config_hex)
                    .desired_rows(2).desired_width(f32::INFINITY).code_editor()).changed()
                {
                    self.writes_confirmed = false;
                }
                ui.label("Producer schedule");
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.preferences.schedule, Schedule::Deadline, "Deadline");
                    #[cfg(windows)]
                    ui.selectable_value(&mut self.preferences.schedule, Schedule::DeadlineAboveNormal, "Deadline (above normal)");
                    ui.selectable_value(&mut self.preferences.schedule, Schedule::Tokio, "Tokio baseline");
                });
            });
            if before != write_settings(s) {
                self.writes_confirmed = false;
            }
            ui.collapsing("Offline profile check", |ui| self.show_preflight(ui));
        });
        self.identity_view(ui);
    }

    pub(super) fn connection_review(&mut self, ui: &mut Ui, reserved_ports: &[(Ipv4Addr, u16)]) {
        ui.label(format!(
            "Adapter {} · Local NIC {}",
            self.device, self.local
        ));
        ui.label(format!(
            "Input assembly {} · {} bytes · Requested {:.3} ms",
            self.settings.input_instance,
            self.settings.input_size,
            self.settings.input_rpi_us as f64 / 1000.0
        ));
        if let Some(min) = self.settings.input_min_size {
            ui.label(format!(
                "Variable T->O · {min}–{} payload bytes",
                self.settings.input_size
            ));
        }
        if self.settings.input_multicast {
            ui.label("Multicast T->O · the adapter selects the group and port");
        }
        if self.settings.listen_only {
            ui.label(format!(
                "Listen-only · Heartbeat assembly {}",
                self.settings.output_instance
            ));
            ui.small("The adapter must already be producing this multicast input for a controlling connection.");
        }
        if self.settings.large_forward_open {
            ui.label("Large Forward_Open");
        }
        if let Some(key) = self.settings.electronic_key {
            ui.label(format!(
                "Electronic key · {}:{}:{} · rev {}.{} (exact mode)",
                key.vendor, key.device_type, key.product, key.major, key.minor
            ));
        }
        if !self.settings.input_only {
            if let Some(min) = self.settings.output_min_size {
                ui.label(format!(
                    "Variable O->T · {min}–{} payload bytes",
                    self.settings.output_size
                ));
                if let Ok(bytes) = self.settings.output() {
                    ui.label(format!("Initial output length · {} bytes", bytes.len()));
                }
            }
            ui.label(format!("Staged outputs · {}", self.settings.output_hex));
        }
        if self.settings.config_size != 0 {
            ui.label(format!(
                "Configuration bytes · {}",
                self.settings.config_hex
            ));
        }
        if !self.settings.input_only {
            ui.colored_label(
                AMBER,
                "Connecting requests output ownership and sends the staged output bytes.",
            );
        }
        if self.settings.config_size != 0 {
            ui.colored_label(AMBER, "Connecting applies the staged configuration bytes.");
        }
        if self.settings.output_run_idle {
            ui.colored_label(AMBER, "The scanner will assert Run in the O->T header.");
        }
        if !self
            .addresses()
            .is_ok_and(|(a, b)| a.is_loopback() && b.is_loopback())
        {
            ui.checkbox(
                &mut self.bench_confirmed,
                "I verified the adapter and isolated bench network",
            );
        }
        if !self.settings.input_only
            || self.settings.config_size != 0
            || self.settings.output_run_idle
        {
            ui.checkbox(
                &mut self.writes_confirmed,
                "Ownership and these output/configuration writes are authorized",
            );
        }
        if self.settings.supports_auto_reconnect() {
            ui.checkbox(
                &mut self.preferences.auto_reconnect,
                "Reconnect automatically",
            );
            if self.preferences.auto_reconnect && self.verified_identity.is_none() {
                ui.small("Use Read identity to verify this device before connecting.");
            }
        }
        if let Some(pin) = self.verified_identity {
            ui.small(format!(
                "Connect checks the verified identity · Serial {:08X}",
                pin.serial
            ));
        }
        if self.identity_task.is_some() {
            ui.small("Reading identity…");
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.identity_task.is_none(), egui::Button::new("Connect"))
                .clicked()
            {
                self.connect(reserved_ports);
            }
        });
    }
    pub(super) fn select_profile(&mut self, i: Option<usize>) {
        self.selected = i;
        self.writes_confirmed = false;
        self.output_entries.clear();
        if let Some(i) = i {
            let mut settings = self.profiles[i].settings.clone();
            settings.selection = self.settings.selection;
            settings.capture_fields = std::mem::take(&mut self.settings.capture_fields);
            settings.output_fields = std::mem::take(&mut self.settings.output_fields);
            settings.log_fields = self.settings.log_fields.take();
            settings.capture_mode = self.settings.capture_mode;
            settings.local_udp_port = self.settings.local_udp_port;
            settings.timeout_multiplier_code = self.settings.timeout_multiplier_code;
            self.settings = settings;
        }
    }
    pub(super) fn show_preflight(&self, ui: &mut Ui) {
        ui.small("Checks the configuration locally. Verify the adapter identity and payload on the bench.");
        let settings = match self.current() {
            Ok(settings) => settings,
            Err(error) => {
                ui.colored_label(RED, error);
                return;
            }
        };
        let profile = self.selected.map(|i| &self.profiles[i]);
        let report = preflight::inspect(&settings, &self.preferences, profile);
        if report.connection_valid {
            ui.colored_label(CYAN, "Connection settings valid");
        }
        for error in &report.connection_errors {
            ui.colored_label(RED, error);
        }
        if report.capture_config_valid {
            ui.colored_label(CYAN, "Capture settings valid");
        }
        for error in &report.capture_errors {
            ui.colored_label(AMBER, error);
        }
        for (label, direction) in [
            ("T->O input", &report.input),
            ("O->T output", &report.output),
        ] {
            let bytes = direction.minimum_payload_bytes.map_or_else(
                || direction.payload_bytes.to_string(),
                |min| format!("{min}–{}", direction.payload_bytes),
            );
            ui.label(format!(
                "{label} · Assembly {} · {} bytes · {} µs requested",
                direction.assembly, bytes, direction.requested_rpi_us
            ));
        }
        if let Some(us) = report.connection_timeout.requested_receive_watchdog_us {
            ui.small(format!(
                "Receive watchdog at requested input RPI · {:.3} ms",
                us as f64 / 1000.0
            ));
        }
        if report.requests_output_ownership {
            ui.colored_label(AMBER, "Connecting requests output ownership");
        }
        if let Some(length) = report.staged_output_bytes {
            ui.label(format!("Staged output length · {length} bytes"));
        }
        if report.asserts_run {
            ui.colored_label(AMBER, "Connecting asserts Run in the O->T header");
        }
        if report.configuration_bytes != 0 {
            ui.colored_label(
                AMBER,
                format!(
                    "Connecting sends {} configuration bytes",
                    report.configuration_bytes
                ),
            );
        }
        if !report.origins.is_empty() {
            egui::Grid::new("preflight-origins")
                .striped(true)
                .show(ui, |ui| {
                    for origin in report.origins {
                        ui.label(origin.field);
                        ui.monospace(origin.effective);
                        let response = ui.label(origin.state.label());
                        if let Some(value) = origin.eds {
                            response.on_hover_text(value);
                        }
                        ui.end_row();
                    }
                });
        }
    }
    pub(super) fn identity_view(&mut self, ui: &mut Ui) {
        title(ui, "Device identity");
        if let Some(id) = &self.status.identity {
            ui.label(RichText::new(&id.name).size(20.0).strong());
            ui.label(format!(
                "Vendor {}  ·  Type {}  ·  Product {}  ·  Revision {}.{}",
                id.vendor, id.device_type, id.code, id.major, id.minor
            ));
            ui.label(format!(
                "Serial {:08X}  ·  Device status 0x{:04X}",
                id.serial, id.status
            ));
        } else {
            ui.label("Identity unavailable");
        }
        if ui
            .add_enabled(
                !self.link_busy() && self.identity_task.is_none(),
                egui::Button::new("Read identity"),
            )
            .on_disabled_hover_text(
                "Available after disconnect and once the previous read finishes",
            )
            .clicked()
        {
            self.begin_identity_read();
        }
    }
}
