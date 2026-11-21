use super::{AMBER, Explorer, InstancePage, Ipv4Addr, Response, Route, Ui, egui, explicit};

pub(super) enum Action {
    Select(u16),
    Next(u16),
}

impl Explorer {
    pub(super) fn find_instances(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        match self.inspection_request(adapter, local, &self.class) {
            Ok((adapter, local, class, route)) => {
                self.load_instances(adapter, local, class, 0, route)
            }
            Err(error) => self.notice = Some(error),
        }
    }

    pub(super) fn load_instances(
        &mut self,
        adapter: Ipv4Addr,
        local: Ipv4Addr,
        class: u32,
        after: u16,
        route: Option<Route>,
    ) {
        self.instance_page = None;
        self.start(move || {
            explicit::find_instances_page(adapter, local, class, after, route)
                .map(Response::InstancePage)
        });
    }
}

pub(super) fn show(ui: &mut Ui, page: &InstancePage, can_query: bool) -> Option<Action> {
    let mut action = None;
    egui::CollapsingHeader::new(format!("CIP instances for class 0x{:X}", page.class))
        .default_open(true)
        .show(ui, |ui| {
            ui.small(format!(
                "{} · Local NIC {}",
                page.adapter_or_bridge, page.local_ip
            ));
            if let Some(route) = &page.route {
                ui.small(format!("Route {route}"));
            }
            ui.small(format!("Instances after {}", page.after_instance));
            match page.decoded() {
                Ok(Some(decoded)) => {
                    if !decoded.ids.is_empty() {
                        let height = ui.spacing().interact_size.y;
                        egui::ScrollArea::vertical()
                            .id_salt(("cip-instance-rows", page.after_instance))
                            .max_height(190.0)
                            .show_rows(ui, height, decoded.ids.len(), |ui, rows| {
                                for row in rows {
                                    let id = decoded.ids[row];
                                    if ui
                                        .selectable_label(false, format!("Instance {id}"))
                                        .clicked()
                                    {
                                        action = Some(Action::Select(id));
                                    }
                                }
                            });
                    }
                    if let Some(after) = decoded.next_after {
                        if ui
                            .add_enabled(can_query, egui::Button::new("Next instances"))
                            .clicked()
                        {
                            action = Some(Action::Next(after));
                        }
                    } else {
                        ui.label("End of instance list");
                    }
                }
                Err(error) => {
                    ui.colored_label(AMBER, error);
                }
                Ok(None) => {}
            }
            match &page.reply {
                Ok(bytes) => {
                    ui.collapsing("Instance reply bytes", |ui| {
                        super::show_reply_payload(ui, bytes, &Ok(None))
                    });
                }
                Err(error) => {
                    ui.colored_label(AMBER, error.to_string());
                }
            }
        });
    action
}
