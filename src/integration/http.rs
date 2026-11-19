use super::{Feed, IDLE, counters};
use crate::scanner::DiagnosticsSource;
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::Ordering::Relaxed},
    thread,
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_BATCH: usize = 64;

struct TrackedConnection {
    generation: u64,
    source: DiagnosticsSource,
}

#[derive(Default)]
pub(super) struct Connections {
    current: Option<Arc<TrackedConnection>>,
    previous: Option<Arc<TrackedConnection>>,
}

impl Connections {
    pub(super) fn track(&mut self, generation: u64, source: DiagnosticsSource) {
        self.previous = self
            .current
            .replace(Arc::new(TrackedConnection { generation, source }));
    }
}

fn connection_json(feed: &Feed, tracked: &TrackedConnection) -> Value {
    let diagnostics = tracked.source.diagnostics();
    let status = &diagnostics.status;
    let (actual_ot_us, actual_to_us) = status.actual.unzip();
    json!({
        "generation": tracked.generation,
        "state": status.state,
        "identity": status.identity,
        "requested_ot_us": diagnostics.requested_ot_us,
        "requested_to_us": diagnostics.requested_to_us,
        "actual_ot_us": actual_ot_us,
        "actual_to_us": actual_to_us,
        "input_age_us": tracked.source.input_age().map(|age| age.as_micros() as u64),
        "accepted": status.valid,
        "sequence_gaps": status.gaps,
        "duplicates": status.duplicates,
        "late": status.late,
        "invalid": status.invalid,
        "timeouts": status.timeouts,
        "sent": diagnostics.sent,
        "skipped_deadlines": diagnostics.skipped,
        "receive_spacing": status.spacing,
        "send_spacing": diagnostics.send_spacing,
        "send_timing": diagnostics.send_timing,
        "startup_timing": status.startup_timing(diagnostics.send_timing),
        "connection_timeout": feed.settings.connection_timeout(actual_to_us),
        "receive_timeout": status.receive_timeout,
        "forward_close": status.forward_close,
        "cyclic_io": status.cyclic_io,
        "identity_error": status.identity_error,
        "cip_error": status.cip_error,
        "network_error": status.network_error,
    })
}

fn connections_json(feed: &Feed) -> Value {
    let (current, previous, active) = {
        let connections = feed
            .http_connections
            .as_ref()
            .expect("HTTP feed")
            .lock()
            .unwrap();
        (
            connections.current.clone(),
            connections.previous.clone(),
            feed.active_generation.load(Relaxed),
        )
    };
    json!({
        "schema_version": 1,
        "stream_start_utc_us": feed.started_utc_us,
        "active_generation": active,
        "adapter_ip": feed.settings.device_ip,
        "local_ip": feed.settings.local_ip,
        "io_route": (!feed.settings.io_route.trim().is_empty()).then_some(&feed.settings.io_route),
        "connection": current.as_deref().map(|tracked| connection_json(feed, tracked)),
        "previous_connection": previous.as_deref().map(|tracked| connection_json(feed, tracked)),
    })
}

pub(super) fn serve(feed: &Feed, listener: TcpListener) {
    let port = listener.local_addr().expect("bound listener").port();
    let mut hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if port == 80 {
        hosts.extend(["127.0.0.1".into(), "localhost".into()]);
    }
    while !feed.stop.load(Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = respond(feed, stream, &hosts);
            }
            Err(_) => thread::park_timeout(IDLE),
        }
    }
}

fn respond(feed: &Feed, mut stream: TcpStream, hosts: &[String]) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut request = [0; 2048];
    let mut used = 0;
    let response = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if used == request.len() || remaining.is_zero() {
            return Ok(());
        }
        stream.set_read_timeout(Some(remaining))?;
        let length = stream.read(&mut request[used..])?;
        if length == 0 {
            return Ok(());
        }
        used += length;
        if let Some(end) = request[..used]
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
        {
            break route(feed, &request[..end], hosts);
        }
    };
    let body = response.1.to_string();
    let headers = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.0,
        body.len()
    );
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    for mut bytes in [headers.as_bytes(), body.as_bytes()] {
        while !bytes.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            stream.set_write_timeout(Some(remaining))?;
            let written = stream.write(bytes)?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            bytes = &bytes[written..];
        }
    }
    Ok(())
}

fn route(feed: &Feed, request: &[u8], hosts: &[String]) -> (&'static str, Value) {
    let bad_request = || ("400 Bad Request", json!({"error": "invalid request"}));
    let Ok(request) = std::str::from_utf8(request) else {
        return bad_request();
    };
    let mut lines = request.split("\r\n");
    let mut words = lines.next().unwrap_or_default().split_ascii_whitespace();
    let (Some(method), Some(target), Some("HTTP/1.0" | "HTTP/1.1"), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return bad_request();
    };
    let mut host = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return bad_request();
        };
        if name.eq_ignore_ascii_case("host") && host.replace(value.trim()).is_some() {
            return bad_request();
        }
    }
    if !host.is_some_and(|value| hosts.iter().any(|host| host.eq_ignore_ascii_case(value))) {
        return bad_request();
    }
    if method != "GET" {
        return ("405 Method Not Allowed", json!({"error": "GET only"}));
    }
    let generation = feed.active_generation.load(Relaxed);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/v1/status" if query.is_empty() => (
            "200 OK",
            json!({
                "schema_version": 1,
                "stream_start_utc_us": feed.started_utc_us,
                "active_generation": generation,
                "counters": counters(feed),
            }),
        ),
        "/v1/connection" if query.is_empty() => ("200 OK", connections_json(feed)),
        "/v1/latest" if query.is_empty() => {
            let sample = feed.latest();
            (
                "200 OK",
                json!({
                    "schema_version": 1,
                    "active_generation": generation,
                    "sample_age_us": sample.map(|sample| (feed.origin.elapsed().as_micros() as u64).saturating_sub(sample.elapsed_us)),
                    "sample": sample.map(|sample| feed.frame_json(sample)),
                }),
            )
        }
        "/v1/samples" => {
            let after = if query.is_empty() {
                0
            } else if let Some(index) = query
                .strip_prefix("after=")
                .and_then(|index| index.parse::<u64>().ok())
            {
                index
            } else {
                return bad_request();
            };
            let (oldest, latest, batch) = {
                let history = feed.history.lock().unwrap();
                (
                    history.front().map(|frame| frame.index),
                    history.back().map(|frame| frame.index),
                    history
                        .iter()
                        .filter(|frame| frame.index > after)
                        .take(MAX_BATCH)
                        .copied()
                        .collect::<Vec<_>>(),
                )
            };
            (
                "200 OK",
                json!({
                    "schema_version": 1,
                    "stream_start_utc_us": feed.started_utc_us,
                    "active_generation": generation,
                    "oldest_sample_index": oldest,
                    "latest_sample_index": latest,
                    "samples": batch.into_iter().map(|frame| feed.frame_json(frame)).collect::<Vec<_>>(),
                }),
            )
        }
        _ => ("404 Not Found", json!({"error": "unknown endpoint"})),
    }
}
