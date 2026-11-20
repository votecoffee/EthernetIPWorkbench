use super::*;

pub(super) fn temporary(name: &str) -> PathBuf {
    let mut id = [0; 8];
    getrandom::fill(&mut id).unwrap();
    std::env::temp_dir().join(format!("eipmaster-{name}-{:016x}", u64::from_le_bytes(id)))
}

#[test]
fn migrates_single_device_and_restores_two_independent_setups() {
    let directory = temporary("session");
    let path = directory.join("last-session.toml");
    fs::create_dir(&directory).unwrap();
    let mut legacy = Device::new(1, None);
    legacy.profiles = eds::parse(include_str!("../../samples/loopback.eds")).unwrap();
    legacy.settings.timeout_multiplier_code = 4;
    legacy.select_profile(Some(0));
    assert_eq!(legacy.settings.timeout_multiplier_code, 4);
    legacy.settings.input_rpi_us = 8_000;
    legacy.device = "in-progress address".into();
    legacy.local = "127.0.0.2".into();
    legacy.eds_path = "previous adapter.eds".into();
    legacy.toml_path = "saved profile.toml".into();
    legacy.preferences.rotate_seconds = 15;
    legacy.preferences.gui_capture_seconds = 10;
    legacy.preferences.schedule = Schedule::Tokio;
    let saved = legacy.snapshot();
    fs::write(&path, toml::to_string_pretty(&saved.setup).unwrap()).unwrap();

    let expected = {
        let mut app = App::new(Vec::new(), Some(path.clone()));
        assert_eq!(app.devices.len(), 1);
        assert_eq!(app.devices[0].snapshot(), saved);
        app.devices[0].name = "First adapter".into();
        app.devices[0].bench_confirmed = true;
        app.devices[0].writes_confirmed = true;
        app.devices[0]
            .output_entries
            .push("uncommitted output".into());
        app.add_device();
        let second = &mut app.devices[1];
        assert!(!second.bench_confirmed && !second.writes_confirmed);
        assert!(second.profiles.is_empty());
        second.name = "Second adapter".into();
        second.device = "127.0.0.3".into();
        second.preferences.directory = "second-capture".into();
        second.settings.input_rpi_us = 16_000;
        app.snapshot()
    };
    let restored = App::new(Vec::new(), Some(path.clone()));
    assert_eq!(restored.snapshot(), expected);
    assert_eq!(restored.selected, 1);
    for device in &restored.devices {
        assert!(!device.bench_confirmed && !device.writes_confirmed && !device.capturing);
        assert!(device.connection.is_none() && device.output_entries.is_empty());
    }
    drop(restored);
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}

#[test]
fn refuses_a_reserved_fixed_udp_port_before_connecting() {
    let mut device = Device::new(1, None);
    device.settings.local_udp_port = 2222;
    device.connect(&[(Ipv4Addr::LOCALHOST, 2222)]);
    assert!(device.connection.is_none());
    assert!(
        device
            .error
            .as_deref()
            .unwrap()
            .contains("already reserved")
    );
}

#[test]
fn automatic_reconnect_requires_an_identity_and_can_be_canceled() {
    let mut device = Device::new(1, None);
    device.preferences.auto_reconnect = true;
    device.connect(&[]);
    assert!(device.connection.is_none());
    assert!(device.error.as_deref().unwrap().contains("Read identity"));

    let pin = IdentityPin {
        key: (0, 12, 1),
        serial: 1,
    };
    let mut reconnect = Reconnect::new(pin);
    assert!(reconnect.schedule(Instant::now()));
    device.reconnect = Some(reconnect);
    assert!(device.link_busy());
    device.disconnect();
    device.poll();
    assert!(!device.link_busy());

    device.verified_identity = Some(pin);
    device.reset_endpoint_status();
    assert!(device.verified_identity.is_none());
}

