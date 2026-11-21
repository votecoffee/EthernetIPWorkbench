use super::*;
use assembly_scanner::{
    discovery::{self, ServicesTransport},
    explicit::{
        self, AssemblyInfo, AttributeWrite, Class3Size, ClassInfo, LinkInfo, ManagerInfo, PortInfo,
        ProfileAssemblyCheck, QosInfo, ReadPath, Reader, Route, SavedRead, SavedReadMode,
        SavedReadReport, TcpIpInfo, WriteCurrentCheck, WriteInspection, WriteOutcome, WriteReport,
    },
};
use std::sync::Arc;

mod batch;
mod class_info;
mod receipt;
mod services;
mod write;

const COMMON_PRESETS: [(&str, u32, u32); 7] = [
    ("Identity · Vendor ID", 1, 1),
    ("Identity · Device type", 1, 2),
    ("Identity · Product code", 1, 3),
    ("Identity · Revision", 1, 4),
    ("Identity · Status", 1, 5),
    ("Identity · Serial number", 1, 6),
    ("Identity · Product name", 1, 7),
];
const NETWORK_PRESETS: [(&str, u32, u32); 3] = [
    (
        "TCP/IP Interface · Status",
        explicit::TCPIP_INTERFACE_CLASS,
        1,
    ),
    ("Ethernet Link · Speed", explicit::ETHERNET_LINK_CLASS, 1),
    ("Quality of Service · Explicit DSCP", explicit::QOS_CLASS, 8),
];

enum Response {
    Discovery(Ipv4Addr, discovery::Report),
    Services(discovery::ServicesReport),
    Read(ReadReply),
    ClassInfo(ClassInfo),
    PortInfo(PortInfo),
    TcpIpInfo(TcpIpInfo),
    LinkInfo(LinkInfo),
    ManagerInfo(ManagerInfo),
    QosInfo(QosInfo),
    AssemblyInfo(AssemblyInfo),
    ProfileAssemblies(ProfileAssemblyCheck),
    Batch(SavedReadReport),
    SavedReport(PathBuf),
    OpenedWriteDraft(Box<write::Draft>),
    SavedWriteDraft(PathBuf),
    WriteIdentity(Identity),
    WriteInspection(Box<WriteInspection>),
    WriteResult(Arc<write::Receipt>),
    SavedWriteReceipt(PathBuf),
    CheckedWriteReceipt(Arc<WriteCurrentCheck>),
    OpenedReceiptFile(receipt::Opened),
    CheckedReceiptFile(Arc<WriteCurrentCheck>),
    SavedWriteCheck(receipt::Source, PathBuf),
}

struct ReadReply {
    address: String,
    interpretation: Result<Option<String>, String>,
    bytes: Vec<u8>,
}

pub(super) struct Explorer {
    destination: String,
    services_transport: ServicesTransport,
    services: Option<discovery::ServicesReport>,
    class: String,
    instance: String,
    attribute: String,
    route: String,
    name: String,
    selection: Option<Selection>,
    read_result: Option<ReadReply>,
    class_info: Option<ClassInfo>,
    port_info: Option<PortInfo>,
    tcpip_info: Option<TcpIpInfo>,
    link_info: Option<LinkInfo>,
    manager_info: Option<ManagerInfo>,
    qos_info: Option<QosInfo>,
    assembly_info: Option<AssemblyInfo>,
    profile_assemblies: Option<ProfileAssemblyCheck>,
    batch_mode: SavedReadMode,
    saved_batch: Option<Arc<SavedReadReport>>,
    saved_report: Option<PathBuf>,
    discovered: Option<(Ipv4Addr, discovery::Report)>,
    work: Option<mpsc::Receiver<Result<Response, String>>>,
    notice: Option<String>,
    write_draft: Option<write::Draft>,
    write_draft_path: String,
    write_result: Option<Arc<write::Receipt>>,
    saved_write_receipt: Option<PathBuf>,
    write_current_check: Option<receipt::FollowUp>,
    receipt_files: receipt::Panel,
}

