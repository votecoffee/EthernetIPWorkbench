use super::{Explorer, Ipv4Addr, Ui, egui, explicit};
use std::collections::BTreeMap;

impl Explorer {
    pub(super) fn browse_attributes(
        &mut self,
        ui: &mut Ui,
        adapter: &str,
        local: Option<Ipv4Addr>,
    ) {
        let Ok(class @ 1..) = explicit::parse_id(&self.class) else {
            return;
        };
        let Ok(instance) = explicit::parse_id(&self.instance) else {
            return;
        };
        let selected = egui::ComboBox::from_id_salt("cip-attribute-browser")
            .selected_text("Browse attributes")
            .show_ui(ui, |ui| {
                let mut attributes = explicit::known_attributes(class, instance)
                    .iter()
                    .map(|&(id, name)| (id, (Some(name), false)))
                    .collect::<BTreeMap<_, _>>();
                if instance != 0
                    && let Some(info) = &self.class_info
                    && info.class == class
                    && adapter.parse() == Ok(info.batch.adapter_or_bridge)
                    && local == Some(info.batch.local_ip)
                    && self.parsed_route().is_ok_and(|route| route == info.route)
                    && let Some(ids) = info.optional_instance_attributes()
                {
                    for id in ids.into_iter().filter(|&id| id != 0) {
                        attributes.entry(u32::from(id)).or_default().1 = true;
                    }
                }
                let entries: Vec<_> = attributes.into_iter().collect();
                if entries.is_empty() {
                    ui.label("Read class info or enter a documented attribute ID.");
                    return None;
                }
                let mut selected = None;
                let height = ui.spacing().interact_size.y;
                egui::ScrollArea::vertical()
                    .id_salt("cip-attribute-browser-rows")
                    .max_height(180.0)
                    .show_rows(ui, height, entries.len(), |ui, rows| {
                        for index in rows {
                            let (id, (name, advertised)) = entries[index];
                            let label = match (name, advertised) {
                                (Some(name), true) => format!("0x{id:X} · {name} · Advertised"),
                                (Some(name), false) => format!("0x{id:X} · {name}"),
                                (None, _) => format!("0x{id:X} · Advertised"),
                            };
                            if ui.selectable_label(false, label).clicked() {
                                selected = Some((id, name));
                            }
                        }
                    });
                selected
            })
            .inner
            .flatten();
        if let Some((id, name)) = selected {
            self.attribute = format!("0x{id:X}");
            self.name = name.map_or_else(|| format!("Attribute 0x{id:X}"), str::to_owned);
            self.selection = None;
        }
    }
}
