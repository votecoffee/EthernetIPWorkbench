use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Tab {
    #[default]
    Device,
    Inputs,
    Outputs,
    Logging,
    Mappings,
    Integration,
    Diagnostics,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(
        device: &mut Device,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> Vec<(String, egui::Rect)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| device.show(ctx, &[]));
        let mut labels = Vec::new();
        fn collect(shape: egui::epaint::Shape, labels: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::epaint::Shape::Text(text) => {
                    labels.push((
                        text.galley.job.text.clone(),
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                    ));
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, labels);
                    }
                }
                _ => {}
            }
        }
        for shape in output.shapes {
            collect(shape.shape, &mut labels);
        }
        labels
    }

    #[test]
    fn tabs_isolate_workflows_and_keep_connection_actions_visible() {
        for size in [egui::vec2(900.0, 580.0), egui::vec2(1240.0, 860.0)] {
            let ctx = egui::Context::default();
            let mut device = Device::new(1, None);
            device.status.network_error = Some("Simulated transport error".into());
            let saved = device.snapshot();
            for tab in Tab::ALL {
                device.tab = tab;
                draw(&mut device, &ctx, size);
                let labels = draw(&mut device, &ctx, size);
                assert!(
                    labels
                        .iter()
                        .any(|(text, _)| text == "Simulated transport error")
                );
                for label in [
                    "Connect…",
                    "Record CSV…",
                    "Profiles…",
                    "Device",
                    "Inputs",
                    "Outputs",
                    "Logging",
                    "Mappings",
                    "Integration",
                    "Diagnostics",
                ] {
                    let (_, bounds) = labels
                        .iter()
                        .find(|(text, _)| text == label)
                        .unwrap_or_else(|| panic!("{label} missing from {tab:?}"));
                    assert!(
                        bounds.min.x >= 0.0 && bounds.max.x <= size.x,
                        "{label} {bounds:?}"
                    );
                    assert!(
                        bounds.min.y >= 0.0 && bounds.max.y <= size.y,
                        "{label} {bounds:?}"
                    );
                }
                for (page, heading) in [
                    (Tab::Device, "Device setup"),
                    (Tab::Inputs, "Live inputs"),
                    (Tab::Outputs, "Stage and apply outputs"),
                    (Tab::Logging, "CSV logging"),
                    (Tab::Mappings, "Assembly mappings"),
                    (Tab::Integration, "Software integration"),
                    (Tab::Diagnostics, "Connection diagnostics"),
                ] {
                    assert_eq!(
                        labels.iter().any(|(text, _)| text == heading),
                        page == tab,
                        "{tab:?} {heading}"
                    );
                }
                assert_eq!(device.snapshot(), saved);
                assert!(device.connection.is_none() && device.capture.is_none());
            }
        }
    }

    fn click(device: &mut Device, ctx: &egui::Context, size: egui::Vec2, label: &str) {
        draw(device, ctx, size);
        let labels = draw(device, ctx, size);
        let (_, bounds) = labels
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("Missing control {label}"));
        let pos = bounds.center();
        for pressed in [true, false] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| device.show(ctx, &[]));
        }
    }

    #[test]
    fn persistent_actions_navigate_and_stop_capture_from_another_tab() {
        let ctx = egui::Context::default();
        let mut device = Device::new(1, None);
        let size = egui::vec2(900.0, 580.0);
        click(&mut device, &ctx, size, "Record CSV…");
        assert_eq!(device.tab, Tab::Logging);
        assert!(device.capture.is_none());
        click(&mut device, &ctx, size, "Profiles…");
        assert!(device.profile_dialog);
        assert!(
            draw(&mut device, &ctx, size)
                .iter()
                .any(|(text, _)| text == "Device profiles")
        );
        device.profile_dialog = false;

        let mut id = [0; 8];
        getrandom::fill(&mut id).unwrap();
        let directory =
            std::env::temp_dir().join(format!("eip-workspace-{:016x}", u64::from_le_bytes(id)));
        assert!(!directory.exists());
        device.preferences.directory = directory.display().to_string();
        device.start_capture();
        assert!(device.capturing);
        let sink = device.capture.as_ref().unwrap().sink.clone();
        sink.record(Instant::now(), 1, 1, 1, &[0; 4]);
        device.tab = Tab::Inputs;
        click(&mut device, &ctx, size, "Stop capture and flush");
        assert!(!device.capturing);
        assert_eq!(sink.written.load(Relaxed), 1);
        drop(sink);
        drop(device);
        for file in fs::read_dir(&directory).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn owner_review_displays_write_effects_without_connecting() {
        let ctx = egui::Context::default();
        let mut device = Device::new(1, None);
        device.device = "192.0.2.10".into();
        device.local = "192.0.2.11".into();
        device.settings.input_only = false;
        device.settings.output_size = 2;
        device.settings.output_hex = "11 22".into();
        device.settings.config_size = 3;
        device.settings.config_hex = "AA BB CC".into();
        device.settings.output_run_idle = true;
        device.connect_dialog = true;
        let size = egui::vec2(900.0, 580.0);
        draw(&mut device, &ctx, size);
        let labels = draw(&mut device, &ctx, size);
        for expected in [
            "Review connection",
            "Staged outputs · 11 22",
            "Configuration bytes · AA BB CC",
            "Connecting requests output ownership and sends the staged output bytes.",
            "Connecting applies the staged configuration bytes.",
            "The scanner will assert Run in the O->T header.",
            "Ownership and these output/configuration writes are authorized",
        ] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "{expected}"
            );
        }
        assert!(!device.bench_confirmed && !device.writes_confirmed);
        assert!(device.connection.is_none() && device.capture.is_none());
    }
}

