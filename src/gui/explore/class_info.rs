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

    pub(super) fn read_port_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.port_info = None;
        let request = || {
            let adapter = adapter
                .parse()
                .map_err(|_| "Set adapter and local NIC IPv4 addresses in Device")?;
            let local = local.ok_or("Set adapter and local NIC IPv4 addresses in Device")?;
            let instance = explicit::parse_id(&self.instance)?;
            Ok::<_, String>((adapter, local, instance, self.parsed_route()?))
        };
        match request() {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_port_info(adapter, local, instance, route).map(Response::PortInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }
}

pub(super) fn show(ui: &mut Ui, info: &ClassInfo) {
    show_inspection(
        ui,
        format!("Class 0x{:X} details", info.class),
        "class-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_port(ui: &mut Ui, info: &PortInfo) {
    let title = if info.instance == 0 {
        "Port Object directory".into()
    } else {
        format!("Port Object instance {}", info.instance)
    };
    show_inspection(
        ui,
        title,
        "port-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

fn show_inspection(
    ui: &mut Ui,
    title: String,
    id: &str,
    route: Option<&Route>,
    inspection: &explicit::SavedReadBatch,
) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                inspection.adapter_or_bridge, inspection.local_ip
            ));
            if let Some(route) = route {
                ui.small(format!("Route {route}"));
            }
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(240.0)
                .show(ui, |ui| {
                    for result in &inspection.results {
                        ui.push_id(result.request.path.attribute, |ui| {
                            batch::show_result(ui, result)
                        });
                    }
                });
        });
}
