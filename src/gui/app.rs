use super::*;

struct RememberedSession {
    path: PathBuf,
    saved: Workspace,
    check_at: Instant,
    error: Option<String>,
}

pub(super) struct App {
    pub(super) devices: Vec<Device>,
    pub(super) selected: usize,
    next_id: u64,
    remove_confirmed: bool,
    manage_devices: bool,
    session: Option<RememberedSession>,
}

impl App {
    pub(super) fn new(benchmarks: Vec<Benchmark>, session_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            devices: Vec::new(),
            selected: 0,
            next_id: 1,
            remove_confirmed: false,
            manage_devices: false,
            session: None,
        };
        if !benchmarks.is_empty() {
            for bench in benchmarks {
                app.devices.push(Device::new(app.next_id, Some(bench)));
                app.next_id += 1;
            }
            return app;
        }
        if let Some(path) = session_path {
            let error = match Workspace::load(&path) {
                Ok(Some(saved)) => {
                    app.selected = saved.selected_device;
                    for device in saved.devices {
                        app.devices.push(Device::restore(app.next_id, device));
                        app.next_id += 1;
                    }
                    None
                }
                Ok(None) => None,
                Err(error) => Some(format!("Could not restore last session — {error}")),
            };
            if app.devices.is_empty() {
                app.add_device();
            }
            app.session = Some(RememberedSession {
                path,
                saved: app.snapshot(),
                check_at: Instant::now(),
                error,
            });
        } else {
            app.add_device();
            app.devices[0].error = Some(
                "LOCALAPPDATA is unavailable. Use named TOML profiles to save settings.".into(),
            );
        }
        app
    }

    pub(super) fn add_device(&mut self) {
        self.devices.push(Device::new(self.next_id, None));
        self.next_id += 1;
        self.selected = self.devices.len() - 1;
        self.remove_confirmed = false;
    }

    pub(super) fn snapshot(&self) -> Workspace {
        Workspace {
            selected_device: self.selected,
            devices: self.devices.iter().map(Device::snapshot).collect(),
        }
    }

    pub(super) fn save_session(&mut self) {
        if self.session.is_none() {
            return;
        }
        let draft = self.snapshot();
        let session = self.session.as_mut().unwrap();
        session.check_at = Instant::now() + Duration::from_secs(1);
        if draft != session.saved {
            match draft.save(&session.path) {
                Ok(()) => {
                    session.saved = draft;
                    session.error = None;
                }
                Err(error) => {
                    session.error = Some(format!("Could not save last session — {error}"));
                }
            }
        }
    }

    fn device_list(&mut self, ui: &mut Ui) {
        let prior = self.selected;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Devices");
            egui::ComboBox::from_id_salt("selected-device")
                .selected_text(self.devices[self.selected].list_label())
                .show_ui(ui, |ui| {
                    for (index, device) in self.devices.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, index, device.list_label());
                    }
                });
            if self.selected != prior {
                self.remove_confirmed = false;
            }
            if ui.button("Add device").clicked() {
                self.add_device();
            }
            if ui.button("Manage devices…").clicked() {
                self.manage_devices = true;
            }
            if let Some(session) = &self.session {
                if let Some(error) = &session.error {
                    ui.colored_label(AMBER, error);
                } else {
                    ui.small("Saved locally")
                        .on_hover_text(session.path.display().to_string());
                }
            }
        });
    }

    fn manage_devices_dialog(&mut self, ctx: &egui::Context) {
        if !self.manage_devices {
            return;
        }
        let mut open = true;
        egui::Window::new("Manage devices")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                let can_remove = self.devices.len() > 1 && !self.devices[self.selected].needs_repaint();
                let device = &mut self.devices[self.selected];
                ui.label(format!("Selected adapter {}", device.device));
                ui.horizontal(|ui| {
                    ui.label("Display name");
                    ui.add(egui::TextEdit::singleline(&mut device.name).id_salt(device.id));
                });
                if ui.add_enabled(can_remove, egui::Button::new("Remove device"))
                    .on_disabled_hover_text("Disconnect and stop capture before removing a device. Keep at least one device.")
                    .clicked()
                {
                    self.remove_confirmed = true;
                }
                self.remove_confirmed &= can_remove;
                if self.remove_confirmed {
                    ui.label("Remove this setup from the saved device list?");
                    ui.horizontal(|ui| {
                        if ui.button("Remove setup").clicked() {
                            self.devices.remove(self.selected);
                            self.selected = self.selected.min(self.devices.len() - 1);
                            self.remove_confirmed = false;
                        } else if ui.button("Cancel").clicked() {
                            self.remove_confirmed = false;
                        }
                    });
                }
            });
        self.manage_devices = open;
        if !open {
            self.remove_confirmed = false;
        }
    }

    fn reserved_ports(&self) -> Vec<(Ipv4Addr, u16)> {
        self.devices
            .iter()
            .enumerate()
            .filter(|(index, device)| *index != self.selected && device.link_busy())
            .filter_map(|(_, device)| device.active.as_ref())
            .filter(|settings| settings.local_udp_port != 0)
            .map(|settings| (settings.local_ip, settings.local_udp_port))
            .collect()
    }
}

impl Device {
    fn list_label(&self) -> String {
        let name = if self.name.trim().is_empty() {
            "Unnamed device"
        } else {
            &self.name
        };
        let capture = if self.capturing { " · CSV armed" } else { "" };
        format!("{name} · {} · {}{capture}", self.device, self.state_label())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        for device in &self.devices {
            if let Some(connection) = &device.connection {
                connection.stop();
            }
        }
        self.save_session();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let benchmarking = self.devices.iter().any(|device| device.bench.is_some());
        for device in &mut self.devices {
            device.poll();
            device.benchmark();
        }
        if benchmarking && self.devices.iter().all(|device| device.bench.is_none()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::TopBottomPanel::top("devices").show(ctx, |ui| self.device_list(ui));
        let reserved_ports = self.reserved_ports();
        self.devices[self.selected].show(ctx, &reserved_ports);
        self.manage_devices_dialog(ctx);
        if self
            .session
            .as_ref()
            .is_some_and(|session| Instant::now() >= session.check_at)
        {
            self.save_session();
        }
        if let Some(session) = &self.session {
            ctx.request_repaint_after(session.check_at.saturating_duration_since(Instant::now()));
        }
        if self.devices.iter().any(Device::needs_repaint) {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