impl Default for Explorer {
    fn default() -> Self {
        Self {
            destination: Ipv4Addr::BROADCAST.to_string(),
            services_transport: ServicesTransport::default(),
            services: None,
            class: "0x01".into(),
            instance: "1".into(),
            attribute: "7".into(),
            route: String::new(),
            name: String::new(),
            selection: None,
            read_result: None,
            class_info: None,
            port_info: None,
            tcpip_info: None,
            link_info: None,
            manager_info: None,
            qos_info: None,
            assembly_info: None,
            profile_assemblies: None,
            batch_mode: SavedReadMode::default(),
            saved_batch: None,
            saved_report: None,
            discovered: None,
            work: None,
            notice: None,
            write_draft: None,
            write_draft_path: String::new(),
            write_result: None,
            saved_write_receipt: None,
            write_current_check: None,
            receipt_files: receipt::Panel::default(),
        }
    }
}

impl Explorer {
    pub(super) fn busy(&self) -> bool {
        self.work.is_some()
    }

    fn start(&mut self, job: impl FnOnce() -> Result<Response, String> + Send + 'static) {
        let (send, receive) = mpsc::channel();
        self.notice = None;
        match std::thread::Builder::new()
            .name("eip-explore".into())
            .spawn(move || {
                let _ = send.send(job());
            }) {
            Ok(_) => self.work = Some(receive),
            Err(error) => self.notice = Some(error.to_string()),
        }
    }

    pub(super) fn poll(&mut self) {
        let Some(work) = &self.work else { return };
        match work.try_recv() {
            Ok(Ok(Response::Discovery(nic, report))) => self.discovered = Some((nic, report)),
            Ok(Ok(Response::Services(report))) => self.services = Some(report),
            Ok(Ok(Response::Read(reply))) => self.read_result = Some(reply),
            Ok(Ok(Response::ClassInfo(info))) => self.class_info = Some(info),
            Ok(Ok(Response::PortInfo(info))) => self.port_info = Some(info),
            Ok(Ok(Response::TcpIpInfo(info))) => self.tcpip_info = Some(info),
            Ok(Ok(Response::LinkInfo(info))) => self.link_info = Some(info),
            Ok(Ok(Response::ManagerInfo(info))) => self.manager_info = Some(info),
            Ok(Ok(Response::QosInfo(info))) => self.qos_info = Some(info),
            Ok(Ok(Response::AssemblyInfo(info))) => self.assembly_info = Some(info),
            Ok(Ok(Response::ProfileAssemblies(check))) => self.profile_assemblies = Some(check),
            Ok(Ok(Response::Batch(batch))) => self.saved_batch = Some(Arc::new(batch)),
            Ok(Ok(Response::SavedReport(path))) => self.saved_report = Some(path),
            Ok(Ok(Response::OpenedWriteDraft(draft))) => self.write_draft = Some(*draft),
            Ok(Ok(Response::SavedWriteDraft(path))) => {
                if let Some(draft) = &mut self.write_draft {
                    draft.saved_file = Some(path);
                }
            }
            Ok(Ok(Response::WriteIdentity(identity))) => {
                if let Some(draft) = &mut self.write_draft {
                    draft.identity = Some(identity);
                }
            }
            Ok(Ok(Response::WriteInspection(report))) => {
                if let Some(draft) = &mut self.write_draft {
                    draft.inspection = Some(*report);
                }
            }
            Ok(Ok(Response::WriteResult(result))) => {
                self.write_result = Some(result);
                self.saved_write_receipt = None;
                self.write_current_check = None;
            }
            Ok(Ok(Response::SavedWriteReceipt(path))) => self.saved_write_receipt = Some(path),
            Ok(Ok(Response::CheckedWriteReceipt(check))) => {
                self.write_current_check = Some(receipt::FollowUp::new(check));
            }
            Ok(Ok(Response::OpenedReceiptFile(receipt))) => {
                self.receipt_files.opened = Some(receipt)
            }
            Ok(Ok(Response::CheckedReceiptFile(check))) => {
                self.receipt_files.check = Some(receipt::FollowUp::new(check));
            }
            Ok(Ok(Response::SavedWriteCheck(source, path))) => {
                let check = match source {
                    receipt::Source::Current => &mut self.write_current_check,
                    receipt::Source::Reopened => &mut self.receipt_files.check,
                };
                if let Some(check) = check {
                    check.saved_path = Some(path);
                }
            }
            Ok(Err(error)) => self.notice = Some(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.notice = Some("Explore request ended without a response".into())
            }
            Err(mpsc::TryRecvError::Empty) => return,
        }
        self.work = None;
    }

