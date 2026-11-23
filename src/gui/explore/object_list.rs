use super::{ObjectList, Ui, batch, egui, explicit};

pub(super) fn show(ui: &mut Ui, list: &ObjectList) -> Option<u16> {
    let mut selected = None;
    egui::CollapsingHeader::new("Advertised CIP objects")
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                list.batch.adapter_or_bridge, list.batch.local_ip,
            ));
            if let Some(route) = &list.route {
                ui.small(format!("Route {route}"));
            }
            if let Some(ids) = &list.class_ids {
                if ids.is_empty() {
                    ui.label("The endpoint returned an empty Object List.");
                } else {
                    let height = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical()
                        .id_salt("object-list-rows")
                        .max_height(200.0)
                        .show_rows(ui, height, ids.len(), |ui, rows| {
                            for index in rows {
                                let id = ids[index];
                                let label = format!("0x{id:04X} · {}", class_name(u32::from(id)));
                                if ui.selectable_label(false, label).clicked() {
                                    selected = Some(id);
                                }
                            }
                        });
                }
            }
            for result in &list.batch.results {
                batch::show_result(ui, result);
            }
        });
    selected
}

fn class_name(class: u32) -> &'static str {
    match class {
        1 => "Identity",
        2 => "Message Router",
        explicit::ASSEMBLY_CLASS => "Assembly",
        explicit::CONNECTION_MANAGER_CLASS => "Connection Manager",
        explicit::DLR_CLASS => "Device Level Ring",
        explicit::QOS_CLASS => "Quality of Service",
        explicit::PORT_OBJECT_CLASS => "Port Object",
        explicit::TCPIP_INTERFACE_CLASS => "TCP/IP Interface",
        explicit::ETHERNET_LINK_CLASS => "Ethernet Link",
        explicit::LLDP_DATA_TABLE_CLASS => "LLDP Data Table",
        _ => "Class",
    }
}
