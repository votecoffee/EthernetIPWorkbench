use super::*;

impl Explorer {
    pub(super) fn read_class_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.class_info = None;
        let request = || {
            let adapter = adapter
                .parse()
                .map_err(|_| "Set adapter and local NIC IPv4 addresses in Device")?;
            let local = local.ok_or("Set adapter and local NIC IPv4 addresses in Device")?;
            let class = explicit::parse_id(&self.class)?;
            Ok::<_, String>((adapter, local, class, self.parsed_route()?))
        };
        match request() {
            Ok((adapter, local, class, route)) => self.start(move || {
                explicit::read_class_info(adapter, local, class, route).map(Response::ClassInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }
}

pub(super) fn show(ui: &mut Ui, info: &ClassInfo) {
    egui::CollapsingHeader::new(format!("Class 0x{:X} details", info.class))
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                info.batch.adapter_or_bridge, info.batch.local_ip
            ));
            if let Some(route) = &info.route {
                ui.small(format!("Route {route}"));
            }
            egui::ScrollArea::vertical()
                .id_salt("class-info-results")
                .max_height(240.0)
                .show(ui, |ui| {
                    for result in &info.batch.results {
                        ui.push_id(result.request.path.attribute, |ui| {
                            batch::show_result(ui, result)
                        });
                    }
                });
        });
}
