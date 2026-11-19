use super::*;

pub(super) struct Benchmark {
    pub(super) schedule: Schedule,
    pub(super) directory: PathBuf,
    pub(super) started: Instant,
    pub(super) stopping: bool,
    pub(super) owner: bool,
    pub(super) profile: Option<PathBuf>,
    pub(super) reconnect: bool,
    pub(super) transitioned: bool,
    pub(super) target: Option<(Ipv4Addr, u32)>,
}

impl Device {
    pub(super) fn benchmark(&mut self) {
        let Some(bench) = &mut self.bench else {
            return;
        };
        if bench.started.elapsed() >= Duration::from_secs(4) && !bench.transitioned {
            if bench.owner && self.status.state == "Receiving" {
                if let Some(c) = &self.connection {
                    self.settings
                        .stage_output(0, "22136")
                        .expect("fixture output mapping");
                    c.apply_output(self.settings.output().expect("fixture output bytes"))
                        .expect("receiving fixture owner session");
                }
                bench.transitioned = true;
            } else if bench.reconnect && self.generation == 1 {
                if let Some(c) = &self.connection {
                    c.stop();
                    return;
                }
                bench.transitioned = true;
                self.connect(&[]);
                return;
            }
        }
        if !bench.stopping
            && (bench.started.elapsed() >= Duration::from_secs(12) || self.connection.is_none())
        {
            bench.stopping = true;
            if let Some(c) = &self.connection {
                c.stop();
            }
        }
        if !bench.stopping || self.connection.is_some() {
            return;
        }
        let directory = bench.directory.clone();
        self.stop_capture();
        let spacing = toml::to_string(&self.status.spacing).unwrap_or_default();
        let tx_spacing = toml::to_string(&self.tx_spacing).unwrap_or_default();
        let send_timing = toml::to_string(&self.send_timing).unwrap_or_default();
        let cip_error = self.status.cip_error.as_deref().unwrap_or_default();
        let network_error = self.status.network_error.as_deref().unwrap_or_default();
        let (ot, to) = self.status.actual.unwrap_or_default();
        let (requested_ot, requested_to) = self.active.as_ref().map_or(
            (self.settings.output_rpi_us, self.settings.input_rpi_us),
            |s| (s.output_rpi_us, s.input_rpi_us),
        );
        let setup_error = self.error.as_deref().unwrap_or_default();
        let identity_serial = self.status.identity.as_ref().map_or(0, |id| id.serial);
        let adapter_ip = self
            .active
            .as_ref()
            .map_or(self.device.as_str().to_string(), |cfg| {
                cfg.device_ip.to_string()
            });
        let (seen, unchanged, written, overrun, disk_error) = self
            .capture
            .as_ref()
            .map(|capture| {
                let s = &capture.sink;
                (
                    s.seen.load(Relaxed),
                    s.unchanged.load(Relaxed),
                    s.written.load(Relaxed),
                    s.overrun.load(Relaxed),
                    s.error.lock().unwrap().clone(),
                )
            })
            .unwrap_or_default();
        let text = format!(
            "state = {:?}\nschedule = {:?}\nadapter_ip = {adapter_ip:?}\nidentity_serial = {identity_serial}\nrequested_ot_us = {requested_ot}\nrequested_to_us = {requested_to}\nactual_ot_us = {ot}\nactual_to_us = {to}\nvalid = {}\ngaps = {}\nduplicates = {}\nlate = {}\ninvalid = {}\ntimeouts = {}\ntx = {}\nskipped_deadlines = {}\nseen = {seen}\nunchanged = {unchanged}\nwritten = {written}\noverruns = {overrun}\ndisk_error = {}\ngenerations = {}\ncip_error = {cip_error:?}\nnetwork_error = {network_error:?}\nsetup_error = {setup_error:?}\n\n[spacing]\n{spacing}\n[tx_spacing]\n{tx_spacing}\n[send_timing]\n{send_timing}",
            self.status.state,
            format!("{:?}", self.preferences.schedule),
            self.status.valid,
            self.status.gaps,
            self.status.duplicates,
            self.status.late,
            self.status.invalid,
            self.status.timeouts,
            self.tx,
            self.skipped,
            disk_error.is_some(),
            self.generation
        );
        let _ = fs::write(directory.join("scanner.toml"), text);
        self.bench = None;
    }
}
