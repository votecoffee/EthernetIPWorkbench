use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Tab {
    #[default]
    Device,
    Explore,
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
                    "Explore",
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
                    (Tab::Explore, "Explore EtherNet/IP"),
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
    #[ignore = "requires tools/loopback_adapter.py --mode saved-reads on localhost"]
    fn runs_and_displays_saved_reads_from_explore_headlessly() {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let size = egui::vec2(1240.0, 1100.0);
        let mut device = Device::new(1, None);
        device.tab = Tab::Explore;
        device.preferences.cip_reads = [
            ("Product name", 7, None),
            ("Unavailable route", 6, Some("1/7")),
            ("Serial", 6, Some("1/3")),
            ("Peer closes", 126, None),
            ("Later", 4, None),
        ]
        .into_iter()
        .map(
            |(name, attribute, route)| assembly_scanner::explicit::SavedRead {
                name: name.into(),
                path: assembly_scanner::explicit::ReadPath {
                    class: 1,
                    instance: 1,
                    attribute: Some(attribute),
                },
                route: route.map(|route| route.parse().unwrap()),
                selection: None,
            },
        )
        .collect();
        let wait_for_explore = |device: &mut Device| {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while device.explorer.busy() && Instant::now() < deadline {
                device.explorer.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!device.explorer.busy());
        };
        click(&mut device, &ctx, size, "Saved reads");
        click(&mut device, &ctx, size, "Read all saved");
        wait_for_explore(&mut device);
        click(&mut device, &ctx, size, "Saved read results");
        let labels = draw(&mut device, &ctx, size);
        for expected in [
            "Product name · Reply",
            "Unavailable route · CIP rejection",
            "Serial · Reply",
            "Peer closes · Exchange failed",
            "Later · Not attempted",
        ] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "{expected}"
            );
        }
        click(&mut device, &ctx, size, "Serial · Reply");
        let labels = draw(&mut device, &ctx, size);
        for expected in ["Route 1/3", "0x12345678", "0000  78 56 34 12"] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "{expected}"
            );
        }
        device.preferences.directory.clear();
        click(&mut device, &ctx, size, "Save JSON");
        wait_for_explore(&mut device);
        assert!(
            draw(&mut device, &ctx, size)
                .iter()
                .any(|(text, _)| text == "Set an output directory in Logging")
        );

        let folder = super::super::tests::temporary("saved-read-reports");
        device.preferences.directory = folder.display().to_string();
        for _ in 0..2 {
            click(&mut device, &ctx, size, "Save JSON");
            wait_for_explore(&mut device);
        }
        let reports = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(reports.len(), 2);
        let first = fs::read(&reports[0]).unwrap();
        assert_eq!(fs::read(&reports[1]).unwrap(), first);
        let report: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(report["adapter_or_bridge"], "127.0.0.1");
        assert_eq!(report["results"][1]["extended_status"][0], 0x0311);
        assert_eq!(report["results"][2]["data_hex"], "78 56 34 12");
        assert_eq!(report["results"][2]["decoded"], "0x12345678");
        assert_eq!(report["results"][3]["status"], "exchange_failed");
        assert_eq!(report["results"][4]["status"], "not_attempted");
        for report in reports {
            assert!(
                report
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("saved-reads-")
            );
            assert_eq!(report.extension().unwrap(), "json");
            fs::remove_file(report).unwrap();
        }
        fs::remove_dir(folder).unwrap();
        assert!(device.connection.is_none());
    }

    #[test]
    fn loading_a_saved_read_restores_its_reply_editor() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1240.0, 860.0);
        let mut device = Device::new(1, None);
        device.tab = Tab::Explore;
        let saved = toml::from_str(
            r#"
name = "Test reply"
selection = { offset = 2, kind = "U16", big_endian = true, swap = "bytes_in_words" }
path = { class = 4, instance = 101, attribute = 3 }
"#,
        )
        .unwrap();
        device.preferences.cip_reads.push(saved);
        let labels = draw(&mut device, &ctx, size);
        assert!(!labels.iter().any(|(text, _)| text == "Byte offset"));

        click(&mut device, &ctx, size, "Saved reads");
        click(&mut device, &ctx, size, "Test reply");
        let labels = draw(&mut device, &ctx, size);
        for expected in [
            "Byte offset",
            "U16",
            "Big endian",
            "Swap bytes within words",
        ] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "{expected}"
            );
        }
        assert!(!device.explorer.busy() && device.connection.is_none());
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
        device.settings.output_size = 12;
        device.settings.output_min_size = Some(2);
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
            "Variable O->T · 2–12 payload bytes",
            "Initial output length · 2 bytes",
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

    #[test]
    fn adding_eds_outputs_preserves_staged_bytes_and_pending_values() {
        let source = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                concat!(
                    "Param2 = 0,,,0,0xC3,2,\"Command value\";\n",
                    "Param3 = 0,,,0,0xC6,1,\"Mode\";\n",
                    "Param4 = 0,,,0,0xC1,,\"Enable\";\n",
                    "[Assembly]"
                ),
            )
            .replace(
                "[Connection Manager]",
                "Assem102 = \"Outputs\", \"\", ,0x0001,,,8,Param3,3,,1,Param4,4,,,Param2;\n[Connection Manager]",
            )
            .replace("0x02010002", "0x04010002")
            .replace("0x22440305", "0x22440405")
            .replace("Param1,0,,", "Param1,,Assem102,")
            .replace("2C FE", "2C 66");
        let mut device = Device::new(1, None);
        device.profiles = eds::parse(&source).unwrap();
        device.select_profile(Some(0));
        device.settings.output_hex = "11 22 33 44".into();
        device.settings.config_hex = "AA BB".into();
        let existing = CaptureField {
            name: "mode".into(),
            selection: Selection {
                kind: Kind::U8,
                ..Selection::default()
            },
        };
        device.settings.output_fields.push(existing.clone());
        device.output_entries.push("17".into());
        device.tab = Tab::Mappings;
        let ctx = egui::Context::default();
        let size = egui::vec2(1240.0, 860.0);

        device.settings.output_instance = 103;
        let labels = draw(&mut device, &ctx, size);
        assert!(
            !labels
                .iter()
                .any(|(text, _)| text == "EDS output mapping suggestions")
        );
        device.settings.output_instance = 102;
        click(&mut device, &ctx, size, "EDS output mapping suggestions");
        let labels = draw(&mut device, &ctx, size);
        assert!(
            labels
                .iter()
                .any(|(text, _)| text == "Enable · byte 1, bit 3 · Bool")
        );
        click(&mut device, &ctx, size, "Add to named outputs (2)");

        assert_eq!(device.settings.output_fields.len(), 3);
        assert_eq!(device.settings.output_fields[0], existing);
        assert_eq!(device.settings.output_fields[1].name, "Enable");
        assert_eq!(device.settings.output_fields[2].name, "Command_value");
        assert_eq!(device.output_entries, ["17"]);
        assert_eq!(device.settings.output_hex, "11 22 33 44");
        assert_eq!(device.settings.config_hex, "AA BB");
        assert!(!device.writes_confirmed && !device.bench_confirmed);
        assert!(device.connection.is_none());
        device.settings.stage_output(1, "true").unwrap();
        assert_eq!(device.settings.output().unwrap(), [0x11, 0x2a, 0x33, 0x44]);
        device.settings.stage_output(1, "false").unwrap();
        assert_eq!(device.settings.output().unwrap(), [0x11, 0x22, 0x33, 0x44]);
        device.settings.stage_output(2, "-300").unwrap();
        assert_eq!(device.settings.output().unwrap(), [0x11, 0x22, 0xd4, 0xfe]);
    }

    #[test]
    fn connection_review_shows_only_out_of_range_rpi_requests() {
        let ctx = egui::Context::default();
        let mut device = Device::new(1, None);
        let source = include_str!("../../samples/loopback.eds")
            .replace("1000,1000000,1000", "2000,1000000,2000");
        device.profiles = eds::parse(&source).unwrap();
        device.select_profile(Some(0));
        device.settings.input_rpi_us = 1_000;
        device.connect_dialog = true;
        let size = egui::vec2(1240.0, 860.0);
        draw(&mut device, &ctx, size);
        let labels = draw(&mut device, &ctx, size);
        assert!(labels.iter().any(|(text, _)| {
            text == "T->O input RPI · 1000 µs requested · EDS Param1 · 2000–1000000 µs · Requested interval below minimum"
        }));
        assert!(
            !labels
                .iter()
                .any(|(text, _)| text.starts_with("O->T output RPI"))
        );
        assert_eq!(device.settings.input_rpi_us, 1_000);
        assert!(device.connection.is_none());
    }

    #[test]
    fn review_shows_the_eds_identity_comparison() {
        let ctx = egui::Context::default();
        let mut device = Device::new(1, None);
        device.profiles = eds::parse(include_str!("../../samples/loopback.eds")).unwrap();
        device.select_profile(Some(0));
        device.connect_dialog = true;
        let identity = scanner::Identity {
            vendor: 0,
            device_type: 12,
            code: 7,
            major: 1,
            minor: 2,
            status: 0,
            serial: 1234,
            name: "Fixture".into(),
        };
        device.status.identity = Some(identity.clone());
        let size = egui::vec2(1240.0, 860.0);
        draw(&mut device, &ctx, size);
        let labels = draw(&mut device, &ctx, size);
        for expected in [
            "Adapter identity differs from the selected EDS.",
            "Product code · EDS 1 · adapter 7",
            "Minor revision · EDS 0 · adapter 2",
        ] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "{expected}"
            );
        }
        device.status.identity = Some(scanner::Identity {
            code: 1,
            minor: 0,
            ..identity
        });
        let labels = draw(&mut device, &ctx, size);
        assert!(
            labels
                .iter()
                .any(|(text, _)| text == "EDS identity matches on 5 declared fields.")
        );
        assert!(device.connection.is_none() && device.verified_identity.is_none());
    }

    #[test]
    fn configuration_editor_stages_eds_boolean_locally() {
        let source = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                concat!(
                    "Param2 = 0,,,0x0010,0xC6,1,\"Reserved value\";\n",
                    "Param3 = 0,,,0,0xC1,,\"Enable flag\";\n",
                    "[Assembly]"
                ),
            )
            .replace(
                "[Connection Manager]",
                "Assem201 = \"Configuration\", \"\", 2,0,,,8,Param2,1,Param3,7,;\n[Connection Manager]",
            )
            .replace("    0,,", "    ,Assem201,");
        let mut device = Device::new(1, None);
        device.profiles = eds::parse(&source).unwrap();
        device.select_profile(Some(0));
        let layout = device.profiles[0].configuration_layout.as_ref().unwrap();
        assert_eq!(layout.fields.len(), 1);
        assert_eq!(layout.skipped_members, 1);
        assert!(device.settings.config_hex.is_empty());
        let ctx = egui::Context::default();
        let size = egui::vec2(1240.0, 1200.0);
        click(&mut device, &ctx, size, "Configuration and advanced timing");
        device.settings.config_instance = Some(2);
        let labels = draw(&mut device, &ctx, size);
        assert!(
            !labels
                .iter()
                .any(|(text, _)| text == "Stage typed configuration values")
        );
        device.settings.config_instance = Some(1);
        click(&mut device, &ctx, size, "Stage typed configuration values");
        draw(&mut device, &ctx, size);
        device.configuration_entries[0] = "true".into();
        click(&mut device, &ctx, size, "Stage");
        assert!(device.settings.config_hex.is_empty());
        device.settings.config_hex = "A5 A0".into();
        device.writes_confirmed = true;
        click(&mut device, &ctx, size, "Stage");
        assert_eq!(device.settings.config_hex, "A5 A1");
        assert!(!device.writes_confirmed);
        assert!(device.settings.output_hex.is_empty());
        assert!(device.connection.is_none());
    }
}

impl Tab {
    const ALL: [Self; 8] = [
        Self::Device,
        Self::Explore,
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
            Self::Explore => "Explore",
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
                            Tab::Explore => self.explore(ui),
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
            if self.status.valid != 0
                && self
                    .active
                    .as_ref()
                    .is_some_and(|settings| settings.input_min_size.is_some())
            {
                ui.label(format!("Last input · {} bytes", self.status.input.len()));
            }
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
