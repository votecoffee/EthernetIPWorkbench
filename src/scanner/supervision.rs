use super::{Connection, Diagnostics, ExpectedIdentity, Reconnect, Schedule, Status};
use crate::{capture::Slot, integration::Publisher, model::Settings};
use std::{
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub struct InputRunOptions {
    pub expected: ExpectedIdentity,
    pub schedule: Schedule,
    pub auto_reconnect: bool,
}

/// Run an identity-checked read-only session with bounded reconnects. The
/// caller owns the recorder and publisher across generations. A true stop
/// predicate requests Forward_Close and waits for the final diagnostics.
///
/// Returned attempts are in generation order, starting at 1. A failure to
/// start a retry appears as its own error attempt.
pub fn run_input(
    settings: &Settings,
    options: InputRunOptions,
    slot: &Slot,
    publisher: Option<Publisher>,
    mut should_stop: impl FnMut() -> bool,
) -> Result<Vec<Diagnostics>, String> {
    let auto_reconnect = options.auto_reconnect && settings.supports_auto_reconnect();
    let mut attempts = Vec::new();
    let mut reconnect = None;
    let mut expected = options.expected;

    'session: while !should_stop() {
        let connection = match Connection::start_with_publisher(
            settings.clone(),
            slot.clone(),
            attempts.len() as u64 + 1,
            options.schedule,
            false,
            Some(expected),
            publisher.clone(),
        ) {
            Ok(connection) => connection,
            Err(error) if attempts.is_empty() => return Err(error),
            Err(error) => {
                attempts.push(Diagnostics {
                    requested_ot_us: settings.output_rpi_us,
                    requested_to_us: settings.input_rpi_us,
                    status: Status {
                        state: "Error",
                        network_error: Some(error),
                        ..Status::default()
                    },
                    sent: 0,
                    skipped: 0,
                    send_spacing: Default::default(),
                    send_timing: Default::default(),
                });
                break;
            }
        };
        while !connection.finished() {
            if should_stop() {
                connection.stop();
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }

        let result = connection.finish();
        if auto_reconnect && attempts.is_empty() && result.status.identity_error.is_none() {
            reconnect = result
                .status
                .identity
                .as_ref()
                .map(|id| Reconnect::new(id.pin()));
        }
        let retryable = result.status.retryable;
        attempts.push(result);
        if should_stop() || !retryable {
            break;
        }
        let Some(reconnect) = reconnect.as_mut() else {
            break;
        };
        if !reconnect.schedule(Instant::now()) {
            break;
        }
        loop {
            if should_stop() {
                break 'session;
            }
            if let Some(pin) = reconnect.take_ready(Instant::now()) {
                expected = ExpectedIdentity::Device(pin);
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
    Ok(attempts)
}
