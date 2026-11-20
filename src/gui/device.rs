use super::*;

impl Device {
    pub(super) fn new(id: u64, bench: Option<Benchmark>) -> Self {
        let mut device = Self {
            id,
            name: format!("Device {id}"),
            tab: Tab::Device,
            profile_dialog: false,
            connect_dialog: false,
            explorer: explore::Explorer::default(),
            settings: Settings::default(),
            device: "127.0.0.1".into(),
            local: "127.0.0.1".into(),
            eds_path: "samples/loopback.eds".into(),
            toml_path: String::new(),
            replace_profile: None,
            profiles: vec![],
            selected: None,
            bench_confirmed: false,
            writes_confirmed: false,
            output_entries: Vec::new(),
            configuration_entries: Vec::new(),
            preview_hex: String::new(),
            connection: None,
            integration: None,
            integration_counters: None,
            active: None,
            status: Status::default(),
            generation: 0,
            tx: 0,
            skipped: 0,
            tx_spacing: Spacing::default(),
            send_timing: SendTiming::default(),
            capture: None,
            capture_run: run_report::CaptureRun::default(),
            slot: Default::default(),
            capturing: false,
            preferences: Preferences::default(),
            notice: None,
            error: None,
            identity_task: None,
            verified_identity: None,
            reconnect: None,
            bench,
        };
        if let Some(bench) = &device.bench {
            device.preferences.directory = bench.directory.to_string_lossy().into_owned();
            device.preferences.schedule = bench.schedule;
            device.preferences.rotate_seconds = 3;
            if let Some(profile) = &bench.profile {
                match settings_file::load_profile(profile) {
                    Ok((settings, _)) => device.settings = settings,
                    Err(error) => {
                        device.error = Some(format!("Could not load loopback settings: {error}"));
                        return device;
                    }
                }
            }
            if let Some((target, rpi)) = bench.target {
                device.device = target.to_string();
                device.settings.input_rpi_us = rpi;
                device.settings.output_rpi_us = rpi;
            }
            if bench.owner {
                device.settings.input_only = false;
                device.settings.output_instance = 100;
                device.settings.output_size = 2;
                device.settings.output_run_idle = true;
                device.settings.input_run_idle = true;
                device.settings.output_hex = "00 00".into();
                device.settings.output_fields = vec![CaptureField {
                    name: "test_word".into(),
                    selection: Selection {
                        kind: Kind::U16,
                        big_endian: true,
                        ..Selection::default()
                    },
                }];
                device
                    .settings
                    .stage_output(0, "4660")
                    .expect("fixture output mapping");
                device.settings.config_size = 3;
                device.settings.config_hex = "AA BB CC".into();
                device.writes_confirmed = true;
            }
            device.start_capture();
            device.connect(&[]);
        }
        device
    }

    pub(super) fn restore(id: u64, saved: NamedDevice) -> Self {
        let mut device = Self::new(id, None);
        device.name = saved.name;
        let saved = saved.setup;
        device.settings = saved.settings;
        device.preferences = saved.preferences;
        device.device = saved.device;
        device.local = saved.local;
        device.eds_path = saved.eds_path;
        device.toml_path = saved.profile_path;
        device.profiles = saved.eds_profiles;
        device.selected = saved.selected_eds;
        device
    }

    pub(super) fn snapshot(&self) -> NamedDevice {
        NamedDevice {
            name: self.name.clone(),
            setup: Session {
                settings: self.settings.clone(),
                preferences: self.preferences.clone(),
                device: self.device.clone(),
                local: self.local.clone(),
                eds_path: self.eds_path.clone(),
                profile_path: self.toml_path.clone(),
                eds_profiles: self.profiles.clone(),
                selected_eds: self.selected,
            },
        }
    }

    pub(super) fn state_label(&self) -> &str {
        if self.reconnect_pending().is_some() {
            "Reconnect pending"
        } else if self.status.state.is_empty() {
            "Offline"
        } else {
            self.status.state
        }
    }

    pub(super) fn reconnect_pending(&self) -> Option<(usize, Instant)> {
        self.reconnect.as_ref().and_then(Reconnect::pending)
    }

    pub(super) fn link_busy(&self) -> bool {
        self.connection.is_some() || self.reconnect_pending().is_some()
    }

    pub(super) fn needs_repaint(&self) -> bool {
        self.link_busy()
            || self.capturing
            || self.bench.is_some()
            || self.identity_task.is_some()
            || self.explorer.busy()
    }

