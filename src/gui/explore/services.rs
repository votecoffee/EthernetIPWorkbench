use super::{Ui, discovery::ServicesReport, egui};

pub(super) fn show(ui: &mut Ui, report: &ServicesReport) {
    egui::CollapsingHeader::new("Advertised EtherNet/IP services")
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                report.adapter_ip, report.local_ip
            ));
            ui.small(format!("Transport {}", report.transport.label()));
            if report.services.is_empty() {
                ui.label("The endpoint returned an empty service list.");
            }
            egui::ScrollArea::vertical()
                .id_salt("list-services-items")
                .max_height(180.0)
                .show(ui, |ui| {
                    let yes_no = |value| if value { "Yes" } else { "No" };
                    for (index, item) in report.services.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            let name = if let Some(info) = &item.communications {
                                ui.label(format!(
                                    "{} · Protocol version {} · Flags 0x{:04X}",
                                    info.name, info.protocol_version, info.capability_flags
                                ));
                                ui.label(format!("CIP over TCP · {}", yes_no(info.cip_over_tcp)));
                                ui.label(format!(
                                    "Cyclic I/O over UDP · {}",
                                    yes_no(info.cyclic_io_over_udp)
                                ));
                                "Raw item"
                            } else {
                                "Unrecognized item"
                            };
                            ui.collapsing(format!("{name} · 0x{:04X}", item.type_id), |ui| {
                                ui.monospace(&item.data_hex);
                            });
                        });
                    }
                });
        });
}
