use super::{
    Explorer, Ipv4Addr, Response, TimeSyncReport, Ui, class_info::show_inspection, explicit,
};

impl Explorer {
    pub(super) fn read_time_sync(&mut self, adapter: &str, local: Option<Ipv4Addr>) {
        self.time_sync = None;
        match self.inspection_request(adapter, local, &self.instance) {
            Ok((adapter, local, instance, route)) => self.start(move || {
                explicit::read_time_sync(adapter, local, instance, route).map(Response::TimeSync)
            }),
            Err(error) => self.notice = Some(error),
        }
    }
}

pub(super) fn show(ui: &mut Ui, info: &TimeSyncReport) {
    let mut title = format!("CIP Sync instance {}", info.instance);
    if let Some(synchronized) = info.synchronized() {
        title.push_str(if synchronized {
            " · Reported synchronized"
        } else {
            " · Reported unsynchronized"
        });
    }
    show_inspection(
        ui,
        title,
        "time-sync-results",
        info.route.as_ref(),
        &info.batch,
    );
}
