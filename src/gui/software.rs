use super::*;

impl Device {
    pub(super) fn edit_integration(&mut self, ui: &mut Ui) {
        let settings = &mut self.preferences.integration;
        ui.small("Read-only feeds start when this device connects.");
        let mut http = settings.http_port.is_some();
        if ui.checkbox(&mut http, "Loopback HTTP API").changed() {
            settings.http_port = http.then_some(8765);
        }
        if let Some(port) = &mut settings.http_port {
            ui.horizontal(|ui| {
                ui.label("HTTP port");
                ui.add(egui::DragValue::new(port).range(1..=u16::MAX));
            });
        }
        let mut memory = settings.shared_memory.is_some();
        if ui.checkbox(&mut memory, "Windows shared memory").changed() {
            settings.shared_memory = memory.then(|| format!("device-{}", self.id));
        }
        if let Some(name) = &mut settings.shared_memory {
            ui.text_edit_singleline(name);
        }
        let mut mqtt = settings.mqtt.is_some();
        if ui.checkbox(&mut mqtt, "MQTT publish").changed() {
            settings.mqtt = mqtt.then(|| MqttOptions {
                broker: "127.0.0.1".into(),
                port: 1883,
                topic: format!("ethernetip/device-{}/samples", self.id),
                period_ms: 100,
            });
        }
        if let Some(mqtt) = &mut settings.mqtt {
            ui.small("Use a trusted MQTT listener or a local broker with a secure bridge.");
            ui.horizontal(|ui| {
                ui.label("Broker");
                ui.text_edit_singleline(&mut mqtt.broker);
                ui.add(egui::DragValue::new(&mut mqtt.port).range(1..=u16::MAX));
            });
            ui.horizontal(|ui| {
                ui.label("Topic");
                ui.text_edit_singleline(&mut mqtt.topic);
            });
            ui.horizontal(|ui| {
                ui.label("Publish period (ms)");
                ui.add(egui::DragValue::new(&mut mqtt.period_ms).range(1..=60_000));
            });
        }
        if let Err(error) = settings.validate() {
            ui.colored_label(AMBER, error);
        }
    }
}