impl Tab {
    const ALL: [Self; 7] = [
        Self::Device,
        Self::Inputs,
        Self::Outputs,
        Self::Logging,
        Self::Mappings,
        Self::Integration,
        Self::Diagnostics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Device => "Device",
            Self::Inputs => "Inputs",
            Self::Outputs => "Outputs",
            Self::Logging => "Logging",
            Self::Mappings => "Mappings",
            Self::Integration => "Integration",
            Self::Diagnostics => "Diagnostics",
        }
    }
}

impl Device {
    pub(super) fn show(&mut self, ctx: &egui::Context, reserved_ports: &[(Ipv4Addr, u16)]) {
        egui::TopBottomPanel::top("connection").show(ctx, |ui| {
            ui.push_id(self.id, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.heading(&self.name);
                    ui.label(&self.device);
                    ui.colored_label(
                        if self.link_busy() { CYAN } else { AMBER },
                        self.state_label(),
                    );
                    if self.connection.is_some() {
                        if ui.button("Disconnect").clicked() {
                            self.disconnect();
                        }
                    } else if self.reconnect_pending().is_some() {
                        if ui.button("Cancel reconnect").clicked() {
                            self.disconnect();
                        }
                    } else if ui
                        .add_enabled(self.identity_task.is_none(), egui::Button::new("Connect…"))
                        .clicked()
                    {
                        self.connect_dialog = true;
                    }
                    if self.capturing {
                        if ui.button("Stop capture and flush").clicked() {
                            self.stop_capture();
                        }
                    } else if ui.button("Record CSV…").clicked() {
                        self.tab = Tab::Logging;
                    }
                    if ui.button("Profiles…").clicked() {
                        self.profile_dialog = true;
                    }
                });
                self.alerts(ui);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for tab in Tab::ALL {
                        ui.selectable_value(&mut self.tab, tab, tab.label());
                    }
                });
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.push_id(self.id, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(self.tab)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(1000.0);
                        if matches!(self.tab, Tab::Device | Tab::Mappings | Tab::Integration)
                            && (self.link_busy() || self.capturing)
                        {
                            ui.colored_label(
                                AMBER,
                                "Disconnect and stop capture to edit this setup.",
                            );
                        }
                        match self.tab {
                            Tab::Device => self.setup(ui),
                            Tab::Inputs => {
                                self.input_health(ui);
                                self.inputs(ui);
                            }
                            Tab::Outputs => self.outputs(ui),
                            Tab::Logging => self.logging(ui),
                            Tab::Mappings => self.mappings(ui),
                            Tab::Integration => {
                                title(ui, "Software integration");
                                ui.label("Share input data with local programs or a broker.");
                                ui.add_enabled_ui(!self.link_busy() && !self.capturing, |ui| {
                                    self.edit_integration(ui)
                                });
                            }
                            Tab::Diagnostics => self.diagnostics(ui),
                        }
                    });
            });
        });
        self.dialogs(ctx, reserved_ports);
    }

    fn alerts(&mut self, ui: &mut Ui) {
        if let Some(error) = self
            .error
            .as_ref()
            .or(self.status.network_error.as_ref())
            .or(self.status.cip_error.as_ref())
            .or(self.status.identity_error.as_ref())
        {
            ui.colored_label(RED, error);
        } else if let Some(notice) = &self.notice {
            ui.colored_label(CYAN, notice);
        }
        let disk_error = self
            .capture
            .as_ref()
            .and_then(|capture| capture.sink.error.lock().unwrap().clone());
        if let Some(error) = disk_error {
            ui.colored_label(RED, format!("CSV writer error · {error}"));
        }
        let overruns = self
            .capture
            .as_ref()
            .map_or(0, |capture| capture.sink.overrun.load(Relaxed));
        if self.status.timeouts != 0 || self.status.gaps != 0 || overruns != 0 {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    AMBER,
                    format!(
                        "Timeouts {} · Packet gaps {} · CSV overruns {overruns}",
                        self.status.timeouts, self.status.gaps
                    ),
                );
                if ui.small_button("Review diagnostics").clicked() {
                    self.tab = Tab::Diagnostics;
                }
            });
        }
    }

    fn input_health(&self, ui: &mut Ui) {
        let age = self.connection.as_ref().map_or_else(
            || self.status.last_input.map(|stamp| stamp.elapsed()),
            Connection::input_age,
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Accepted input packets {}", self.status.valid));
            ui.label(age.map_or_else(
                || "No input received".into(),
                |age| format!("Last input {:.2} ms ago", age.as_secs_f64() * 1000.0),
            ));
            if let Some((ot, input)) = self.status.actual {
                ui.label(format!(
                    "Actual intervals · Input {input} µs · Output {ot} µs"
                ));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, reserved_ports: &[(Ipv4Addr, u16)]) {
        if self.profile_dialog {
            let mut open = true;
            egui::Window::new("Device profiles")
                .id(egui::Id::new(("profiles-dialog", self.id)))
                .open(&mut open)
                .default_width(620.0)
                .show(ctx, |ui| self.profile_controls(ui));
            self.profile_dialog = open;
        }
        if self.connect_dialog && !self.link_busy() {
            let mut open = true;
            egui::Window::new("Review connection")
                .id(egui::Id::new(("connect-dialog", self.id)))
                .open(&mut open)
                .default_width(620.0)
                .vscroll(true)
                .show(ctx, |ui| {
                    self.identity_view(ui);
                    ui.separator();
                    self.connection_review(ui, reserved_ports);
                    if let Some(error) = &self.error {
                        ui.colored_label(RED, error);
                    }
                });
            self.connect_dialog = open && !self.link_busy();
        } else {
            self.connect_dialog = false;
        }
    }
}
