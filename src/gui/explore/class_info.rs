use super::*;

type InstanceRead = fn(&mut Explorer, &str, Option<Ipv4Addr>);

impl Explorer {
    pub(super) fn instance_action(&mut self, ui: &mut Ui, adapter: &str, local: Option<Ipv4Addr>) {
        let (label, description, read): (&str, &str, InstanceRead) = match explicit::parse_id(
            &self.class,
        ) {
            Ok(explicit::PORT_OBJECT_CLASS) => (
                "Read port info",
                "Read the Port Object directory at instance 0 or inspect a port instance.",
                Self::read_port_info,
            ),
            Ok(explicit::TCPIP_INTERFACE_CLASS) => (
                "Read interface info",
                "Read the selected TCP/IP Interface instance's reported network settings.",
                Self::read_tcpip_info,
            ),
            Ok(explicit::ETHERNET_LINK_CLASS) => (
                "Read link info",
                "Read the selected Ethernet Link instance's status and counters.",
                Self::read_link_info,
            ),
            Ok(explicit::CONNECTION_MANAGER_CLASS) => (
                "Read connection counters",
                "Read the selected Connection Manager's open, close, reject and timeout counters.",
                Self::read_manager_info,
            ),
            Ok(explicit::QOS_CLASS) => (
                "Read QoS settings",
                "Read 802.1Q tagging and DSCP settings reported by the selected QoS instance.",
                Self::read_qos_info,
            ),
            Ok(explicit::ASSEMBLY_CLASS) => (
                "Read assembly info",
                "Read the selected assembly's declared size and explicit data.",
                Self::read_assembly_info,
            ),
            _ => return,
        };
        if ui
            .add_enabled(!self.busy(), egui::Button::new(label))
            .on_hover_text(description)
            .clicked()
        {
            read(self, adapter, local);
        }
    }

    fn inspection_request(
        &self,
        adapter: &str,
        local: Option<Ipv4Addr>,
        id: &str,
    ) -> Result<(Ipv4Addr, Ipv4Addr, u32, Option<Route>), String> {
        const NEED_ADDRESSES: &str = "Set adapter and local NIC IPv4 addresses in Device";
        let adapter = adapter.parse().map_err(|_| NEED_ADDRESSES)?;
        let local = local.ok_or(NEED_ADDRESSES)?;
        Ok((
            adapter,
            local,
            explicit::parse_id(id)?,
            self.parsed_route()?,
        ))
    }

    pub(super) fn read_class_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.class_info = None;
        match self.inspection_request(adapter, local, &self.class) {
            Ok((adapter, local, class, route)) => self.start(move || {
                explicit::read_class_info(adapter, local, class, route).map(Response::ClassInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_port_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.port_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_port_info(adapter, local, instance, route).map(Response::PortInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_tcpip_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.tcpip_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_tcpip_info(adapter, local, instance, route).map(Response::TcpIpInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_link_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.link_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_link_info(adapter, local, instance, route).map(Response::LinkInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_manager_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.manager_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_manager_info(adapter, local, instance, route)
                    .map(Response::ManagerInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_qos_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.qos_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_qos_info(adapter, local, instance, route).map(Response::QosInfo)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_assembly_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.assembly_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_assembly_info(adapter, local, instance, route)
                    .map(Response::AssemblyInfo)
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

pub(super) fn show_tcpip(ui: &mut Ui, info: &TcpIpInfo) {
    show_inspection(
        ui,
        format!("TCP/IP Interface instance {}", info.instance),
        "tcpip-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_link(ui: &mut Ui, info: &LinkInfo) {
    show_inspection(
        ui,
        format!("Ethernet Link instance {}", info.instance),
        "link-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_manager(ui: &mut Ui, info: &ManagerInfo) {
    show_inspection(
        ui,
        format!("Connection Manager instance {}", info.instance),
        "manager-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_qos(ui: &mut Ui, info: &QosInfo) {
    show_inspection(
        ui,
        format!("Quality of Service instance {}", info.instance),
        "qos-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_assembly(ui: &mut Ui, info: &AssemblyInfo) {
    let mut title = format!("Assembly instance {}", info.instance);
    if info.sizes_match() == Some(false) {
        title.push_str(" · Size mismatch");
    }
    show_inspection(
        ui,
        title,
        "assembly-info-results",
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
