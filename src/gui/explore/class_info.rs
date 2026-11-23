use super::*;

type InstanceRead = fn(&mut Explorer, &str, Option<Ipv4Addr>);
type InterfaceRead = fn(Ipv4Addr, Ipv4Addr, u32, Option<Route>) -> Result<TcpIpInfo, String>;

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
            Ok(explicit::TCPIP_INTERFACE_CLASS) => {
                ui.checkbox(
                    &mut self.tcpip_diagnostics,
                    "Include multicast and session settings",
                );
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Read address conflicts"))
                    .on_hover_text(
                        "Read the interface's address conflict flags and last recorded conflict.",
                    )
                    .clicked()
                {
                    self.read_tcpip_instance(
                        adapter,
                        local,
                        explicit::read_tcpip_address_conflicts,
                    );
                }
                (
                    "Read interface info",
                    "Read the selected TCP/IP Interface instance's reported network settings.",
                    Self::read_tcpip_info,
                )
            }
            Ok(explicit::ETHERNET_LINK_CLASS) => {
                ui.checkbox(
                    &mut self.link_port_details,
                    "Include port settings and capabilities",
                );
                (
                    "Read link info",
                    "Read the selected Ethernet Link instance's status and counters.",
                    Self::read_link_info,
                )
            }
            Ok(explicit::LLDP_DATA_TABLE_CLASS) => (
                "Read LLDP neighbor",
                "Read one neighbor recorded in the target's LLDP table.",
                Self::read_lldp_neighbor,
            ),
            Ok(explicit::DLR_CLASS) => {
                self.dlr_participants_action(ui, adapter, local);
                ui.checkbox(
                    &mut self.dlr_supervisor_details,
                    "Include supervisor details",
                );
                (
                    "Read ring status",
                    "Read Device Level Ring diagnostics from the selected instance.",
                    Self::read_dlr_info,
                )
            }
            Ok(explicit::TIME_SYNC_CLASS) => (
                "Read clock status",
                "Read the selected device's reported CIP Sync status and grandmaster details.",
                Self::read_time_sync,
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

    pub(super) fn inspection_request(
        &self,
        adapter: &str,
        local: Option<Ipv4Addr>,
        id: &str,
    ) -> Result<(Ipv4Addr, Ipv4Addr, u32, Option<Route>), String> {
        let id = explicit::parse_id(id)?;
        let (adapter, local, route) = self.inspection_target(adapter, local)?;
        Ok((adapter, local, id, route))
    }

    pub(super) fn inspection_target(
        &self,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) -> Result<(Ipv4Addr, Ipv4Addr, Option<Route>), String> {
        const NEED_ADDRESSES: &str = "Set adapter and local NIC IPv4 addresses in Device";
        let adapter = adapter.parse().map_err(|_| NEED_ADDRESSES)?;
        let local = local.ok_or(NEED_ADDRESSES)?;
        Ok((adapter, local, self.parsed_route()?))
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
        let read = if self.tcpip_diagnostics {
            explicit::read_tcpip_diagnostics
        } else {
            explicit::read_tcpip_info
        };
        self.read_tcpip_instance(adapter, local, read);
    }

    fn read_tcpip_instance(&mut self, adapter: &str, local: Option<Ipv4Addr>, read: InterfaceRead) {
        self.tcpip_info = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => {
                self.start(move || read(adapter, local, instance, route).map(Response::TcpIpInfo))
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_link_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.link_info = None;
        let read = if self.link_port_details {
            explicit::read_link_port_details
        } else {
            explicit::read_link_info
        };
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => {
                self.start(move || read(adapter, local, instance, route).map(Response::LinkInfo))
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_lldp_neighbor(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.lldp_neighbor = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_lldp_neighbor(adapter, local, instance, route)
                    .map(Response::LldpNeighbor)
            }),
            Err(error) => self.notice = Some(error),
        }
    }

    fn read_dlr_info(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.dlr_info = None;
        let supervisor_details = self.dlr_supervisor_details;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_dlr_info(adapter, local, instance, route, supervisor_details)
                    .map(Response::DlrInfo)
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
    if let Some(acd) = info.address_conflicts() {
        ui.horizontal_wrapped(|ui| {
            let show_flag = |value| match value {
                Some(true) => "Yes",
                Some(false) => "No",
                None => "Unavailable",
            };
            ui.label(format!(
                "ACD supported {} · Enabled {}",
                show_flag(acd.capable),
                show_flag(acd.enabled)
            ));
            ui.colored_label(
                if acd.conflict_detected == Some(true) || acd.fault == Some(true) {
                    AMBER
                } else {
                    ui.visuals().text_color()
                },
                format!(
                    "Conflict flag {} · Fault flag {}",
                    show_flag(acd.conflict_detected),
                    show_flag(acd.fault)
                ),
            );
        });
        show_inspection(
            ui,
            format!(
                "Address conflicts · TCP/IP Interface instance {}",
                info.instance
            ),
            "tcpip-acd-results",
            info.route.as_ref(),
            &info.batch,
        );
        return;
    }
    if info
        .diagnostics()
        .is_some_and(|diagnostics| diagnostics.multicast_pending == Some(true))
    {
        ui.colored_label(AMBER, "Multicast settings pending");
    }
    show_inspection(
        ui,
        format!("TCP/IP Interface instance {}", info.instance),
        "tcpip-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_link(ui: &mut Ui, info: &LinkInfo) {
    let mut title = format!("Ethernet Link instance {}", info.instance);
    if let Some(label) = info.port_details().and_then(|details| details.label)
        && !label.is_empty()
    {
        title.push_str(" · ");
        title.push_str(&label);
    }
    show_inspection(
        ui,
        title,
        "link-info-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_lldp_neighbor(ui: &mut Ui, info: &LldpNeighborReport) {
    let mut title = format!("LLDP neighbor {}", info.instance);
    if let Some(label) = info.neighbor().interface_label
        && !label.is_empty()
    {
        title.push_str(" · ");
        title.push_str(&label);
    }
    show_inspection(
        ui,
        title,
        "lldp-neighbor-results",
        info.route.as_ref(),
        &info.batch,
    );
}

pub(super) fn show_dlr(ui: &mut Ui, info: &DlrReport) {
    let mut title = format!("Device Level Ring instance {}", info.instance);
    if let Some(status) = info.network_status_label() {
        title.push_str(" · ");
        title.push_str(status);
    }
    show_inspection(
        ui,
        title,
        "dlr-info-results",
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

pub(super) fn show_profile_assemblies(ui: &mut Ui, report: &ProfileAssemblyCheck) {
    egui::CollapsingHeader::new("Profile assembly size check")
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                report.batch.adapter_or_bridge, report.batch.local_ip
            ));
            if let Some(route) = &report.route {
                ui.small(format!("Cyclic I/O route {route}"));
            }
            egui::Grid::new("profile-assembly-size-comparison")
                .striped(true)
                .show(ui, |ui| {
                    for heading in [
                        "Role",
                        "Instance",
                        "Profile bytes at check",
                        "Reported bytes",
                        "Result",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for check in &report.checks {
                        ui.label(check.role);
                        ui.label(check.instance.to_string());
                        if check.minimum_bytes == check.maximum_bytes {
                            ui.label(check.maximum_bytes.to_string());
                        } else {
                            ui.label(format!("{}–{}", check.minimum_bytes, check.maximum_bytes));
                        }
                        ui.label(
                            check
                                .declared_size_bytes
                                .map_or_else(|| "—".into(), |size| size.to_string()),
                        );
                        match check.matches_profile {
                            Some(true) => ui.colored_label(CYAN, "Within range"),
                            Some(false) => ui.colored_label(AMBER, "Size mismatch"),
                            None => ui.label("Unavailable"),
                        };
                        ui.end_row();
                    }
                });
            ui.collapsing("Assembly size replies", |ui| {
                for result in &report.batch.results {
                    batch::show_result(ui, result);
                }
            });
        });
}

pub(super) fn show_inspection(
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