#[test]
fn disk_failure_releases_only_the_failed_capture_slot() {
    let directory = temporary("capture-isolation");
    let mut healthy = Device::new(1, None);
    healthy.preferences.directory = directory.join("healthy").display().to_string();
    healthy.start_capture();
    let mut failing = Device::new(2, None);
    failing.preferences.directory = directory.join("failing").display().to_string();
    failing.preferences.rotate_bytes = 1;
    failing.start_capture();
    let failing_sink = failing.capture.as_ref().unwrap().sink.clone();
    let sentinel = PathBuf::from(format!("{}-000002.csv", failing_sink.prefix.display()));
    fs::write(&sentinel, "existing file").unwrap();
    for sequence in 1..=2 {
        failing_sink.record(Instant::now(), 1, sequence, sequence as u16, &[0; 4]);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while failing.capturing && Instant::now() < deadline {
        failing.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!failing.capturing && failing.slot.read().unwrap().is_none());
    assert!(failing_sink.error.lock().unwrap().is_some());
    assert!(healthy.capturing && healthy.slot.read().unwrap().is_some());
    let healthy_sink = healthy.capture.as_ref().unwrap().sink.clone();
    healthy_sink.record(Instant::now(), 1, 1, 1, &[0; 4]);
    healthy.stop_capture();
    assert_eq!(healthy_sink.written.load(Relaxed), 1);
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "existing file");
    let report_path = failing.save_run_report().unwrap();
    let report: toml::Table = toml::from_str(&fs::read_to_string(report_path).unwrap()).unwrap();
    assert!(report.get("connection").is_none());
    assert!(report["attempts"].as_array().unwrap().is_empty());
    assert!(report["capture"]["disk_error"].as_str().is_some());
    assert_eq!(report["capture"]["unconfirmed_rows"].as_integer(), Some(1));
    assert_eq!(report["capture"]["buffer_overruns"].as_integer(), Some(0));
    assert!(report["capture"].get("field_summaries").is_none());
    for name in ["healthy", "failing"] {
        for file in fs::read_dir(directory.join(name)).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(directory.join(name)).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}

#[test]
fn run_report_keeps_only_connections_that_belonged_to_the_capture() {
    let directory = temporary("run-report");
    let mut device = Device::new(1, None);
    device.preferences.directory = directory.display().to_string();
    device.settings.capture_mode = CaptureMode::OnChange;
    device.preferences.gui_capture_seconds = 60;
    device.start_capture();
    assert!(device.save_run_report().is_err());
    let sink = device.capture.as_ref().unwrap().sink.clone();
    device.capture_run.track(1);
    sink.record(Instant::now(), 1, 1, 1, &[0; 4]);
    sink.record(Instant::now(), 1, 2, 2, &[0; 4]);
    device.generation = 1;
    device.active = Some(Settings {
        device_ip: Ipv4Addr::new(127, 0, 0, 2),
        output_rpi_us: 1000,
        input_rpi_us: 1000,
        ..Settings::default()
    });
    device.status = Status {
        state: "Closed",
        actual: Some((1000, 1000)),
        valid: 2,
        ..Status::default()
    };
    device.record_capture_connection();

    device.capture_run.track(2);
    sink.record(Instant::now(), 2, 1, 1, &[0; 4]);
    device.stop_capture();
    device.generation = 2;
    device.status = Status {
        state: "Timed out",
        actual: Some((1250, 1500)),
        valid: 1,
        timeouts: 1,
        receive_timeout: Some(scanner::ReceiveTimeout {
            waiting_for_first_input: false,
            limit_us: 48_000,
            valid_input_silence_us: 48_100,
            last_udp_age_us: Some(48_100),
            last_sequence: Some(1),
            ot_sends_during_silence: 38,
            last_check_gap_us: 2_100,
            longest_check_gap_us: 2_300,
        }),
        ..Status::default()
    };
    device.record_capture_connection();

    device.generation = 3;
    device.preferences.schedule = Schedule::Tokio;
    device.status.state = "Closed";
    device.record_capture_connection();
    device.reset_endpoint_status();

    let path = device.save_run_report().unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        format!(
            "{}-report.toml",
            sink.prefix.file_name().unwrap().to_string_lossy()
        )
    );
    let text = fs::read_to_string(&path).unwrap();
    let report: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(report["mode"].as_str(), Some("gui"));
    let capture = &report["capture"];
    assert_eq!(capture["mode"].as_str(), Some("on_change"));
    assert_eq!(capture["duration_seconds"].as_integer(), Some(60));
    assert_eq!(capture["duration_reached"].as_bool(), Some(false));
    assert_eq!(capture["started_utc"].as_str(), Some(sink.start_utc()));
    assert_eq!(capture["csv_parts"].as_integer(), Some(1));
    assert_eq!(capture["first_armed_generation"].as_integer(), Some(1));
    assert_eq!(capture["last_armed_generation"].as_integer(), Some(2));
    assert_eq!(capture["seen"].as_integer(), Some(3));
    assert_eq!(capture["written"].as_integer(), Some(2));
    assert_eq!(capture["unchanged"].as_integer(), Some(1));
    let summary = &capture["field_summaries"].as_array().unwrap()[0];
    assert_eq!(summary["name"].as_str(), Some("value"));
    assert_eq!(summary["first_value"].as_str(), Some("0"));
    assert_eq!(summary["last_value"].as_str(), Some("0"));
    assert_eq!(summary["changes_within_generation"].as_integer(), Some(0));
    let connection = &report["connection"];
    let attempts = report["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["generation"].as_integer(), Some(1));
    assert_eq!(attempts[0]["state"].as_str(), Some("Closed"));
    assert_eq!(attempts[0]["accepted"].as_integer(), Some(2));
    assert_eq!(&attempts[1], connection);
    assert_eq!(connection["generation"].as_integer(), Some(2));
    assert_eq!(connection["adapter_ip"].as_str(), Some("127.0.0.2"));
    assert_eq!(connection["state"].as_str(), Some("Timed out"));
    assert_eq!(connection["schedule"].as_str(), Some("deadline"));
    assert_eq!(connection["requested_to_us"].as_integer(), Some(1000));
    assert_eq!(connection["actual_to_us"].as_integer(), Some(1500));
    assert_eq!(
        connection["receive_timeout"]["limit_us"].as_integer(),
        Some(48_000)
    );
    assert_eq!(
        connection["receive_timeout"]["ot_sends_during_silence"].as_integer(),
        Some(38)
    );
    assert!(device.save_run_report().is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    remove_capture_directory(directory);
}

fn remove_capture_directory(directory: PathBuf) {
    for file in fs::read_dir(&directory).unwrap() {
        fs::remove_file(file.unwrap().path()).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}

#[track_caller]
fn poll_until(device: &mut Device, ready: impl Fn(&Device) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        device.poll();
        if ready(device) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "device state {}",
            device.state_label()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode normal on localhost"]
fn timed_gui_capture_finishes_after_first_input_even_when_polling_is_delayed() {
    let directory = temporary("gui-timed-capture");
    let mut device = Device::new(1, None);
    device.preferences.directory = directory.display().to_string();
    device.preferences.gui_capture_seconds = 1;
    device.preferences.rotate_bytes = 2048;
    device.settings.input_rpi_us = 8000;
    device.settings.output_rpi_us = 8000;
    device.start_capture();
    let sink = device.capture.as_ref().unwrap().sink.clone();
    std::thread::sleep(Duration::from_millis(1050));
    device.poll();
    assert!(device.capturing && sink.deadline().is_none());

    device.connect(&[]);
    poll_until(&mut device, |_| sink.deadline().is_some());
    let remaining = sink
        .deadline()
        .unwrap()
        .saturating_duration_since(Instant::now());
    std::thread::sleep(remaining + Duration::from_millis(200));
    device.poll();
    assert!(!device.capturing && device.capture_run.duration_reached);
    assert!(device.link_busy());
    let written = sink.written.load(Relaxed);
    assert!(written > 100 && sink.part_count() > 1);
    assert_eq!(sink.seen.load(Relaxed), written);
    poll_until(&mut device, |device| device.status.valid > written);
    device.disconnect();
    poll_until(&mut device, |device| !device.link_busy());

    let path = device.save_run_report().unwrap();
    let report: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let capture = &report["capture"];
    assert_eq!(capture["duration_seconds"].as_integer(), Some(1));
    assert_eq!(capture["duration_reached"].as_bool(), Some(true));
    assert_eq!(capture["written"].as_integer(), Some(written as i64));
    let after_deadline = capture["after_capture_deadline"].as_integer().unwrap();
    assert!(after_deadline > 0);
    let link = &report["connection"];
    assert_eq!(link["forward_close"]["outcome"].as_str(), Some("confirmed"));
    let accepted = link["accepted"].as_integer().unwrap();
    assert!(accepted >= written as i64 + after_deadline);
    eprintln!("Timed GUI capture saved {written} rows of {accepted} connection packets");
    remove_capture_directory(directory);
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode qualify and a new EIP_GUI_QUALIFICATION_DIR"]
fn save_gui_qualification_reports_headlessly() {
    let directory = PathBuf::from(std::env::var_os("EIP_GUI_QUALIFICATION_DIR").unwrap());
    fs::create_dir(&directory).unwrap();
    for (name, rpi) in [
        ("baseline-8ms", 8000),
        ("trial-1", 1000),
        ("trial-2", 1000),
        ("trial-3", 1000),
    ] {
        let mut device = Device::new(1, None);
        device.preferences.directory = directory.join(name).display().to_string();
        device.preferences.gui_capture_seconds = 1;
        device.preferences.rotate_bytes = 4096;
        device.settings.input_rpi_us = rpi;
        device.settings.output_rpi_us = rpi;
        device.start_capture();
        device.connect(&[]);
        poll_until(&mut device, |device| !device.capturing);
        assert!(device.capture_run.duration_reached);
        device.disconnect();
        poll_until(&mut device, |device| !device.link_busy());
        assert!(device.save_run_report().unwrap().is_file());
    }
}

#[test]
fn pending_identity_blocks_connect_and_failed_refresh_keeps_the_pin() {
    let mut device = Device::new(1, None);
    let pin = IdentityPin {
        key: (0, 12, 1),
        serial: 0x12345678,
    };
    device.verified_identity = Some(pin);
    let (tx, rx) = mpsc::channel();
    device.identity_task = Some(rx);
    device.connect(&[]);
    assert!(device.connection.is_none());
    assert_eq!(device.generation, 0);
    assert_eq!(
        device.error.as_deref(),
        Some("Wait for the identity read to finish")
    );

    tx.send(Err("Fixture identity read failed".into())).unwrap();
    device.poll();
    assert!(device.identity_task.is_none());
    assert_eq!(device.verified_identity, Some(pin));
    assert_eq!(
        device.status.identity_error.as_deref(),
        Some("Fixture identity read failed")
    );
    device.reset_endpoint_status();
    assert!(device.verified_identity.is_none());
}

fn read_loopback_owner_identity() -> Device {
    let mut device = Device::new(1, None);
    let settings = &mut device.settings;
    settings.input_only = false;
    settings.output_instance = 100;
    settings.output_size = 2;
    settings.output_hex = "12 34".into();
    settings.input_run_idle = true;
    settings.output_run_idle = true;
    settings.config_size = 3;
    settings.config_hex = "AA BB CC".into();
    settings.input_rpi_us = 8000;
    settings.output_rpi_us = 8000;
    device.begin_identity_read();
    device.bench_confirmed = true;
    device.writes_confirmed = true;
    poll_until(&mut device, |device| device.identity_task.is_none());
    assert!(!device.bench_confirmed && !device.writes_confirmed);
    assert_eq!(
        device.verified_identity,
        Some(IdentityPin {
            key: (0, 12, 1),
            serial: 0x12345678,
        })
    );
    device
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode qualify-replacement on localhost"]
fn manual_pinned_connection_blocks_a_replacement_before_writes() {
    let mut device = read_loopback_owner_identity();
    device.writes_confirmed = true;
    device.connect(&[]);
    poll_until(&mut device, |device| !device.link_busy());
    assert_eq!(device.status.state, "Error");
    assert_eq!(device.status.identity.as_ref().unwrap().serial, 0x12345679);
    let error = device.status.identity_error.as_deref().unwrap();
    assert!(error.contains("Identity mismatch") && error.contains("12345678"));
    assert!(device.status.actual.is_none());
    assert_eq!(device.tx, 0);
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode identity-then-owner on localhost"]
fn matching_pin_allows_separately_authorized_owner_connection() {
    let mut device = read_loopback_owner_identity();
    device.connect(&[]);
    assert!(device.connection.is_none());
    assert!(
        device
            .error
            .as_deref()
            .unwrap()
            .contains("output permission")
    );

    device.writes_confirmed = true;
    device.connect(&[]);
    poll_until(&mut device, |device| device.status.valid >= 25);
    assert!(device.reconnect.is_none());
    assert!(!device.writes_confirmed);
    device.disconnect();
    poll_until(&mut device, |device| !device.link_busy());
    assert_eq!(device.status.state, "Closed");
    assert!(device.status.forward_close.as_ref().unwrap().is_confirmed());
}

fn exercise_variable_owner(max: u16, run_idle: bool) {
    let mut device = Device::new(1, None);
    let settings = &mut device.settings;
    settings.set_role(model::ConnectionRole::ExclusiveOwner);
    settings.output_instance = 100;
    settings.output_size = max;
    settings.output_min_size = Some(2);
    settings.output_hex = "10 20".into();
    settings.input_run_idle = true;
    settings.output_run_idle = run_idle;
    settings.large_forward_open = max > model::MAX_ASSEMBLY_BYTES as u16;
    settings.config_size = 3;
    settings.config_hex = "AA BB CC".into();
    settings.input_rpi_us = 8_000;
    settings.output_rpi_us = 8_000;

    device.connect(&[]);
    assert!(device.connection.is_none());
    assert!(
        device
            .error
            .as_deref()
            .unwrap()
            .contains("output permission")
    );

    device.writes_confirmed = true;
    device.connect(&[]);
    poll_until(&mut device, |device| device.status.valid >= 8);
    let connection = device.connection.as_ref().unwrap();
    assert!(connection.apply_output(vec![0]).is_err());
    assert!(
        connection
            .apply_output(vec![0; usize::from(max) + 1])
            .is_err()
    );
    assert_eq!(connection.output_bytes(), [0x10, 0x20]);

    for payload in [
        vec![0xA5; usize::from(max)],
        vec![0x5B; 5],
        vec![0xCA, 0xFE],
    ] {
        device.settings.output_hex = model::hex(&payload);
        let connection = device.connection.as_ref().unwrap();
        assert_ne!(connection.output_bytes(), payload);
        connection
            .apply_output(device.settings.output().unwrap())
            .unwrap();
        assert_eq!(connection.output_bytes(), payload);
        let target = device.status.valid + 8;
        poll_until(&mut device, |device| device.status.valid >= target);
    }
    assert_eq!(device.status.actual, Some((8_000, 8_000)));
    assert_eq!(
        device.status.invalid + device.status.gaps + device.status.timeouts,
        0
    );
    device.disconnect();
    poll_until(&mut device, |device| !device.link_busy());
    assert_eq!(device.status.state, "Closed");
    assert!(device.status.forward_close.as_ref().unwrap().is_confirmed());
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode variable-output on localhost"]
fn standard_open_resizes_cyclic_owner_payload_with_run_idle() {
    exercise_variable_owner(12, true);
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode large-variable-output on localhost"]
fn large_open_resizes_modeless_cyclic_owner_payload() {
    exercise_variable_owner(600, false);
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode different-api on localhost"]
fn run_report_uses_the_final_loopback_connection() {
    use std::{
        io::{Read, Write},
        net::{Ipv4Addr, TcpListener, TcpStream},
    };

    let directory = temporary("loopback-run-report");
    let mut device = Device::new(1, None);
    let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    device.preferences.integration.http_port = Some(port);
    device.preferences.directory = directory.display().to_string();
    device.settings.timeout_multiplier_code = 4;
    device.start_capture();
    device.connect(&[]);
    poll_until(&mut device, |device| device.status.valid >= 50);
    let get = |path: &str| -> serde_json::Value {
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(
            client,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    let body = get("/v1/latest");
    assert_eq!(body["active_generation"].as_u64(), Some(1));
    assert_eq!(body["sample"]["generation"].as_u64(), Some(1));
    assert!(body["sample"]["sample_index"].as_u64().unwrap() >= 50);
    let body = get("/v1/connection");
    assert_eq!(body["active_generation"].as_u64(), Some(1));
    assert!(body["previous_connection"].is_null());
    let connection = &body["connection"];
    assert_eq!(connection["state"].as_str(), Some("Receiving"));
    assert_eq!(connection["requested_to_us"].as_u64(), Some(1000));
    assert_eq!(connection["actual_to_us"].as_u64(), Some(1500));
    assert_eq!(
        connection["connection_timeout"]["receive_watchdog_us"].as_u64(),
        Some(96_000)
    );
    assert!(connection["input_age_us"].is_u64());
    device.disconnect();
    poll_until(&mut device, |device| !device.link_busy());
    device.stop_capture();
    let integration = device.integration_counters.unwrap();
    assert_eq!(integration.received, device.status.valid);
    assert_eq!(integration.queue_drops, 0);
    let path = device.save_run_report().unwrap();
    let report: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        report["capture"]["first_armed_generation"].as_integer(),
        Some(1)
    );
    assert_eq!(
        report["capture"]["last_armed_generation"].as_integer(),
        Some(1)
    );
    let link = &report["connection"];
    assert_eq!(
        report["attempts"].as_array().unwrap().as_slice(),
        std::slice::from_ref(link)
    );
    assert_eq!(link["state"].as_str(), Some("Closed"));
    assert_eq!(link["forward_close"]["outcome"].as_str(), Some("confirmed"));
    assert_eq!(link["requested_ot_us"].as_integer(), Some(1000));
    assert_eq!(link["requested_to_us"].as_integer(), Some(1000));
    assert_eq!(link["actual_ot_us"].as_integer(), Some(1250));
    assert_eq!(link["actual_to_us"].as_integer(), Some(1500));
    let io = &link["cyclic_io"];
    let local: std::net::SocketAddr = io["local_udp"].as_str().unwrap().parse().unwrap();
    assert!(local.ip().is_loopback() && local.port() != 0);
    assert_eq!(io["first_input_source"], io["output_destination"]);
    assert_eq!(io["ot_connection_id"].as_integer(), Some(0x1234ABCD));
    assert!(io["to_connection_id"].as_integer().unwrap() > 0);
    let startup = &link["startup_timing"];
    assert!(startup["forward_open_exchange_us"].is_integer());
    assert!(startup["first_send_after_open_us"].is_integer());
    assert_eq!(
        startup["first_udp_after_open_us"],
        startup["first_input_after_open_us"]
    );
    let timeout = &link["connection_timeout"];
    assert_eq!(timeout["multiplier_code"].as_integer(), Some(4));
    assert_eq!(
        timeout["requested_receive_watchdog_us"].as_integer(),
        Some(64_000)
    );
    assert_eq!(timeout["receive_watchdog_us"].as_integer(), Some(96_000));
    assert_eq!(
        link["receive_spacing"]["intervals"].as_integer(),
        Some(device.status.valid as i64 - 1)
    );
    assert_eq!(
        report["capture"]["written"].as_integer(),
        link["accepted"].as_integer()
    );
    assert_eq!(report["capture"]["unconfirmed_rows"].as_integer(), Some(0));
    remove_capture_directory(directory);
}

#[test]
#[ignore = "requires tools/loopback_adapter.py --mode recover on localhost"]
fn run_report_keeps_loopback_reconnect_attempts() {
    let directory = temporary("reconnect-run-report");
    let mut device = Device::new(1, None);
    device.preferences.directory = directory.display().to_string();
    device.preferences.rotate_bytes = 2048;
    device.preferences.auto_reconnect = true;
    device.settings.input_rpi_us = 8000;
    device.settings.output_rpi_us = 8000;
    device.verified_identity = Some(IdentityPin {
        key: (0, 12, 1),
        serial: 0x12345678,
    });
    device.start_capture();
    device.connect(&[]);
    poll_until(&mut device, |device| {
        device.generation == 2 && device.status.valid >= 50
    });
    device.disconnect();
    poll_until(&mut device, |device| !device.link_busy());
    device.stop_capture();

    let path = device.save_run_report().unwrap();
    let report: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let [first, restored] = report["attempts"].as_array().unwrap().as_slice() else {
        panic!("expected two connection attempts");
    };
    assert_eq!(first["generation"].as_integer(), Some(1));
    assert_eq!(first["state"].as_str(), Some("Timed out"));
    assert_eq!(
        first["receive_timeout"]["limit_us"].as_integer(),
        Some(256_000)
    );
    let timeout = &first["receive_timeout"];
    assert!(timeout["ot_sends_during_silence"].as_integer().unwrap() > 0);
    let last_gap = timeout["last_check_gap_us"].as_integer().unwrap();
    let longest_gap = timeout["longest_check_gap_us"].as_integer().unwrap();
    assert!(0 < last_gap && last_gap <= longest_gap);
    assert_eq!(restored["generation"].as_integer(), Some(2));
    assert_eq!(restored["state"].as_str(), Some("Closed"));
    assert!(restored.get("receive_timeout").is_none());
    assert_eq!(&report["connection"], restored);
    for attempt in [first, restored] {
        assert_eq!(attempt["actual_ot_us"].as_integer(), Some(8000));
        assert_eq!(attempt["actual_to_us"].as_integer(), Some(8000));
        assert_eq!(attempt["identity"]["serial"].as_integer(), Some(0x12345678));
        assert_eq!(
            attempt["forward_close"]["outcome"].as_str(),
            Some("confirmed")
        );
    }
    let capture = &report["capture"];
    let accepted =
        first["accepted"].as_integer().unwrap() + restored["accepted"].as_integer().unwrap();
    assert_eq!(capture["written"].as_integer(), Some(accepted));
    assert_eq!(capture["buffer_overruns"].as_integer(), Some(0));
    assert!(capture["csv_parts"].as_integer().unwrap() > 1);
    eprintln!("GUI report saved {accepted} rows across two attempts");
    remove_capture_directory(directory);
}

#[test]
fn refreshing_eds_keeps_the_effective_setup_and_staged_bytes() {
    let directory = temporary("refresh-eds");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("updated.eds");
    let base = include_str!("../../samples/loopback.eds")
        .replace("0x02010002", "0x04010002")
        .replace("0x22440305", "0x22440405")
        .replace("Param1,0,,", "Param1,2,,")
        .replace("    0,,", "    2,,")
        .replace("2C FE", "2C 66");
    let mut device = Device::new(1, None);
    device.profiles = eds::parse(&base).unwrap();
    device.select_profile(Some(0));
    device.eds_path = path.display().to_string();
    device.settings.input_rpi_us = 8_000;
    device.settings.output_hex = "12 34".into();
    device.settings.config_hex = "56 78".into();
    device.current().unwrap().validate().unwrap();
    device.preferences.rotate_seconds = 12;
    device.bench_confirmed = true;
    device.writes_confirmed = true;
    device.configuration_entries.push("123".into());
    let settings = device.settings.clone();
    let profiles = device.profiles.clone();

    fs::write(&path, base.replace("ProdCode = 1", "ProdCode = 2")).unwrap();
    device.refresh_eds();
    assert!(
        device
            .error
            .as_deref()
            .unwrap()
            .contains("different product code")
    );
    assert_eq!(device.profiles, profiles);
    assert_eq!(device.selected, Some(0));
    assert!(device.bench_confirmed && device.writes_confirmed);

    let (_, connection) = base.split_once("Connection1 =").unwrap();
    let revised = base
        .replace("MajRev = 1", "MajRev = 2")
        .replace("1000,1000000,1000", "2000,8000,4000")
        .replace(
            "Connection1 =",
            &format!("Connection2 ={connection}\nconnection1 ="),
        );
    fs::write(&path, revised).unwrap();
    device.refresh_eds();
    assert!(device.error.is_none(), "{:?}", device.error);
    assert_eq!(device.selected, Some(1));
    let imported = &device.profiles[1];
    assert_eq!(imported.device.as_ref().unwrap().major, Some(2));
    assert_eq!(
        imported.input_rpi_range.as_ref().unwrap().min_us,
        Some(2_000)
    );
    assert_eq!(imported.settings.input_rpi_us, 4_000);
    assert_eq!(device.settings, settings);
    assert_eq!(device.preferences.rotate_seconds, 12);
    assert!(!device.bench_confirmed && !device.writes_confirmed);
    assert!(device.configuration_entries.is_empty());
    assert!(device.connection.is_none());
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}

#[test]
fn named_profile_restores_imported_eds_and_preserves_overrides() {
    let directory = temporary("named-eds");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("profile.toml");
    let imported = eds::parse(include_str!("../../samples/loopback.eds"))
        .unwrap()
        .remove(0);
    let mut source = Device::new(1, None);
    source.profiles.push(imported.clone());
    source.select_profile(Some(0));
    source.eds_path = "missing-vendor-source.eds".into();
    source.toml_path = path.display().to_string();
    source.settings.input_rpi_us = 8_000;
    source.save_profile(None);
    assert!(source.error.is_none(), "{:?}", source.error);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("missing-vendor-source.eds"));

    let mut restored = Device::new(2, None);
    restored.toml_path.clone_from(&source.toml_path);
    restored.bench_confirmed = true;
    restored.writes_confirmed = true;
    restored.load_profile();
    assert!(restored.error.is_none(), "{:?}", restored.error);
    assert_eq!(restored.selected, Some(0));
    assert_eq!(restored.profiles, vec![imported]);
    assert_eq!(restored.settings.input_rpi_us, 8_000);
    assert_eq!(restored.profiles[0].settings.input_rpi_us, 1_000);
    assert!(restored.eds_path.is_empty());
    assert!(!restored.bench_confirmed && !restored.writes_confirmed);
    assert!(restored.verified_identity.is_none() && restored.connection.is_none());
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}

#[test]
fn named_profile_loads_preferences_and_requires_confirmation_to_replace() {
    let directory = temporary("profiles");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("profile.toml");
    let mut app = Device::new(1, None);
    app.toml_path = path.display().to_string();
    app.device = "127.0.0.2".into();
    app.settings.io_route = "1/2/2/127.0.0.3/1/0".into();
    app.preferences.capacity = 200;
    app.preferences
        .cip_reads
        .push(assembly_scanner::explicit::SavedRead {
            name: "Serial".into(),
            route: Some("1/3".parse().unwrap()),
            selection: Some(Selection {
                offset: 1,
                kind: Kind::U16,
                big_endian: true,
                ..Selection::default()
            }),
            path: assembly_scanner::explicit::ReadPath {
                class: 1,
                instance: 1,
                attribute: Some(6),
            },
        });
    app.save_profile(None);
    assert!(app.error.is_none());
    let original = fs::read(&path).unwrap();
    assert!(
        settings_file::load_named_profile(&path)
            .unwrap()
            .eds_profile
            .is_none()
    );
    app.preferences.capacity = 800;
    app.preferences.rotate_bytes = 4_096;
    app.save_profile(None);
    assert_eq!(app.replace_profile.as_deref(), Some(path.as_path()));
    assert_eq!(fs::read(&path).unwrap(), original);
    let confirmed = app.replace_profile.take();
    app.save_profile(confirmed);
    assert!(app.error.is_none());
    app.device = "127.0.0.3".into();
    app.preferences = Preferences::default();
    app.load_profile();
    assert!(app.error.is_none());
    assert_eq!(app.device, "127.0.0.2");
    assert_eq!(app.settings.io_route, "1/2/2/127.0.0.3/1/0");
    assert_eq!(app.preferences.capacity, 800);
    assert_eq!(app.preferences.rotate_bytes, 4_096);
    assert_eq!(app.preferences.cip_reads.len(), 1);
    assert_eq!(app.preferences.cip_reads[0].name, "Serial");
    assert_eq!(app.preferences.cip_reads[0].path.attribute, Some(6));
    let selection = app.preferences.cip_reads[0].selection.unwrap();
    assert_eq!(selection.offset, 1);
    assert_eq!(selection.kind, Kind::U16);
    assert!(selection.big_endian);
    assert_eq!(
        app.preferences.cip_reads[0]
            .route
            .as_ref()
            .unwrap()
            .to_string(),
        "1/3"
    );
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}