    pub(super) fn load_profile(&mut self) {
        self.replace_profile = None;
        self.notice = None;
        match settings_file::load_named_profile(Path::new(&self.toml_path)) {
            Ok(profile) => {
                self.reset_endpoint_status();
                self.device = profile.settings.device_ip.to_string();
                self.local = profile.settings.local_ip.to_string();
                self.settings = profile.settings;
                if let Some(preferences) = profile.preferences {
                    self.preferences = preferences;
                }
                self.notice = if let Some(eds_profile) = profile.eds_profile {
                    self.profiles = vec![eds_profile];
                    self.selected = Some(0);
                    self.eds_path.clear();
                    Some("Loaded settings and their saved EDS profile".into())
                } else {
                    self.selected = None;
                    Some("Loaded settings".into())
                };
                self.output_entries.clear();
                self.configuration_entries.clear();
                self.preview_hex.clear();
                self.bench_confirmed = false;
                self.writes_confirmed = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn save_profile(&mut self, confirmed_path: Option<PathBuf>) {
        let replace = confirmed_path.is_some();
        let path = confirmed_path.unwrap_or_else(|| PathBuf::from(&self.toml_path));
        self.replace_profile = None;
        self.notice = None;
        let eds_profile = self.selected.map(|index| &self.profiles[index]);
        if eds_profile.is_some_and(|profile| !profile.unsupported.is_empty()) {
            self.error =
                Some("Resolve the listed EDS profile issues in Manual mode before saving".into());
            return;
        }
        let text = match self.current().and_then(|settings| {
            settings_file::profile_text_with_eds(&settings, &self.preferences, eds_profile)
        }) {
            Ok(text) => text,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let result = if replace {
            settings_file::replace(&path, &text)
        } else {
            settings_file::write_new(&path, &text)
        };
        match result {
            Ok(()) => {
                self.error = None;
                self.notice = Some(format!("Saved settings to {}", path.display()));
            }
            Err(error) if !replace && error.kind() == ErrorKind::AlreadyExists => {
                self.error = None;
                self.replace_profile = Some(path);
            }
            Err(error) => self.error = Some(format!("Could not save {} — {error}", path.display())),
        }
    }
    pub(super) fn reset_endpoint_status(&mut self) {
        self.integration_counters = None;
        self.verified_identity = None;
        self.identity_task = None;
        self.active = None;
        self.status = Status::default();
        self.tx = 0;
        self.skipped = 0;
        self.tx_spacing = Spacing::default();
        self.send_timing = SendTiming::default();
    }

    pub(super) fn addresses(&self) -> Result<(Ipv4Addr, Ipv4Addr), String> {
        Ok((
            self.device
                .trim()
                .parse()
                .map_err(|_| "Enter a valid adapter IPv4 address")?,
            self.local
                .trim()
                .parse()
                .map_err(|_| "Enter a valid local NIC IPv4 address")?,
        ))
    }
    pub(super) fn current(&self) -> Result<Settings, String> {
        let mut s = self.settings.clone();
        (s.device_ip, s.local_ip) = self.addresses()?;
        Ok(s)
    }

    pub(super) fn begin_identity_read(&mut self) {
        if self.link_busy() || self.identity_task.is_some() {
            return;
        }
        match self.current() {
            Ok(cfg) => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(scanner::read_identity(&cfg));
                });
                self.identity_task = Some(rx);
                self.bench_confirmed = false;
                self.writes_confirmed = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn connect(&mut self, reserved_ports: &[(Ipv4Addr, u16)]) {
        self.error = None;
        self.notice = None;
        let result = (|| {
            if self.identity_task.is_some() {
                return Err("Wait for the identity read to finish".into());
            }
            let s = self.current()?;
            if s.local_udp_port != 0 && reserved_ports.contains(&(s.local_ip, s.local_udp_port)) {
                return Err(format!(
                    "UDP {}:{} is already reserved by another device. Choose port 0 or another supported bind endpoint.",
                    s.local_ip, s.local_udp_port
                ));
            }
            let physical = !(s.device_ip.is_loopback() && s.local_ip.is_loopback());
            if physical && !self.bench_confirmed {
                return Err("Confirm the isolated bench connection".into());
            }
            if let Some(i) = self.selected {
                if !self.profiles[i].unsupported.is_empty() {
                    return Err(
                        "Resolve the listed profile issues in Manual mode before connecting".into(),
                    );
                }
            }
            let auto_reconnect = self.preferences.auto_reconnect && s.supports_auto_reconnect();
            let pin = self.verified_identity;
            if auto_reconnect && pin.is_none() {
                return Err(
                    "Use Read identity and verify the adapter before enabling automatic reconnect"
                        .into(),
                );
            }
            s.validate()?;
            if !self.writes_confirmed && !s.input_only_without_writes() {
                return Err(
                    "Confirm bench ownership, configuration and output permission first".into(),
                );
            }
            self.integration = Integration::start(&self.preferences.integration, &s)?;
            self.integration_counters = None;
            if let Err(error) = self.open_connection(s, pin, self.writes_confirmed) {
                self.integration = None;
                return Err(error);
            }
            self.reconnect = pin.filter(|_| auto_reconnect).map(Reconnect::new);
            self.tab = Tab::Inputs;
            Ok(())
        })();
        self.bench_confirmed = false;
        self.writes_confirmed = false;
        if let Err(e) = result {
            self.error = Some(e);
        }
    }
    pub(super) fn open_connection(
        &mut self,
        settings: Settings,
        pin: Option<IdentityPin>,
        permit_writes: bool,
    ) -> Result<(), String> {
        let generation = self.generation + 1;
        let schedule = self.preferences.schedule;
        let connection = Connection::start_with_publisher(
            settings.clone(),
            self.slot.clone(),
            generation,
            schedule,
            permit_writes,
            pin.map(ExpectedIdentity::Device),
            self.integration.as_ref().map(Integration::publisher),
        )?;
        self.generation = generation;
        if self.capturing {
            self.capture_run.track(generation);
        }
        self.active = Some(settings);
        self.status = Status::default();
        self.tx = 0;
        self.skipped = 0;
        self.tx_spacing = Spacing::default();
        self.send_timing = SendTiming::default();
        self.identity_task = None;
        self.connection = Some(connection);
        Ok(())
    }

    pub(super) fn disconnect(&mut self) {
        self.reconnect = None;
        self.notice = None;
        if let Some(connection) = &self.connection {
            connection.stop();
        }
    }

    pub(super) fn start_capture(&mut self) {
        let settings = self
            .active
            .as_ref()
            .filter(|_| self.link_busy())
            .unwrap_or(&self.settings);
        let options = capture::Options {
            duration_seconds: self.preferences.gui_capture_seconds,
            ..self
                .preferences
                .capture_options(PathBuf::from(&self.preferences.directory))
        };
        match Capture::start(&self.slot, options, settings) {
            Ok(capture) => {
                self.capture = Some(capture);
                self.capture_run = run_report::CaptureRun::default();
                if self.connection.is_some() {
                    self.capture_run.track(self.generation);
                }
                self.capturing = true;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not start CSV capture — {e}")),
        }
    }
    pub(super) fn stop_capture(&mut self) {
        if let Some(capture) = self.capture.as_mut() {
            capture.stop(&self.slot);
        }
        self.capturing = false;
    }
    pub(super) fn poll(&mut self) {
        self.explorer.poll();
        if let Some(c) = &self.connection {
            let finished = c.finished();
            let diagnostics = if finished {
                self.connection.take().unwrap().finish()
            } else {
                c.diagnostics()
            };
            self.status = diagnostics.status;
            self.tx = diagnostics.sent;
            self.skipped = diagnostics.skipped;
            self.tx_spacing = diagnostics.send_spacing;
            self.send_timing = diagnostics.send_timing;
            if finished {
                self.record_capture_connection();
                self.writes_confirmed = false;
                if let Some(reconnect) = &mut self.reconnect {
                    if !self.status.retryable {
                        self.reconnect = None;
                    } else if !reconnect.schedule(Instant::now()) {
                        self.reconnect = None;
                        self.error = Some("Automatic reconnect stopped after three retries".into());
                    }
                }
            }
        }
        if self.capturing
            && let Some(capture) = &self.capture
        {
            if capture.finished() {
                self.stop_capture();
            } else if capture
                .sink
                .deadline()
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.capture_run.duration_reached = true;
                self.stop_capture();
            }
        }
        if let Some((attempt, _)) = self.reconnect_pending()
            && let Some(pin) = self.reconnect.as_mut().unwrap().take_ready(Instant::now())
        {
            let settings = self.active.as_ref().expect("reconnect settings").clone();
            match self.open_connection(settings, Some(pin), false) {
                Ok(()) => self.notice = Some(format!("Automatic reconnect attempt {attempt} of 3")),
                Err(error) => {
                    self.reconnect = None;
                    self.error = Some(error);
                }
            }
        }
        if !self.link_busy()
            && let Some(integration) = self.integration.take()
        {
            self.integration_counters = Some(integration.counters());
        }
        if let Some(task) = &self.identity_task {
            match task.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(identity) => {
                            self.verified_identity = Some(identity.pin());
                            self.bench_confirmed = false;
                            self.writes_confirmed = false;
                            self.status.identity = Some(identity);
                            self.status.identity_error = None;
                        }
                        Err(error) => {
                            self.status.identity = None;
                            self.status.identity_error = Some(error);
                        }
                    }
                    self.identity_task = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.identity_task = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        if let Some(c) = self.connection.take() {
            c.stop();
            c.join();
        }
        self.stop_capture();
    }
}
