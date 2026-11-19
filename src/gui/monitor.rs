use super::*;

impl Device {
    pub(super) fn diagnostics(&mut self, ui: &mut Ui) {
        title(ui, "Connection diagnostics");
        if let Some(e) = &self.status.identity_error {
            ui.colored_label(AMBER, format!("Identity — {e}"));
        }
        if let Some(e) = &self.status.network_error {
            ui.colored_label(RED, format!("Transport — {e}"));
        }
        if let Some(e) = &self.status.cip_error {
            ui.colored_label(RED, format!("Connection Manager — {e}"));
        }
        if let Some(close) = &self.status.forward_close {
            ui.colored_label(
                if close.is_confirmed() { CYAN } else { AMBER },
                format!("Forward_Close {close}"),
            );
        }
        if let Some(active) = &self.active {
            if !active.io_route.trim().is_empty() {
                ui.label(format!(
                    "Bridge {} · Cyclic I/O route {}",
                    active.device_ip, active.io_route
                ));
            }
            egui::Grid::new("intervals")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Intervals in µs");
                    ui.strong("O->T");
                    ui.strong("T->O");
                    ui.end_row();
                    ui.label("Requested RPI");
                    ui.label(active.output_rpi_us.to_string());
                    ui.label(active.input_rpi_us.to_string());
                    ui.end_row();
                    ui.label("Forward_Open actual");
                    if let Some((ot, to)) = self.status.actual {
                        ui.label(ot.to_string());
                        ui.label(to.to_string());
                    } else {
                        ui.label("—");
                        ui.label("—");
                    }
                    ui.end_row();
                });
            if let Some(startup) = self.status.startup_timing(self.send_timing) {
                ui.small(format!(
                    "Forward_Open exchange {:.3} ms",
                    startup.forward_open_exchange_us as f64 / 1000.0
                ));
                if self.status.actual.is_some() {
                    let show = |us: Option<u64>| {
                        us.map_or_else(|| "—".into(), |us| format!("{:.3} ms", us as f64 / 1000.0))
                    };
                    ui.small(format!(
                        "After Forward_Open · First O->T send {} · First UDP {} · First valid input {}",
                        show(startup.first_send_after_open_us),
                        show(startup.first_udp_after_open_us),
                        show(startup.first_input_after_open_us)
                    ));
                }
            }
            if let Some(io) = self.status.cyclic_io {
                let input = io
                    .first_input_source
                    .map_or_else(|| "—".into(), |addr| addr.to_string());
                ui.small(format!(
                    "UDP local {} · O->T destination {} · First T->O source {input}",
                    io.local_udp, io.output_destination
                ));
                if let (Some(group), Some(listener)) =
                    (io.input_multicast_destination, io.input_local_udp)
                {
                    ui.small(format!(
                        "T->O multicast {group} · Local receiver {listener}"
                    ));
                }
                ui.small(format!(
                    "Connection IDs · O->T 0x{:08X} · T->O 0x{:08X}",
                    io.ot_connection_id, io.to_connection_id
                ));
            }
            let timeout = active.connection_timeout(self.status.actual.map(|(_, input)| input));
            if let (Some(factor), Some(us)) = (timeout.factor, timeout.receive_watchdog_us) {
                ui.small(format!(
                    "Receive watchdog {:.3} ms · CIP code {} · ×{factor}",
                    us as f64 / 1000.0,
                    timeout.multiplier_code
                ));
            }
        }
        if let Some(counters) = self
            .integration
            .as_ref()
            .map(Integration::counters)
            .or(self.integration_counters)
        {
            ui.small(format!(
                "Integration samples {} · Queue drops {} · MQTT errors {} · Shared memory busy {}",
                counters.received,
                counters.queue_drops,
                counters.mqtt_errors,
                counters.shared_memory_busy
            ));
            if self.preferences.integration.mqtt.is_some() && self.integration.is_some() {
                ui.small(if counters.mqtt_connected {
                    "MQTT broker connected"
                } else {
                    "MQTT broker disconnected"
                });
            }
        }
        let input_age = self.connection.as_ref().map_or_else(
            || self.status.last_input.map(|stamp| stamp.elapsed()),
            Connection::input_age,
        );
        ui.label(format!(
            "Last input age {}  ·  Timeouts {}",
            input_age.map_or_else(
                || "—".into(),
                |age| format!("{:.2} ms", age.as_secs_f64() * 1000.0)
            ),
            self.status.timeouts
        ));
        if let Some(timeout) = self.status.receive_timeout {
            let phase = if timeout.waiting_for_first_input {
                "First input timeout"
            } else {
                "Receive watchdog"
            };
            ui.colored_label(
                AMBER,
                format!(
                    "{phase} · Valid input silent for {:.3} ms · Limit {:.3} ms",
                    timeout.valid_input_silence_us as f64 / 1000.0,
                    timeout.limit_us as f64 / 1000.0,
                ),
            );
            if let Some(age) = timeout.last_udp_age_us {
                ui.small(format!(
                    "Last UDP datagram at timeout · {:.3} ms old",
                    age as f64 / 1000.0
                ));
            }
            if let Some(sequence) = timeout.last_sequence {
                ui.small(format!("Last accepted packet sequence · {sequence}"));
            }
            ui.small(format!(
                "O->T sends during silence {} · Receive check gap {:.3} ms (longest {:.3} ms)",
                timeout.ot_sends_during_silence,
                timeout.last_check_gap_us as f64 / 1000.0,
                timeout.longest_check_gap_us as f64 / 1000.0
            ));
        }
        ui.label(format!(
            "Accepted {}  ·  Gap estimates {}  ·  Duplicates {}  ·  Late {}  ·  Invalid {}",
            self.status.valid,
            self.status.gaps,
            self.status.duplicates,
            self.status.late,
            self.status.invalid
        ));
        ui.label(format!(
            "Sent {}  ·  Skipped deadline slots {}",
            self.tx, self.skipped
        ));
        if self.tx != 0 {
            ui.small(format!(
                "Longest UDP send call · {} µs · {} calls at or above O->T interval",
                self.send_timing.max_send_call_us, self.send_timing.send_calls_at_or_over_rpi
            ));
            if let Some(us) = self.send_timing.max_dispatch_lateness_us {
                ui.small(format!("Maximum producer dispatch lateness · {us} µs"));
            }
            if let Some(applied) = self.send_timing.above_normal_applied {
                ui.small(if applied {
                    "Producer thread priority · Above normal"
                } else {
                    "Producer thread priority · Above-normal request unavailable"
                });
            }
        }
        for (label, s) in [
            ("T->O receive", self.status.spacing),
            ("O->T send", self.tx_spacing),
        ] {
            if s.intervals == 0 {
                continue;
            }
            ui.label(format!(
                "{label} spacing in µs  ·  p50 {}  ·  p95 {}  ·  p99 {}  ·  longest {}",
                s.p50_us, s.p95_us, s.p99_us, s.max_us
            ));
            ui.small(format!(
                "Observed span {:.3} s  ·  {} intervals ≥2× negotiated RPI  ·  {} ≥4×",
                s.span_us as f64 / 1_000_000.0,
                s.intervals_at_or_over_2x_rpi,
                s.intervals_at_or_over_4x_rpi
            ));
            if let (Some(rate), Some(percent)) =
                (s.observed_packets_per_second, s.percent_of_negotiated_rate)
            {
                ui.small(format!(
                    "{rate:.1} packets/s  ·  {percent:.1}% of negotiated rate"
                ));
            }
        }
    }

    pub(super) fn inputs(&mut self, ui: &mut Ui) {
        title(ui, "Live inputs");
        if let Some(v) = &self.status.value {
            ui.label(
                RichText::new(v.to_string())
                    .size(34.0)
                    .monospace()
                    .color(CYAN),
            );
        } else {
            ui.label("Waiting for an input sample");
        }
        if let Some(run) = self.status.input_run {
            ui.label(if run {
                "T->O header · Run"
            } else {
                "T->O header · Idle"
            });
        }
        if !self.status.input.is_empty() {
            if let Some(active) = &self.active {
                egui::ScrollArea::vertical()
                    .id_salt("mapped-input")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        egui::Grid::new("input-readback")
                            .striped(true)
                            .show(ui, |ui| {
                                for field in &active.capture_fields {
                                    ui.label(&field.name);
                                    ui.monospace(
                                        field
                                            .selection
                                            .decode(&self.status.input)
                                            .map_or_else(|| "—".into(), |value| value.to_string()),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            }
            ui.collapsing("Raw input bytes", |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("raw-input")
                    .max_height(116.0)
                    .show(ui, |ui| {
                        for (row, bytes) in self.status.input.chunks(16).enumerate() {
                            ui.monospace(format!("{:04X}  {}", row * 16, model::hex(bytes)));
                        }
                    });
            });
        }
        if ui.button("Edit input mappings").clicked() {
            self.tab = Tab::Mappings;
        }
    }
}