    fn request(&self) -> Result<(ReadPath, Option<Route>), String> {
        let path = ReadPath {
            class: explicit::parse_id(&self.class)?,
            instance: explicit::parse_id(&self.instance)?,
            attribute: if self.attribute.trim().eq_ignore_ascii_case("all") {
                None
            } else {
                Some(explicit::parse_id(&self.attribute)?)
            },
        };
        path.validate()?;
        Ok((path, self.parsed_route()?))
    }

    fn parsed_route(&self) -> Result<Option<Route>, String> {
        if self.route.trim().is_empty() {
            Ok(None)
        } else {
            self.route.parse().map(Some)
        }
    }

    fn load(&mut self, path: ReadPath) {
        self.class = format!("0x{:X}", path.class);
        self.instance = path.instance.to_string();
        self.attribute = path
            .attribute
            .map_or("all".into(), |id| format!("0x{id:X}"));
    }

    fn load_preset(&mut self, name: &str, class: u32, instance: u32, attribute: u32) {
        self.load(ReadPath {
            class,
            instance,
            attribute: Some(attribute),
        });
        self.name = name.into();
        self.selection = None;
    }

    fn instance_one_presets(&mut self, ui: &mut Ui, presets: &[(&str, u32, u32)]) {
        for &(name, class, attribute) in presets {
            if ui.selectable_label(false, name).clicked() {
                self.load_preset(name, class, 1, attribute);
            }
        }
    }
}

