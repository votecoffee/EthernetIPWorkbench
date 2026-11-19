use super::{diagnostics::Histogram, *};

fn request_above_normal() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: The current thread pseudo-handle stays valid for the call.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) != 0 }
    }
    #[cfg(not(windows))]
    false
}

fn publish_send_status(shared: &Shared, spacing: &Histogram, timing: SendTiming) {
    *shared.tx_spacing.lock().unwrap() = spacing.snapshot();
    *shared.send_timing.lock().unwrap() = timing;
}

pub(super) fn produce(
    udp: &UdpSocket,
    peer: SocketAddr,
    cfg: &Settings,
    reply: &ForwardOpenResponse,
    open_completed: Instant,
    shared: &Shared,
    schedule: Schedule,
) -> Result<()> {
    let period = Duration::from_micros(reply.o_to_t_actual_rpi_us.into());
    let lengths = cfg.output_lengths().map_err(protocol)?;
    let data_start = 20 + 4 * usize::from(cfg.output_run_idle);
    let mut packet = vec![0; data_start + lengths.end()];
    packet[0..6].copy_from_slice(&[2, 0, 2, 0x80, 8, 0]);
    packet[6..10].copy_from_slice(&reply.o_to_t_connection_id.to_le_bytes());
    packet[14..16].copy_from_slice(&0x00b1u16.to_le_bytes());
    if cfg.output_run_idle {
        packet[20] = 1;
    }
    let mut sequence = 0u32;
    let mut spacing = Histogram::new(reply.o_to_t_actual_rpi_us);
    let mut last = None;
    let mut publish = Instant::now();
    let mut timing = SendTiming {
        above_normal_applied: (schedule == Schedule::DeadlineAboveNormal)
            .then(request_above_normal),
        ..SendTiming::default()
    };
    let mut send = |lateness: Option<Duration>| -> Result<()> {
        if let Some(lateness) = lateness {
            let us = lateness.as_micros() as u64;
            timing.max_dispatch_lateness_us =
                Some(timing.max_dispatch_lateness_us.unwrap_or(0).max(us));
        }
        sequence = sequence.wrapping_add(1);
        packet[10..14].copy_from_slice(&sequence.to_le_bytes());
        packet[18..20].copy_from_slice(&(sequence as u16).to_le_bytes());
        let end = {
            let output = shared.output.lock().unwrap();
            if !lengths.contains(&output.len()) {
                return Err(protocol(
                    "Cyclic output buffer is outside the configured size range",
                ));
            }
            let end = data_start + output.len();
            packet[data_start..end].copy_from_slice(&output);
            end
        };
        let connected_len = (end - 18) as u16;
        packet[16..18].copy_from_slice(&connected_len.to_le_bytes());
        let call_start = Instant::now();
        let result = udp.send_to(&packet[..end], peer);
        let stamp = Instant::now();
        let call_time = stamp - call_start;
        timing.max_send_call_us = timing.max_send_call_us.max(call_time.as_micros() as u64);
        timing.send_calls_at_or_over_rpi += u64::from(call_time >= period);
        result?;
        timing
            .first_send_after_open_us
            .get_or_insert_with(|| (stamp - open_completed).as_micros() as u64);
        if let Some(prior) = last.replace(stamp) {
            spacing.add(stamp - prior);
        }
        if stamp - publish >= Duration::from_millis(100) {
            publish_send_status(shared, &spacing, timing);
            publish = stamp;
        }
        shared.tx.fetch_add(1, Relaxed);
        Ok(())
    };
    if schedule == Schedule::Tokio {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;
        let result = runtime.block_on(async {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while !shared.stop.load(Relaxed) {
                tokio::select! {
                    _ = interval.tick() => if !shared.stop.load(Relaxed) { send(None)?; },
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
            }
            Ok(())
        });
        publish_send_status(shared, &spacing, timing);
        return result;
    }
    let mut waiter = crate::deadline_wait::DeadlineWait::new();
    let mut deadline = Instant::now();
    let spin = Duration::from_micros(150).min(period / 4);
    let result = loop {
        if shared.stop.load(Relaxed) {
            break Ok(());
        }
        let now = Instant::now();
        if now < deadline {
            let remaining = deadline - now;
            if remaining > spin {
                waiter.wait((remaining - spin).min(Duration::from_millis(10)));
            } else {
                std::hint::spin_loop();
            }
            continue;
        }
        let lateness = now - deadline;
        let missed = lateness.as_nanos() / period.as_nanos();
        if missed != 0 {
            shared.skipped.fetch_add(missed as u64, Relaxed);
        }
        deadline =
            now + (period - Duration::from_nanos((lateness.as_nanos() % period.as_nanos()) as u64));
        if let Err(error) = send(Some(lateness)) {
            break Err(error);
        }
    };
    publish_send_status(shared, &spacing, timing);
    result
}

fn parse_packet<'packet>(
    packet: &'packet [u8],
    connection: u32,
    header: bool,
    lengths: &std::ops::RangeInclusive<usize>,
) -> Option<(u32, u16, Option<bool>, &'packet [u8])> {
    let start = 20 + 4 * usize::from(header);
    if !lengths.contains(&packet.len().checked_sub(start)?)
        || packet[..6] != [2, 0, 2, 0x80, 8, 0]
        || le32(packet, 6) != connection
        || le16(packet, 14) != 0xb1
        || le16(packet, 16) as usize != packet.len() - 18
    {
        return None;
    }
    Some((
        le32(packet, 10),
        le16(packet, 18),
        header.then(|| le32(packet, 20) & 1 != 0),
        &packet[start..],
    ))
}
#[cfg(windows)]
fn wait_for_udp(udp: &UdpSocket) -> io::Result<bool> {
    use std::{os::windows::io::AsRawSocket, ptr::null_mut};
    use windows_sys::Win32::Networking::WinSock::{
        FD_SET, SOCKET_ERROR, TIMEVAL, WSAGetLastError, select,
    };

    let mut readable = FD_SET {
        fd_count: 1,
        ..FD_SET::default()
    };
    readable.fd_array[0] = udp.as_raw_socket() as usize;
    let timeout = TIMEVAL {
        tv_sec: 0,
        tv_usec: 2_000,
    };
    // SAFETY: The arguments stay valid for this call, and the borrowed socket stays open.
    let result = unsafe { select(0, &mut readable, null_mut(), null_mut(), &timeout) };
    if result == SOCKET_ERROR {
        Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
    } else {
        Ok(result != 0)
    }
}