impl Device {
    pub(super) fn explore(&mut self, ui: &mut Ui) {
        title(ui, "Explore EtherNet/IP");
        let can_change_adapter =
            !self.link_busy() && !self.capturing && self.identity_task.is_none();
        let local = self.local.parse::<Ipv4Addr>();
        let profile_addresses = self.addresses();
        let explorer = &mut self.explorer;
        ui.heading("Discover adapters");
        ui.label(format!("Local NIC · {} (set in Device)", self.local));
        ui.horizontal_wrapped(|ui| {
            ui.label("Destination IPv4");
            ui.text_edit_singleline(&mut explorer.destination);
            if ui
                .add_enabled(!explorer.busy(), egui::Button::new("Discover"))
                .clicked()
            {
                let parsed = local
                    .clone()
                    .and_then(|nic| explorer.destination.parse().map(|target| (nic, target)));
                match parsed {
                    Ok((nic, target)) => {
                        explorer.discovered = None;
                        explorer.start(move || {
                            discovery::discover(nic, target)
                                .map(|report| Response::Discovery(nic, report))
                                .map_err(|error| error.to_string())
                        });
                    }
                    Err(_) => {
                        explorer.notice =
                            Some("Enter valid NIC and destination IPv4 addresses".into())
                    }
                }
            }
        });
        let mut selected = None;
        if let Some((nic, report)) = &explorer.discovered {
            ui.small(format!(
                "{} replies on {nic} · {} invalid{}",
                report.devices.len(),
                report.invalid_replies,
                if report.limit_reached {
                    " · Result limit reached"
                } else {
                    ""
                }
            ));
            egui::ScrollArea::vertical()
                .id_salt("discovered")
                .max_height(160.0)
                .show(ui, |ui| {
                    for device in &report.devices {
                        ui.horizontal_wrapped(|ui| {
                            let id = &device.identity;
                            ui.label(format!(
                                "{} · {} · vendor:type:product {}:{}:{} · serial {:08X}",
                                device.source.ip(),
                                id.name,
                                id.vendor,
                                id.device_type,
                                id.code,
                                id.serial
                            ));
                            if ui
                                .add_enabled(
                                    can_change_adapter && local.as_ref() == Ok(nic),
                                    egui::Button::new("Use adapter"),
                                )
                                .clicked()
                            {
                                selected = Some(*device.source.ip());
                            }
                        });
                    }
                });
        }
        ui.separator();
        ui.heading("CIP object and attribute reads");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Adapter or bridge · {}", self.device));
            if ui
                .add_enabled(!explorer.busy(), egui::Button::new("List services"))
                .on_hover_text("Query the displayed endpoint's EtherNet/IP services before session registration.")
                .clicked()
            {
                explorer.services = None;
                match &profile_addresses {
                    Ok((adapter, nic)) => {
                        let (adapter, nic) = (*adapter, *nic);
                        let transport = explorer.services_transport;
                        explorer.start(move || {
                            discovery::list_services_with_transport(adapter, nic, transport)
                                .map(Response::Services)
                                .map_err(|error| error.to_string())
                        });
                    }
                    Err(error) => explorer.notice = Some(error.clone()),
                }
            }
            egui::ComboBox::from_id_salt("list-services-transport")
                .selected_text(explorer.services_transport.label())
                .show_ui(ui, |ui| {
                    for transport in [ServicesTransport::Udp, ServicesTransport::Tcp] {
                        ui.selectable_value(&mut explorer.services_transport, transport, transport.label());
                    }
                })
                .response
                .on_hover_text("ListServices transport");
        });
        if let Some(report) = &explorer.services {
            services::show(ui, report);
        }
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("cip-presets")
                .selected_text("Common attributes")
                .show_ui(ui, |ui| {
                    explorer.instance_one_presets(ui, &COMMON_PRESETS);
                    if ui.selectable_label(false, "Assembly · Data").clicked() {
                        explorer.load_preset(
                            "Input assembly data",
                            explicit::ASSEMBLY_CLASS,
                            u32::from(self.settings.input_instance),
                            3,
                        );
                    }
                });
            egui::ComboBox::from_id_salt("network-object-presets")
                .selected_text("Network objects")
                .show_ui(ui, |ui| explorer.instance_one_presets(ui, &NETWORK_PRESETS));
            egui::ComboBox::from_id_salt("port-object-presets")
                .selected_text("Port Object")
                .show_ui(ui, |ui| {
                    for (label, instance, attribute) in
                        [("Directory", 0, 8), ("Port instance", 1, 2)]
                    {
                        if ui.selectable_label(false, label).clicked() {
                            explorer.load_preset(
                                &format!("Port Object {label}"),
                                explicit::PORT_OBJECT_CLASS,
                                instance,
                                attribute,
                            );
                        }
                    }
                });
            if ui.button("Connection Manager").clicked() {
                explorer.load_preset(
                    "Connection Manager open requests",
                    explicit::CONNECTION_MANAGER_CLASS,
                    1,
                    1,
                );
            }
            if ui
                .add_enabled(
                    !explorer.busy(),
                    egui::Button::new("Check profile assembly sizes"),
                )
                .on_hover_text(
                    "Read declared sizes using the adapter, NIC, and cyclic I/O route in Device.",
                )
                .clicked()
            {
                explorer.profile_assemblies = None;
                match profile_addresses {
                    Ok((adapter, nic)) => {
                        let mut settings = self.settings.clone();
                        settings.device_ip = adapter;
                        settings.local_ip = nic;
                        explorer.start(move || {
                            explicit::check_profile_assemblies(&settings)
                                .map(Response::ProfileAssemblies)
                        });
                    }
                    Err(error) => explorer.notice = Some(error),
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("CIP route (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut explorer.route)
                    .desired_width(220.0)
                    .hint_text("1/3 for backplane slot 3"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            for (label, field) in [
                ("Class", &mut explorer.class),
                ("Instance", &mut explorer.instance),
                ("Attribute or all", &mut explorer.attribute),
            ] {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(field).desired_width(80.0));
            }
            if ui
                .add_enabled(!explorer.busy(), egui::Button::new("Read class info"))
                .on_hover_text("Read the selected class's common attributes at instance zero.")
                .clicked()
            {
                explorer.read_class_info(&self.device, local.as_ref().ok().copied());
            }
            explorer.instance_action(ui, &self.device, local.as_ref().ok().copied());
        });
        let mut typed = explorer.selection.is_some();
        if ui.checkbox(&mut typed, "Decode reply field").changed() {
            explorer.selection = typed.then_some(Selection::default());
        }
        if let Some(selection) = &mut explorer.selection {
            super::mapping::edit_selection(ui, selection);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!explorer.busy(), egui::Button::new("Read"))
                .clicked()
            {
                explorer.read_result = None;
                match (self.device.parse::<Ipv4Addr>(), &local, explorer.request()) {
                    (Ok(adapter), Ok(nic), Ok((path, route))) => {
                        let nic = *nic;
                        let selection = explorer.selection;
                        explorer.start(move || {
                            let target = route.as_ref().map_or_else(
                                || adapter.to_string(),
                                |route| format!("{adapter} via {route}"),
                            );
                            let address = format!(
                                "{target} · class 0x{:X} · instance 0x{:X} · attribute {}",
                                path.class,
                                path.instance,
                                path.attribute
                                    .map_or("all".into(), |id| format!("0x{id:X}"))
                            );
                            let mut reader = Reader::connect(adapter, nic)
                                .map_err(|error| format!("{address} · {error}"))?;
                            let bytes = match &route {
                                Some(route) => reader
                                    .read_routed(path, route)
                                    .map_err(|error| error.to_string()),
                                None => reader.read(path).map_err(|error| error.to_string()),
                            }
                            .map_err(|error| format!("{address} · {error}"))?;
                            Ok(Response::Read(ReadReply {
                                address,
                                interpretation: path.decode_reply(&bytes, selection),
                                bytes,
                            }))
                        });
                    }
                    (_, _, Err(error)) => explorer.notice = Some(error),
                    _ => {
                        explorer.notice =
                            Some("Set adapter and local NIC IPv4 addresses in Device".into())
                    }
                }
            }
            if ui
                .add_enabled(!explorer.busy(), egui::Button::new("Stage attribute write"))
                .clicked()
            {
                explorer.stage_attribute_write(
                    &self.device,
                    local.as_ref().ok().copied(),
                    self.preferences.cip_class3_size,
                );
            }
            ui.label("Save as");
            ui.add(egui::TextEdit::singleline(&mut explorer.name).desired_width(160.0));
            if ui.button("Save / update read").clicked() {
                match explorer.request() {
                    Ok((path, route)) => {
                        let name = explorer.name.trim().to_owned();
                        let mut reads = self.preferences.cip_reads.clone();
                        let read = SavedRead {
                            name,
                            path,
                            route,
                            selection: explorer.selection,
                        };
                        if let Some(prior) = reads.iter_mut().find(|prior| prior.name == read.name)
                        {
                            *prior = read;
                        } else {
                            reads.push(read);
                        }
                        match explicit::validate_saved_reads(&reads) {
                            Ok(()) => {
                                self.preferences.cip_reads = reads;
                                explorer.notice = None;
                            }
                            Err(error) => explorer.notice = Some(error),
                        }
                    }
                    Err(error) => explorer.notice = Some(error),
                }
            }
        });
        if let Some(info) = &explorer.class_info {
            class_info::show(ui, info);
        }
        if let Some(info) = &explorer.port_info {
            class_info::show_port(ui, info);
        }
        if let Some(info) = &explorer.tcpip_info {
            class_info::show_tcpip(ui, info);
        }
        if let Some(info) = &explorer.link_info {
            class_info::show_link(ui, info);
        }
        if let Some(info) = &explorer.manager_info {
            class_info::show_manager(ui, info);
        }
        if let Some(info) = &explorer.qos_info {
            class_info::show_qos(ui, info);
        }
        if let Some(info) = &explorer.assembly_info {
            class_info::show_assembly(ui, info);
        }
        if let Some(check) = &explorer.profile_assemblies {
            class_info::show_profile_assemblies(ui, check);
        }
        ui.collapsing("Saved reads", |ui| {
            if batch::controls(
                ui,
                &mut explorer.batch_mode,
                &mut self.preferences.cip_class3_size,
                &self.preferences.cip_reads,
                explorer.work.is_none(),
            ) {
                match (self.device.parse::<Ipv4Addr>(), &local) {
                    (Ok(adapter), Ok(nic)) => {
                        let nic = *nic;
                        let reads = self.preferences.cip_reads.clone();
                        let mode = explorer.batch_mode;
                        let size = self.preferences.cip_class3_size;
                        explorer.saved_batch = None;
                        explorer.saved_report = None;
                        explorer.start(move || {
                            explicit::run_saved_reads_with_class3_size(
                                adapter, nic, reads, mode, size,
                            )
                            .map(Response::Batch)
                        });
                    }
                    _ => {
                        explorer.notice =
                            Some("Set adapter and local NIC IPv4 addresses in Device".into())
                    }
                }
            }
            if let Some(batch) = &explorer.saved_batch
                && batch::show(
                    ui,
                    batch,
                    explorer.saved_report.as_deref(),
                    !explorer.busy(),
                )
            {
                let batch = Arc::clone(batch);
                let directory = PathBuf::from(self.preferences.directory.trim());
                explorer.saved_report = None;
                explorer.start(move || {
                    save_json_report(&directory, "saved-reads", &batch.report_json()?)
                        .map(Response::SavedReport)
                });
            }
            let mut remove = None;
            for (index, read) in self.preferences.cip_reads.iter().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(&read.name).clicked() {
                            explorer.name.clone_from(&read.name);
                            explorer.load(read.path);
                            explorer.selection = read.selection;
                            explorer.route = read
                                .route
                                .as_ref()
                                .map_or_else(String::new, Route::to_string);
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                    })
                });
            }
            if let Some(index) = remove {
                self.preferences.cip_reads.remove(index);
            }
        });
        explorer.show_open_write_draft(ui, self.preferences.cip_class3_size);
        explorer.show_write_receipts(ui, self.preferences.directory.trim());
        if explorer.busy() {
            ui.spinner();
        }
        if let Some(notice) = &explorer.notice {
            ui.colored_label(AMBER, notice);
        }
        if let Some(reply) = &explorer.read_result {
            ui.label(format!("{} · {} bytes", reply.address, reply.bytes.len()));
            show_reply_payload(ui, &reply.bytes, &reply.interpretation);
        }
        explorer.show_attribute_write(ui, self.preferences.directory.trim());
        if let Some(address) = selected {
            self.device = address.to_string();
            self.reset_endpoint_status();
            self.bench_confirmed = false;
            self.writes_confirmed = false;
        }
    }
}

fn save_json_report(directory: &Path, prefix: &str, json: &str) -> Result<PathBuf, String> {
    if directory.as_os_str().is_empty() {
        return Err("Set an output directory in Logging".into());
    }
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ");
    let path = directory.join(format!("{prefix}-{timestamp}.json"));
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    settings_file::write_new(&path, &format!("{json}\n"))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

fn show_reply_payload(ui: &mut Ui, bytes: &[u8], interpretation: &Result<Option<String>, String>) {
    match interpretation {
        Ok(Some(value)) => {
            ui.monospace(value);
        }
        Err(error) => {
            ui.colored_label(AMBER, error);
        }
        Ok(None) => {}
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical()
        .id_salt("cip-result")
        .max_height(200.0)
        .show_rows(ui, row_height, bytes.len().div_ceil(16), |ui, rows| {
            for row in rows {
                let start = row * 16;
                let end = (start + 16).min(bytes.len());
                ui.monospace(format!("{start:04X}  {}", model::hex(&bytes[start..end])));
            }
        });
}