fn receive_datagram(udp: &UdpSocket, packet: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
    // Wait for readiness so an idle interval never cancels a Windows recvfrom.
    #[cfg(windows)]
    if !wait_for_udp(udp)? {
        return Ok(None);
    }
    match udp.recv_from(packet) {
        Ok(packet) => Ok(Some(packet)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn receive(
    udp: &UdpSocket,
    cfg: &Settings,
    reply: &ForwardOpenResponse,
    open_completed: Instant,
    slot: &Slot,
    generation: u64,
    shared: &Shared,
) -> Result<()> {
    let started = Instant::now();
    let first_timeout = Duration::from_secs(2).max(Duration::from_micros(
        u64::from(reply.t_to_o_actual_rpi_us) * 2,
    ));
    let watchdog = Duration::from_micros(
        cfg.connection_timeout(Some(reply.t_to_o_actual_rpi_us))
            .receive_watchdog_us
            .expect("validated timeout code and actual input interval"),
    );
    let mut packet = [0; 65_536];
    let lengths = cfg.input_lengths().map_err(protocol)?;
    let mut last_seq: Option<u32> = None;
    let mut last_udp = None;
    let mut sent_at_last_input = 0;
    let mut previous_check = started;
    let mut longest_check_gap = Duration::ZERO;
    let mut seen = 0u64;
    let mut status = shared.status.lock().unwrap().clone();
    let mut spacing = Histogram::new(reply.t_to_o_actual_rpi_us);
    let mut publish = started;
    let result = loop {
        if shared.stop.load(Relaxed) {
            break Ok(());
        }
        let result = receive_datagram(udp, &mut packet);
        let stamp = Instant::now();
        let check_gap = stamp - previous_check;
        previous_check = stamp;
        longest_check_gap = longest_check_gap.max(check_gap);
        match result {
            Ok(Some((length, addr))) => {
                if last_udp.replace(stamp).is_none() {
                    status.first_udp_after_open_us =
                        Some((stamp - open_completed).as_micros() as u64);
                }
                match parse_packet(
                    &packet[..length],
                    reply.t_to_o_connection_id,
                    cfg.input_run_idle,
                    &lengths,
                )
                .filter(|_| addr.ip() == cfg.device_ip)
                {
                    Some((seq, cip, run, bytes)) => {
                        let delta = last_seq.map_or(1, |previous| seq.wrapping_sub(previous));
                        if delta == 0 {
                            status.duplicates += 1;
                        } else if delta >= 0x8000_0000 {
                            let distance = last_seq.unwrap().wrapping_sub(seq);
                            if distance < 64 && seen & (1 << distance) != 0 {
                                status.duplicates += 1;
                            } else {
                                status.late += 1;
                            }
                        } else {
                            status.gaps += u64::from(delta - 1);
                            last_seq = Some(seq);
                            seen = seen.checked_shl(delta).unwrap_or(0) | 1;
                            if let Some(last) = status.last_input {
                                spacing.add(stamp - last);
                            }
                            status.last_input = Some(stamp);
                            sent_at_last_input = shared.tx.load(Relaxed);
                            longest_check_gap = Duration::ZERO;
                            let first = status.valid == 0;
                            if first {
                                status.first_input_after_open_us =
                                    Some((stamp - open_completed).as_micros() as u64);
                                if let Some(cyclic_io) = &mut status.cyclic_io {
                                    cyclic_io.first_input_source = Some(addr);
                                }
                            }
                            shared.input_times.record(stamp, first);
                            status.state = "Receiving";
                            status.valid += 1;
                            status.input.clear();
                            status.input.extend_from_slice(bytes);
                            status.input_run = run;
                            if let Some(publisher) = &shared.publisher {
                                publisher.record(stamp, generation, seq, cip, bytes);
                            }
                            if let Some(sink) = &*slot.read().unwrap() {
                                sink.record(stamp, generation, seq, cip, bytes);
                            }
                        }
                    }
                    None => status.invalid += 1,
                }
            }
            Ok(None) => {}
            Err(e) => break Err(e.into()),
        }
        let (since, limit) = status
            .last_input
            .map_or((started, first_timeout), |last| (last, watchdog));
        let silence = stamp - since;
        if silence >= limit {
            status.receive_timeout = Some(ReceiveTimeout {
                waiting_for_first_input: status.last_input.is_none(),
                limit_us: limit.as_micros() as u64,
                valid_input_silence_us: silence.as_micros() as u64,
                last_udp_age_us: last_udp.map(|last| (stamp - last).as_micros() as u64),
                last_sequence: last_seq,
                ot_sends_during_silence: shared.tx.load(Relaxed) - sent_at_last_input,
                last_check_gap_us: check_gap.as_micros() as u64,
                longest_check_gap_us: longest_check_gap.as_micros() as u64,
            });
            status.timeouts += 1;
            status.retryable = true;
            status.state = "Timed out";
            break Ok(());
        }
        if stamp - publish >= Duration::from_millis(100) {
            status.spacing = spacing.snapshot();
            publish_receive_status(shared, &mut status, cfg.selection);
            publish = stamp;
        }
    };
    status.spacing = spacing.snapshot();
    publish_receive_status(shared, &mut status, cfg.selection);
    result
}

fn publish_receive_status(shared: &Shared, status: &mut Status, selection: model::Selection) {
    status.value = selection.decode(&status.input).map(Value::into_owned);
    let mut latest = shared.status.lock().unwrap();
    status.identity.clone_from(&latest.identity);
    status.identity_error.clone_from(&latest.identity_error);
    latest.clone_from(status);
}
