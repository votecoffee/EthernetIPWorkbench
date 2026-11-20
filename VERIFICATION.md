# Assembly Scanner 0.1.1 verification

## Windows build and upstream review

The package was built and run on Windows 11 Pro x64 build 26200 with an Intel Core Ultra 7 155H, using Rust 1.99.1 and the x64 MSVC toolchain. The PE header identifies a Windows GUI x64 executable. The import table uses Windows system DLLs and OpenGL. The application test suite passed five tests. The published ethernetip-connections 0.1.1 crate passed its 27 unit tests on the same Windows host.

- The published scanner uses Tokio TCP and UDP. It registers a session, opens a cyclic Class 1 point-to-point connection, uses the returned O→T actual interval for production, and exchanges CPF sequenced address and connected data items. Its producer uses a Tokio interval with missed ticks set to Skip. Its receiver updates a shared assembly snapshot and has no public per-packet timestamp/sequence callback. The scanner source also uses one Run/Idle flag in setup and decodes incoming data without an input Run/Idle header. [Published scanner source](https://docs.rs/crate/ethernetip-connections/0.1.1/source/src/scanner.rs)
- The application uses the published Forward_Open and Forward_Close codecs and the core TCP encapsulation and CPF codecs. Its connection worker binds both transports to the selected local IPv4 address. The independent OpENer source identifies 0x8000 as O→T Sockaddr Info and 0x8001 as T→O. Its fixed Class 1 connection size includes the two-byte sequence count. These wire details differ from the crate's stock scanner. [OpENer CPF constants](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/enet_encap/cpf.h), [OpENer I/O connection source](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipioconnection.c), [ODVA CPF wire overview](https://www.odva.org/wp-content/uploads/2020/06/PUB00123R1_Common-Industrial_Protocol_and_Family_of_CIP_Networks.pdf)
- The prototype has separate producer, receive and CSV writer threads. The deadline producer uses absolute monotonic deadlines, a sleep with a short spin guard, and skips missed slots. The Tokio baseline uses the stock interval policy on the same application packet path and socket. This comparison isolates the scheduler policy. The stock scanner's full stack was evaluated from source and unit tests.

## Simulator timing

The independent Python adapter and the real Windows GUI ran on the same host over localhost, with CSV capture and three-second rotation active. Each run was about 12 seconds. Both directions requested 1,000 µs in both runs and Forward_Open returned 1,000 µs for each direction. All spacing below is in microseconds.

| Scheduler | Measurement point | p50 | p95 | p99 | Longest |
|---|---|---:|---:|---:|---:|
| Deadline | Scanner O→T send | 1,022 | 1,153 | 1,424 | 2,547 |
| Deadline | Fixture O→T receive | 1,019 | 2,020 | 2,170 | 3,498 |
| Deadline | Scanner T→O receive | 1,006 | 1,176 | 1,372 | 2,473 |
| Tokio Skip | Scanner O→T send | 1,029 | 1,639 | 2,123 | 3,142 |
| Tokio Skip | Fixture O→T receive | 1,023 | 2,115 | 2,544 | 3,434 |
| Tokio Skip | Scanner T→O receive | 1,004 | 1,225 | 1,466 | 2,786 |

| Scheduler | Scanner O→T sent / fixture received | Fixture T→O sent / scanner accepted / CSV written | Sequence gaps O→T / T→O | CSV overruns |
|---|---:|---:|---:|---:|
| Deadline | 11,893 / 11,893 | 11,897 / 11,897 / 11,897 | 0 / 0 | 0 |
| Tokio Skip | 11,889 / 11,889 | 11,880 / 11,880 / 11,880 | 0 / 0 | 0 |

Both runs had zero duplicates, late packets, invalid frames, timeouts and disk errors. The scanner deadline loop skipped 5 scheduled send slots. The Tokio baseline reports no missed-slot count. The fixture producer skipped 2 slots during the deadline run and 9 during the Tokio run. Skipped scheduler slots occur before a packet sequence is assigned and are reported independently from sequence gaps. Timing varied across runs and process load. In these runs, the deadline policy lowered scanner send p99 by approximately 33%.

## Windows wait-handle reuse

The deadline producer on Windows now reuses one high-resolution waitable timer per producer. It keeps the same monotonic deadline, final spin guard, and missed-slot policy. If the dedicated timer is unavailable, the producer uses the standard sleep path. The Tokio baseline is unchanged. Rust's Windows sleep already attempts a high-resolution timer and creates one per call. [Rust Windows sleep source](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/thread/windows.rs), [Windows high-resolution timer API](https://learn.microsoft.com/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw)

The previous build (`55fa1c0`) and reused-handle build ran three trials each, in alternating pair order, on the same Windows host. The independent localhost fixture, GUI, and rotating CSV writer were active for about 12 seconds per trial. Both directions requested and negotiated 1,000 µs in all six trials. Each cell shows the range of the three per-run measurements in microseconds.

| Build | Measurement point | p50 | p95 | p99 | Longest |
|---|---|---:|---:|---:|---:|
| Previous | Scanner O→T send | 1,025–1,029 | 1,191–1,231 | 1,481–1,538 | 2,548–7,528 |
| Reused handle | Scanner O→T send | 1,021–1,039 | 1,168–1,234 | 1,458–1,473 | 2,573–14,472 |
| Previous | Fixture O→T receive | 1,023–1,028 | 2,037–2,084 | 2,220–2,312 | 3,164–7,627 |
| Reused handle | Fixture O→T receive | 1,020–1,034 | 2,037–2,088 | 2,236–2,293 | 3,544–15,126 |
| Previous | Fixture T→O send | 1,008–1,014 | 1,202–1,231 | 1,418–1,515 | 2,475–6,933 |
| Reused handle | Fixture T→O send | 1,008–1,017 | 1,174–1,217 | 1,418–1,452 | 2,535–15,281 |
| Previous | Scanner T→O receive | 1,008–1,014 | 1,202–1,240 | 1,419–1,520 | 2,481–6,900 |
| Reused handle | Scanner T→O receive | 1,008–1,016 | 1,181–1,217 | 1,422–1,451 | 2,538–15,194 |

| Build | Scanner skipped send slots | Fixture skipped send slots | Sequence gaps O→T / T→O | CSV rows | CSV overruns | Timeouts |
|---|---:|---:|---:|---:|---:|---:|
| Previous | 86 | 123 | 0 / 2 | 35,532 | 0 | 0 |
| Reused handle | 46 | 36 | 0 / 0 | 35,538 | 0 | 0 |

All runs closed cleanly. The median per-run scanner send p99 changed from 1,535 to 1,464 µs. The candidate's longest send gap was worse in one trial, when the fixture also recorded a long send gap. This comparison qualifies the offline implementation only. [Physical bench acceptance](https://github.com/votecoffee/EthernetIPWorkbench/issues/2) remains open.

## Offline functional checks

- The Rust tests exercised signed, floating-point and bit decoding, endian order and out-of-range selection, plus EDS profile parsing, unresolved paths, and safe single-byte ParamN path resolution.
- An eight-sample ring and an intentionally delayed writer accepted eight of 100 offered samples, counted 92 overruns and flushed the eight accepted rows across size-rotated parts on stop. A forced collision at the second time-rotated part left the original file intact, raised a disk error and reported one flushed row out of two seen.
- The GUI fixture checked rotation, strict CSV parsing, repeated values and sample-index continuity. Packet fault injection exercised a skipped sequence, duplicate, delayed sequence and invalid payload. The stall case counted a receive timeout and performed Forward_Close. Forward_Open rejection preserved CIP general status 0x01 and extended status 0x0109 in the connection error field.
- With requests of 1,000 / 1,000 µs, the fixture also returned different intervals of 1,250 / 1,500 µs for O→T / T→O. The app reported the returned values separately and used them for its cyclic timing.
- A loopback-only exclusive-owner test checked the padded configuration segment, Run/Idle framing in each direction, and the transition between two staged output buffers. The reconnect test kept one CSV capture across two connections and verified the generation change with contiguous sample indexes.

Historical simulator reports were bundled with the original development packages. The portable preview includes this summary and its build manifest. Device-specific captures stay outside the package.

## Concurrent adapter sessions

The Windows GUI ran two independent localhost fixtures in a single process. The adapters used distinct serial numbers, UDP endpoints, and input values. The healthy adapter requested 8,000 µs in both directions and the stalled adapter requested 16,000 µs. Each fixture returned the same intervals in Forward_Open.

| Stream | End state | CSV rows / parts | T→O sequence gaps | CSV overruns |
|---|---|---:|---:|---:|
| Healthy | Closed | 1,476 / 4 | 1 | 0 |
| Stalled | Timed out | 75 / 1 | 0 | 0 |

A repeat wrote 1,483 healthy and 75 stalled rows with zero sequence gaps or overruns on either stream.

Both fixtures received Forward_Close. Each stream had contiguous CSV sample indexes and every accepted sample was written to the correct stream. The captures overlapped and the healthy stream recorded for more than five seconds after the stalled stream's last sample. The receive timeout was counted only on the stalled adapter. These results cover simulated input-only sessions.

```powershell
python tools/benchmark.py .\target\release\assembly-scanner.exe .\.local\dual-adapter-check --dual-only
```

## Headless recovery

Three concurrent Windows CLI captures used separate localhost fixtures at an 8 ms RPI in both directions. Forward_Open returned 8 ms for both directions. One fixture stalled and resumed after its first reconnect delay. A second fault was injected by placing an existing file at the disk stream's next CSV part.

| Stream | Result | CSV rows | Timeouts | T→O sequence gaps | CSV overruns |
|---|---|---:|---:|---:|---:|
| Healthy | Completed throughout both faults | 752 | 0 | 0 | 0 |
| Recovered | Completed across generations 1 and 2 | 468 | 1 | 0 | 0 |
| Disk fault | Reported the collision and exited incomplete | 250 | 0 | 0 | 0 |

A repeat recorded one sequence gap on the healthy stream and zero on the recovered stream. Both completed with zero overruns.

The recovered stream retained its UTC start and contiguous sample indexes. The disk stream saw 252 rows and confirmed 250 written. The existing file was preserved. A separate fixture changed its serial number after a timeout. The scanner preserved the 75 previously captured rows and rejected the new identity before Forward_Open. Rust tests cover the bounded retry delays, GUI retry cancellation, and releasing a failed GUI capture slot while another writer continues.

```powershell
python tools/test_recovery.py .\target\release\eipmaster-cli.exe .\.local\recovery-check
```

## OpENer compatibility

The Windows CLI captured from the upstream OpENer sample adapter, built with MSVC from revision `0e1ab7ee`. The test launcher binds all adapter sockets to 127.0.0.2, supplies a synthetic MAC address, and fills the 32-byte input assembly with values 0 through 31. The adapter uses the upstream Class 1 stack and sample connection points. [Upstream sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds) · [Upstream sample application](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/ports/WIN32/sample_application/sampleapplication.c)

EDS Connection2 resolves to input assembly 100, heartbeat point 152, and configuration instance 151 with zero configuration bytes. Both directions requested 30,000 µs and Forward_Open returned 30,000 µs in both directions. The Identity Object reported vendor 1, type 12, product 65001, revision 2.3, and serial 123456789. The target trace recorded Forward_Close.

The first 60-second run wrote all 2,000 accepted samples with one receive sequence gap, zero duplicates, zero invalid packets, zero timeouts, and zero CSV overruns. Socket spacing was measured separately.

| Direction | p50 µs | p95 µs | p99 µs | Longest µs |
|---|---:|---:|---:|---:|
| Scanner O→T send | 29,988 | 30,486 | 35,967 | 46,692 |
| Scanner T→O receive | 30,412 | 31,886 | 32,435 | 60,705 |

The reproducible run with rotation wrote all 1,989 accepted samples across six CSV parts. It reported 12 receive sequence gaps and zero CSV overruns, duplicates, invalid packets or timeouts. OpENer logged 2,001 sends with no socket send error. This comparison does not identify where the missing datagrams were dropped. The receive p50, p95, p99 and longest gap were 30,726, 31,980, 32,895 and 62,853 µs, respectively.

This check exposed the previous 20,000 µs percentile ceiling. The fixed histogram retains exact microsecond buckets through 32,767 µs and reports a bounded upper estimate for longer intervals. CSV elapsed times independently reproduced the receive percentiles within that resolution.

To repeat the headless test, install CMake and the Visual Studio 2022 C++ toolchain. Use a fresh output directory and a full upstream Git checkout. The tool builds a local copy of the pinned revision and keeps the upstream EDS, logs and CSV parts in the test directory.

```powershell
git clone https://github.com/EIPStackGroup/OpENer.git .local/opener-upstream
python tools/test_opener.py .\target\release\eipmaster-cli.exe .\.local\opener-upstream .\.local\opener-check
```

## Headless bench sequence

The CLI `qualify` command preserves each trial's capture report and CSV alongside the effective profiles and a cumulative receipt. The short loopback check exercised the 8 ms baseline and all three 1 ms trials. A fixture that returned 1,250 and 1,500 µs on one target trial produced a failed qualification receipt and still ran the remaining trials. An interval mismatch on the baseline stopped before the target trials. A same-model serial mismatch stopped before Forward_Open, including when the serial changed after the baseline. A profile with configuration bytes was rejected before any connection attempt.

```powershell
python tools/test_cli.py .\target\release\eipmaster-cli.exe .\.local\qualification-check --qualification
```

A full headless loopback sequence requested 8,000 µs in both directions for the 15-second baseline and 1,000 µs for three 60-second trials. Forward_Open returned the corresponding requested intervals on all four runs. The receipt correctly reported a failed qualification.

| Run | Sample span | End state | CSV rows | Receive sequence gaps | Scanner missed send slots | Fixture missed send slots |
|---|---:|---|---:|---:|---:|---:|
| 8 ms baseline | 15.008 s | Closed | 1,849 | 28 | 0 | 0 |
| 1 ms trial 1 | 60.012 s | Closed | 50,758 | 93 | 5,178 | 9,162 |
| 1 ms trial 2 | 38.554 s | Timed out | 36,821 | 18 | 1,000 | 1,748 |
| 1 ms trial 3 | 16.599 s | Timed out | 15,144 | 24 | 849 | 1,470 |

| Run | Receive p50 / p95 / p99 / longest (µs) | Send p50 / p95 / p99 / longest (µs) |
|---|---:|---:|
| 8 ms baseline | 8,010 / 9,410 / 16,069 / 19,979 | 8,010 / 9,187 / 10,098 / 16,670 |
| 1 ms trial 1 | 1,044 / 2,485 / 4,698 / 26,259 | 1,054 / 2,001 / 2,764 / 7,728 |
| 1 ms trial 2 | 1,022 / 1,411 / 2,600 / 21,042 | 1,030 / 1,457 / 2,123 / 7,658 |
| 1 ms trial 3 | 1,031 / 1,768 / 3,372 / 22,904 | 1,034 / 1,755 / 2,473 / 6,515 |

The fixture received Forward_Close on all four runs. All accepted packets were present in the rotated CSV parts with contiguous sample indexes. There were zero buffer overruns and disk errors. Spacing describes intervals between received packets. The terminal watchdog silence has no later packet to measure as an interval. These results are from localhost with the GUI closed.

## Windows receive readiness and watchdog evidence

The previous full headless sequence ended twice at a 32 ms receive watchdog limit. In one run the fixture recorded a 39.777 ms send gap. In the other its longest measured send gap was 27.872 ms. The earlier reports did not correlate the exact watchdog event with a fixture send timestamp, so the immediate causes remain unconfirmed.

On Windows the receiver now uses Winsock `select` to wait for readable UDP data before calling `recvfrom`. The receiver requests a 2 ms idle wait and keeps the same watchdog limits. [Winsock readiness semantics](https://learn.microsoft.com/windows/win32/api/winsock2/nf-winsock2-select)

Three alternating 10-second pairs per RPI compared the timed receive build (`83f19bf`) with the readiness build on the same localhost fixture. The requested intervals and Forward_Open intervals matched the listed RPI in both directions.

| RPI | Receive path | Accepted and written | Sequence gaps | Per-run receive p99 range (µs) | Longest receive gap (µs) |
|---|---|---:|---:|---:|---:|
| 8 ms | Timed receive | 3,750 | 4 | 8,402–8,620 | 18,507 |
| 8 ms | Readiness | 3,753 | 0 | 8,494–8,623 | 9,834 |
| 1 ms | Timed receive | 29,894 | 6 | 1,492–1,622 | 11,812 |
| 1 ms | Readiness | 29,815 | 0 | 1,495–1,803 | 4,409 |

All runs closed cleanly with zero receive watchdog timeouts and zero CSV overruns. The fixture skipped 132 send slots in the timed 1 ms trials and 227 in the readiness trials. Skipped fixture slots occur before sequence assignment. The trials show fewer received sequence gaps with readiness on this host. They did not reproduce the earlier watchdog failures.

A separate 60-second readiness run requested 1,000 µs in both directions and received 1,000 µs in both directions from Forward_Open. It closed cleanly with 58,772 accepted and written rows, zero receive sequence gaps, zero watchdog timeouts, and zero CSV overruns.

| Measurement point | p50 µs | p95 µs | p99 µs | Longest µs |
|---|---:|---:|---:|---:|
| Scanner T→O receive | 1,047 | 1,365 | 1,818 | 4,923 |
| Scanner O→T send | 1,043 | 1,280 | 1,708 | 4,785 |
| Fixture T→O send | 1,046 | 1,369 | 1,805 | 5,573 |

The scanner skipped 524 send slots and the fixture skipped 1,243. The scan and CSV capture ran headlessly over localhost.

Timeout reports now include the threshold, terminal silence, time since any received UDP datagram, and the last valid sequence when available. The headless CLI tests distinguish a quiet first-input wait from a first-input wait with malformed traffic. The recovery test checks a cyclic watchdog's terminal sequence against its last CSV sample.

## Producer timing and thread priority

The sender records lateness when it reaches a deadline and wall time around the UDP send call. A long call can include time when Windows descheduled the thread. These measurements describe separate points and can come from different packets.

The opt-in deadline mode requests `THREAD_PRIORITY_ABOVE_NORMAL` only for the cyclic producer thread. The sender reports whether Windows accepted the request. [Windows thread scheduling priorities](https://learn.microsoft.com/windows/win32/procthread/scheduling-priorities)

Three alternating pairs ran two concurrent 12-second CLI captures with three-second CSV rotation. Both streams requested and negotiated 1,000 µs in both directions. The primary stream alternated between normal and above-normal deadline scheduling. The peer always used the normal deadline schedule. All twelve captures closed cleanly with zero sequence gaps, watchdog timeouts, or CSV overruns.

| Primary producer priority | Primary missed slots per run | Peer missed slots per run | Primary send p99 range (µs) | Primary maximum send gap (µs) |
|---|---:|---:|---:|---:|
| Normal | 5 / 77 / 27 | 7 / 60 / 18 | 1,230–1,412 | 4,325 |
| Above normal | 2 / 2 / 4 | 61 / 6 / 8 | 1,204–1,335 | 2,859 |

The primary's maximum dispatch lateness was 3,487 µs at normal priority and 1,736 µs at above-normal priority. Its longest UDP call measured 2,695 µs and 1,761 µs, respectively. Peer performance varied between pairs, and the first above-normal trial had 61 peer misses versus seven in its normal-priority pair. Normal priority remains the default.

A separate 60-second run used above-normal priority on the primary and normal priority on the peer. Both requested 1,000 µs and both negotiated 1,000 µs in each direction. Each wrote 59,627 accepted packets to seven CSV parts with zero sequence gaps, watchdog timeouts, or CSV overruns.

| Stream | Missed producer slots | O→T send p50 / p95 / p99 / longest (µs) | T→O receive p50 / p95 / p99 / longest (µs) |
|---|---:|---:|---:|
| Above-normal primary | 40 | 1,075 / 1,151 / 1,221 / 3,963 | 1,065 / 1,197 / 1,466 / 4,986 |
| Normal peer | 320 | 1,073 / 1,186 / 1,479 / 5,174 | 1,063 / 1,217 / 1,638 / 4,710 |

The fixture producers missed 384 and 383 slots, respectively. The primary's worst dispatch lateness was 2,817 µs and its longest UDP send call was 579 µs. The peer recorded 4,612 µs and 2,712 µs, with six send calls lasting at least one RPI. These are headless localhost measurements.

## Graceful console stops

The CLI integration test sent real Windows console events inside an isolated windowless console. The adapter fixture confirmed Forward_Close in every case.

| Event and state | Preserved CSV rows | Result |
|---|---:|---|
| Ctrl+Break after capture rotated | 131 across two parts | Exit 130 and interrupted capture report |
| Ctrl+C during the qualification baseline | 5 | Exit 130 and interrupted qualification receipt with no later trials |
| Ctrl+C during the automatic reconnect delay | 75 | Exit 130 and no new connection attempt |

All accepted rows reached disk with contiguous sample indexes and zero buffer overruns. The windowless test harness requires Windows 11 build 26100 or later. It verifies the process list before sending either event. [Windows console control events](https://learn.microsoft.com/windows/console/ctrl-c-and-ctrl-break-signals)

```powershell
python tools/test_interrupt.py .\target\release\eipmaster-cli.exe .\.local\interrupt-check
```

## Packet span and long intervals

A headless 60-second Windows loopback capture requested 1,000 microseconds in both directions. Forward_Open returned 1,000 microseconds in both directions. The capture closed cleanly and wrote all 58,770 accepted rows across seven CSV parts with contiguous sample indexes, zero receive sequence gaps, zero watchdog timeouts, and zero CSV overruns.

| Socket measurement | Observed span (s) | p50 / p95 / p99 / longest (microseconds) | Intervals at or above 2x / 4x actual RPI |
|---|---:|---:|---:|
| O-to-T send | 60.013080 | 1,022 / 1,326 / 1,825 / 15,697 | 492 / 86 |
| T-to-O receive | 60.013486 | 1,007 / 1,288 / 1,810 / 20,479 | 481 / 92 |

The scanner skipped 1,128 scheduled sends and the fixture skipped 1,244. Both skip slots before assigning a sequence. The long-interval counts describe socket timing even when the sequence is contiguous. The 4x counts are included in the 2x counts.

The 8 ms and 1 ms CLI checks cross-checked the receive span and long-interval counts against CSV timestamps. The qualification regression also checked a fixture returning 1,250 microseconds O-to-T and 1,500 microseconds T-to-O for a 1,000-microsecond request. The thresholds follow the actual interval for each direction. The CSV comparison allows for sub-microsecond truncation at the interval boundaries.

## GUI run report

A headless test exercised the GUI device model with the independent localhost adapter. It armed CSV, connected at a requested 1,000 microseconds in both directions, received 1,250 and 1,500 microseconds from Forward_Open, disconnected, flushed, and saved the report. The report kept the requested and actual intervals separate, matched accepted packets to written rows, and recorded the final connection state. The adapter confirmed Forward_Close.

Additional tests verified that a later connection cannot change the saved capture's connection context, that timeout details survive subsequent draft changes, and that an existing report is preserved. A rotation failure reported the disk error and one unconfirmed row separately from buffer overruns.

To repeat the loopback check, run the fixture in one terminal and the GUI backend test in another. The test opens no window.

```powershell
python tools/loopback_adapter.py .local/report-adapter.json --mode different-api
```

```powershell
cargo test --locked --release --bin assembly-scanner run_report_uses_the_final_loopback_connection -- --ignored
```

## Forward_Close confirmation

The scanner checks the connection serial number, originator vendor ID, and originator serial number in a successful Forward_Close reply. The run reports distinguish confirmation, a CIP rejection, and an unconfirmed close. These fields are independent of any earlier receive timeout or I/O error. [OpENer response implementation](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionmanager.c#L1087)

| Headless 8 ms loopback case | End state | Close outcome | CSV rows |
|---|---|---|---:|
| CIP rejection | Close unconfirmed | Rejected (0x01, extended 0x0107) | 251 |
| Mismatched connection identifiers | Close unconfirmed | Unconfirmed | 251 |
| Truncated success reply | Close unconfirmed | Unconfirmed | 251 |
| TCP closed before reply | Close unconfirmed | Unconfirmed | 251 |
| Receive watchdog, then CIP rejection | Timed out | Rejected (0x01, extended 0x0107) | 75 |

All five captures reported failure and saved all accepted rows. Qualification saved the first 1 ms trial after a close rejection and stopped before opening another connection. The headless GUI backend, reconnect, and Ctrl+C/Ctrl+Break tests also verified confirmed closes in their receipts.

An independent three-second OpENer run requested and negotiated 30 ms in each direction. The scanner confirmed Forward_Close and wrote 101 accepted rows with zero sequence gaps, watchdog timeouts, or CSV overruns. These checks used localhost software adapters.

## Observed packet rates

The CLI uses the first accepted packet to anchor the capture deadline. A clock test verified that later packets update the live age without moving that anchor. The GUI and run reports compute the observed rate from interval count divided by the first-to-last packet span. They compare each direction against its Forward_Open interval.

An 8 ms loopback fixture produced input every 16 ms while advancing the sequence on each send. The scanner measured 50.0% of the negotiated input rate. The capture reported zero sequence gaps and zero CSV overruns. The CLI tests cross-checked the receive span against CSV timestamps and both rates against their spans and negotiated intervals.

A separate 60-second headless loopback run requested 1,000 microseconds in both directions and Forward_Open returned 1,000 microseconds in both directions. It closed cleanly and wrote all 59,950 accepted input rows across seven CSV parts with contiguous indexes, zero receive sequence gaps, zero watchdog timeouts, and zero CSV overruns.

| Socket measurement | Observed span (s) | Packets per second | Percent of negotiated rate | p99 / longest (microseconds) |
|---|---:|---:|---:|---:|
| O-to-T send | 60.009960 | 998.8 | 99.88 | 1,463 / 15,161 |
| T-to-O receive | 60.010275 | 999.0 | 99.90 | 1,482 / 3,624 |

The scanner skipped 72 scheduled sends and the fixture skipped 61. These were loopback socket measurements.

## Logged field summaries

The CSV writer summarizes each selected column across rotated parts. The reports include the mapping, first and last saved values, finite numeric bounds, and changes between adjacent saved rows in the same connection generation. Unit tests cover exact 64-bit integer bounds, nonfinite floats, quoted text, on-change logging, rotation, and disk failure.

A 60-second localhost run used a synthetic 32-byte input assembly with three signed 32-bit fields. Two fields varied and each reported 3,675 changes. The third field was constant and reported zero changes. All three summaries matched the independently parsed CSV values, and all 58,808 accepted rows were saved across six parts with contiguous sample indexes.

Both directions requested 1,000 microseconds and Forward_Open returned 1,000 microseconds. The input rate was 980.1 packets per second with a receive p99 of 1,688 microseconds and a longest interval of 29,883 microseconds. There were zero sequence gaps, receive timeouts, or CSV overruns. The scanner skipped 905 scheduled sends and the fixture skipped 1,196. The run confirmed Forward_Close.

## Report-checked CSV joins

The headless recovery check joined a finalized capture across two connection generations and verified the saved rows against the capture report. Relocating the CSV parts preserved verification. Copies with a missing final part, missing rows in the final part, or an altered last value were rejected. An attempted join with missing rows left no partial output.

A synthetic GUI report accounted for one visible sample-index gap and one overrun after the last saved row. The joined CSV preserved multiline text, empty cells, leading zeros, and the exact decimal text of a full-width unsigned value. A report with no recorded overrun rejected the same index gap. A run report containing a CSV writer error was rejected.

The check also verified all 58,808 rows of an earlier six-part loopback capture using its original report. That report predated the part-count field and supplied the written row count and field summaries.

## GUI reconnect history

The headless GUI backend test ran an 8 ms input-only capture against the loopback adapter and kept CSV armed through a receive stall and automatic reconnect. The report preserved generation 1 as timed out with a 256 ms watchdog threshold and generation 2 as closed. Both entries retained the matching identity, negotiated intervals, and a confirmed Forward_Close. The adapter observed both close requests.

All 127 accepted rows reached the rotated CSV parts. The compatibility `[connection]` section matched the final attempt, and the capture totals covered both generations. A unit test also verified that a later connection outside the capture leaves its report history intact.

The 8 ms CLI regression and recovery tests passed. Two separate 1 ms CLI regressions hit the receive watchdog. The first fixture reached a 32.262 ms send interval in the mismatched-close case. The report preserved the timeout and mismatched close reply and saved all 668 accepted rows. The retry reached 32.339 ms of terminal input silence in the on-change capture. Its report accounted for 2,337 accepted packets as 147 saved rows and 2,190 packets filtered as unchanged, with a confirmed Forward_Close. These were localhost checks.

Start the fixture in one terminal and run the GUI backend test in another. The test opens no window.

```powershell
python tools/loopback_adapter.py .local/report-recovery-adapter.json --mode recover
```

```powershell
cargo test --locked --release --bin assembly-scanner run_report_keeps_loopback_reconnect_attempts -- --ignored --nocapture
```

## Configurable CIP watchdog

An independent loopback fixture read the timeout code from the Forward_Open request. It returned 1,250 microseconds O-to-T and 1,500 microseconds T-to-O for an 8 ms request in both directions, then paused input after sequence 75. Code 3 (32 times the interval) timed out after 48.824 ms of silence with a 48 ms limit and saved 75 rows. Code 4 (64 times the interval) completed with a 96 ms limit, observed a 61.478 ms input gap, and saved 632 rows. Both captures accounted for all accepted packets and confirmed Forward_Close. [Timeout code encoding](https://www.hilscher.com/fileadmin/cms_upload/de/Resources/pdf/EtherNetIP_Scanner_Protocol_API_14_EN.pdf#page=160)

Two separate 15-second Windows loopback captures requested and negotiated 1,000 microseconds in both directions. Both closed cleanly with zero sequence gaps, watchdog timeouts, and CSV overruns.

| Code | Receive watchdog (ms) | Rows written | Observed receive packets/s | Receive p99 / longest (microseconds) | Scanner / fixture skipped send slots |
|---|---:|---:|---:|---:|---:|
| 3 | 32 | 14,939 | 995.5 | 1,558 / 5,236 | 73 / 67 |
| 4 | 64 | 14,953 | 996.3 | 1,574 / 3,793 | 43 / 56 |

These are loopback socket measurements. The test suite checks all eight codes, legacy profiles, a reserved code, and the negotiated watchdog in the GUI and headless reports.

## GUI identity pinning

Headless GUI backend tests read the adapter identity in a standalone TCP session and then attempted an authorized exclusive-owner connection at the same address. A replacement fixture returned the same model with a different serial number. Its receipt showed identity traffic only, with no Forward_Open or cyclic output.

A matching fixture also verified the write permission gate. The GUI first declined the connection with an identity pin and without write confirmation. After confirmation, it opened at 8 ms, sent the staged test bytes and configuration to the loopback adapter, and confirmed Forward_Close. A unit test covered the pending-read gate and retention of a pin after a failed refresh. The loopback tests also checked that completing the read clears any pending bench and write confirmations.

Start the indicated loopback mode in one terminal. Run the corresponding test filter with `cargo test --locked --release --bin assembly-scanner <filter> -- --ignored` in another terminal.

| Fixture mode | Test filter |
|---|---|
| `qualify-replacement` | `manual_pinned_connection_blocks_a_replacement_before_writes` |
| `identity-then-owner` | `matching_pin_allows_separately_authorized_owner_connection` |

## Timed GUI capture

A headless GUI backend test armed a one-second capture and waited 1.05 seconds before connecting to the 8 ms loopback fixture. The timer started at the first input. The test then delayed GUI polling until 200 ms after the cutoff. The capture saved 125 rows across rotated parts, and the connection received 151 packets before the test disconnected and confirmed Forward_Close. The run report recorded the one-second request and completed timer.

A recorder test accepted an input timestamp one microsecond before the cutoff and excluded timestamps at and after it, across connection generations. Other tests covered early manual stop and saving and restoring the duration preference. The CLI regression completed a three-second capture with a one-second GUI duration in the same profile.

To repeat the GUI backend check, start the normal loopback fixture and run the test in a second terminal. The test opens no window.

```powershell
python tools/loopback_adapter.py .local/gui-timed-adapter.json --mode normal
```

```powershell
cargo test --locked --release --bin assembly-scanner timed_gui_capture_finishes_after_first_input_even_when_polling_is_delayed -- --ignored --nocapture
```

## Headless capture cutoff

A ten-second headless loopback capture requested and negotiated 1,000 microseconds in both directions. It confirmed Forward_Close and saved 9,930 rows across five CSV parts. The CSV first-to-last sample span was 9.999193 seconds. The receive socket observed 9,939 accepted packets over 10.008444 seconds, including nine packets after the recording deadline. The report-backed join verified all saved rows.

Receive p50, p95, p99, and longest spacing were 1,033, 1,251, 1,619, and 4,960 microseconds. There were zero sequence gaps, timeouts, or CSV overruns. The scanner skipped 54 send slots and the fixture skipped 70. These are loopback socket measurements.

The recorder test exercised timestamps before, at, and after the cutoff across generations. The GUI backend verified that its report counts the post-deadline packets when polling is delayed. Headless qualification compares CSV rows with in-window packets and retains the full connection metrics. The recovery test let the window expire during reconnect backoff and verified that the scanner opened no further connection. The independent 30 ms OpENer check saved 101 rows in its three-second window and confirmed Forward_Close.

## Startup timing

The Windows localhost fixture delayed its Forward_Open reply by 50 ms and waited 80 ms after the first heartbeat before sending input. The scanner requested and negotiated 8 ms in both directions. It wrote 125 rows in a one-second capture with a confirmed Forward_Close, zero sequence gaps, zero watchdog timeouts, and zero buffer overruns.

| Fixture case | Forward_Open exchange (µs) | First O-to-T send (µs) | First received UDP (µs) | First valid input (µs) |
|---|---:|---:|---:|---:|
| Delayed reply and input | 51,820 | 594 | 81,036 | 81,036 |
| No input traffic | 998 | 495 | — | — |
| Malformed input | 1,415 | 517 | 738 | — |
| CIP rejection | 226 | — | — | — |
| Identity mismatch | — | — | — | — |

The three I/O columns are measured from the end of the Forward_Open exchange, using the scanner host's monotonic clock. The malformed and quiet fixtures each reached the first-input timeout. The identity mismatch stopped before Forward_Open. Headless regression tests also check the saved GUI report and the timing entries before and after a CLI reconnect. These are localhost measurements.

Three one-second headless qualification trials requested and negotiated 1,000 microseconds in both directions. They wrote 998, 996, and 974 rows, with receive p99 spacing of 1,488, 1,533, and 2,028 microseconds and longest gaps of 2,403, 3,499, and 5,191 microseconds. All confirmed Forward_Close and had zero sequence gaps, watchdog timeouts, and CSV overruns. The fixture skipped 2, 5, and 27 send slots before assigning sequence numbers.

## Class 1 packet trace correlation

A headless Windows fixture bound separate adapter ports to receive O-to-T traffic and send T-to-O inputs. The scanner requested and negotiated 8 ms in both directions, saved 251 rows in two seconds, and confirmed Forward_Close. The scanner and fixture agreed on the actual local UDP port (57113), the output destination port (57111), the first valid input source port (57112), and both connection IDs. There were zero receive sequence gaps, watchdog timeouts, or CSV overruns.

The CLI reports omit the input source for silent or malformed traffic and omit the entire `cyclic_io` table if Forward_Open is rejected or never sent. The recovery test matched each attempt to the fixture's endpoints and connection IDs. Headless GUI backend checks verified the saved endpoint and ID fields.

An independent OpENer capture requested and negotiated 30 ms in both directions and saved 101 rows with no sequence gaps, watchdog timeouts, or CSV overruns. Its outgoing destination and first input source were both `127.0.0.2:2222`.

The full headless qualification regression passed on the third run. Two earlier runs each had one 1 ms trial end at the 32 ms receive watchdog, after 33.350 ms and 32.647 ms of terminal input silence. Both confirmed Forward_Close and retained their endpoint and connection ID records. The fixture measured a 39.052 ms send gap in the second timeout case. The first timeout's cause remains unconfirmed. The watchdog and requested intervals were the same across runs.

## Watchdog silence diagnostics

A headless Windows loopback capture requested and negotiated 1,000 µs in both directions. It stopped producing input after sequence 75 while continuing to accept heartbeats. The scanner saved all 75 input rows and confirmed Forward_Close after the 32 ms receive watchdog expired.

| Metric | Observed |
|---|---:|
| Valid input silence at timeout | 33.802 ms |
| Successful scanner O-to-T sends observed during the silence | 33 |
| O-to-T packets received by the fixture after its last input send | 33 |
| Last receive-check gap | 2.019 ms |
| Longest receive-check gap during the silence | 2.716 ms |

The receive checks use host timestamps around the receive loop. The scanner and fixture count different socket events. In 8 ms first-input tests with silent and malformed traffic, both scanner reports recorded 251 successful O-to-T sends. The malformed case continued to receive UDP but accepted no input. The CLI recovery and headless GUI backend tests preserved the timeout diagnostics on the failed attempt and cleared them on the recovered connection.

A separate ten-second loopback capture requested and negotiated 1,000 µs in both directions. It confirmed Forward_Close and wrote 9,984 rows with zero sequence gaps, watchdog timeouts, or CSV overruns. Receive p50, p95, p99, and longest spacing were 1,029, 1,233, 1,481, and 2,978 µs. The scanner skipped 22 send slots and the fixture skipped 16. The CSV cutoff excluded three additional accepted packets while the connection stopped.

## Offline input payload preview

The headless CLI regression decoded a four-byte payload with a little-endian counter, an LSB-first bit, and a swapped 16-bit word. The preview showed all three values and flagged the two columns selected for CSV. A legacy profile displayed its primary `value` field.

A second payload verified signed big-endian decoding with word reversal, a 32-bit float, the full unsigned 64-bit range, and NUL-terminated UTF-8 with a newline. The command decoded the sample with an unfinished connection profile and an empty CSV selection. It rejected malformed hex, short and oversized payloads, and a named field that extended beyond the assembly. All preview checks ran offline.

## EDS input mapping suggestions

The Windows CLI resolved all 32 byte members in the pinned OpENer sample EDS's input-only Connection2. Each suggestion had a distinct name and a U8 selection at its declared offset from 0 through 31. Copying the suggestions into a profile and passing the byte sequence 0 through 31 to `decode-input` returned all 32 expected values. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

Offline tests covered a signed integer after padding, a floating-point field after a skipped string, duplicate names, invalid member widths, malformed layouts, and mismatched connection sizes. A GUI mapping test verified that adding a suggestion preserves the previous CSV selection. These tests exercised the importer and decoder without an attached adapter.

## EDS sizes from assembly members

The Windows CLI imported a synthetic four-byte input layout with a blank assembly size and a signed 16-bit member whose width came from its `ParamN`. Preflight reported four bytes with `assembly_definition` provenance and placed the suggested input at byte offset 2. Rust tests covered both cyclic directions, bit padding, connection-size precedence, and rejection of widths that were partial, unresolved, or larger than the connection size field can represent.

In a copy of the pinned OpENer sample EDS, removing the two cyclic assembly size declarations and using the byte parameters' natural widths produced 32-byte layouts for both directions of the owner profile. The input-only profile kept its explicit 32-byte input and zero-byte heartbeat. Its 32 suggested offsets still ran from 0 through 31. These checks used files and localhost. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

## EDS output mapping suggestions

The CLI imported 32 uniquely named U8 fields from the upstream OpENer owner profile's `Assem150`, at payload offsets 0 through 31. The input-only and listen-only profiles had no output layout. A synthetic owner EDS also returned a signed I16 at byte offset 2 with a Run/Idle header declared separately. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

A headless GUI test clicked through the mapping workflow. It covered an overridden assembly instance, an existing name, a pending typed entry, staged output and configuration bytes, and the authorization flags. An explicit subsequent staging operation encoded `-300` into the two mapped bytes. The localhost owner regression also verified the 2, 12, 5 and 2-byte output length transitions with Run/Idle, zero invalid O→T packets, and confirmed Forward_Close.

## Packed Boolean EDS members

The importer tests resolved explicit BOOL members at bits 0 and 7 of byte 0 and bit 0 of byte 1. They also checked an aligned byte BOOL, adjacent numeric data, and unsupported member declarations. The headless GUI displayed byte 1, bit 3 for an imported output and verified that setting and clearing it in the staged buffer preserved the other bits in that byte.

The CLI imported a synthetic EDS input as byte 0, bit 3 and decoded both states from test payloads. A two-second, 8 ms localhost capture wrote 250 rows. All 127 false and 123 true values matched the fixture's payload at their packet sequence. The CSV buffer reported zero overruns and Forward_Close was confirmed.

## EDS identity comparison

The CLI read all five declared numeric identity values from the upstream OpENer EDS and displayed its product name. The direct localhost Identity Object read matched all five fields of the loopback EDS. A routed localhost read compared three fields from a UTF-16 identity-only EDS and reported the expected product code and minor revision differences. The fixture recorded no cyclic connection exchange during either identity read. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

The headless GUI test displayed both differences in **Review connection**, then displayed a five-field match when the readback changed. The importer test covered hexadecimal values, malformed and out-of-range fields, and saved profile round-trip.

## Nested EDS assembly members

Rust tests resolved an eight-byte layout with two levels of references, repeated groups, and an explicitly padded group. The repeated BOOL and signed fields decoded distinct values at their final offsets. Missing and circular references with unknown widths left the size unresolved. Explicit widths preserved the offsets of following fields when an inner mapping could not be expanded.

The Windows CLI derived four-byte input and output layouts from a synthetic EDS with a nested BOOL. Preflight, raw payload decoding, and a two-second localhost CSV capture at 8 ms verified the final offsets. Every logged Boolean value matched the fixture's packet payload. The pinned OpENer sample EDS produced the same CLI report as the prior release. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

## EDS typed configuration staging

The headless GUI staged an imported Boolean field into a complete local buffer, changing `A5 A0` to `A5 A1`. The read-only byte retained its value. The test covered a missing buffer, an overridden configuration instance, and clearing prior write authorization after staging. The model test rejected an out-of-range integer and a missing configuration instance without modifying the buffer.

CLI import surfaced ten U8 fields at offsets 0 through 9 for the OpENer owner profile's `Assem151`. The other two profiles declared zero configuration bytes. A synthetic EDS verified read-only filtering, a nested Boolean offset, and a four-byte staged buffer in offline preflight. These were offline configuration checks. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

## Read-only software integration

The 8 ms Windows localhost check exercised the HTTP API, MQTT publisher and named shared memory simultaneously. HTTP and the memory mapping stayed open across scanner reconnect. An independent broker fixture dropped its first TCP connection and received fresh MQTT samples after reconnect. HTTP continued while a client held the shared-memory mutex. Shared memory continued while an HTTP client stalled mid-request. The CLI saved 468 CSV rows with zero capture or integration queue overruns. The headless GUI backend test also fetched a live API sample during capture.

```powershell
python tools/test_integration.py .\target\release\eipmaster-cli.exe .\.local\integration-check
```

One five-second headless pair requested and negotiated 1,000 µs in both directions. Both runs confirmed Forward_Close with zero receive sequence gaps, watchdog timeouts, or CSV overruns.

| Software feeds | CSV rows | Receive p50 / p95 / p99 (µs) | Longest receive gap (µs) | Scanner skipped send slots |
|---|---:|---:|---:|---:|
| Disabled | 4,997 | 1,029 / 1,103 / 1,504 | 2,141 | 3 |
| HTTP, MQTT and shared memory | 4,998 | 1,006 / 1,117 / 1,343 | 2,256 | 5 |

The enabled run recorded zero integration queue drops. The client polled 221 HTTP batches and read shared memory 221 times. The MQTT fixture received 104 publications, including publications after its deliberate disconnect. The final API poll was 14 samples behind the completed connection. These figures measure one localhost pair. Physical-device timing and transport latency require separate measurement.

### Repeatable paired feed benchmark

`tools/benchmark_integration.py` ran three five-second pairs using the headless Windows CLI and localhost adapter. Both directions requested and negotiated 1,000 µs in every run, with the same deadline scheduler and CSV settings. The order alternated between feeds disabled first and feeds enabled first. All six runs confirmed Forward_Close with zero receive sequence gaps, watchdog timeouts, or CSV overruns. The three enabled runs reported zero export queue drops and zero MQTT publisher errors.

| Pair | Receive p99 off / on (µs) | Longest receive gap off / on (µs) | Scanner skipped slots off / on | Simulator skipped slots off / on |
|---|---:|---:|---:|---:|
| 1 | 7,249 / 3,348 | 20,746 / 18,033 | 1,437 / 423 | 1,621 / 492 |
| 2 | 4,343 / 1,749 | 9,812 / 7,755 | 692 / 26 | 579 / 109 |
| 3 | 1,618 / 2,125 | 5,439 / 6,282 | 77 / 50 | 39 / 142 |

Each enabled run observed 122 distinct HTTP samples, 123 shared-memory samples, and 122 to 133 MQTT publishes. Apparent age p99 ranged from 2,435 to 7,571 µs for HTTP, 1,687 to 17,401 µs for shared memory, and 1,794 to 6,160 µs for MQTT. The shared host's scheduler varied considerably between runs, including the simulator's missed slots. These pairs do not isolate a timing effect attributable to enabling the feeds. The existing reconnect and stalled-reader integration regression also passed.

A longer pair negotiated 8,000 µs for both 21-second runs. The disabled run wrote 2,624 rows and the enabled run wrote 2,614 rows. Both closed cleanly with zero receive sequence gaps, watchdog timeouts, and CSV overruns. The enabled run reported zero export queue drops and MQTT publisher errors.

## Live HTTP connection diagnostics

The Windows localhost check queried `GET /v1/connection` during headless `serve` and CSV capture. In the `serve` check, the retry requested 8 ms each way and the fixture returned 10 ms O→T and 12 ms T→O. The response reported both sets of intervals, observed receive spacing, and a 384 ms receive watchdog from the returned input interval. After the retry began, the preceding generation remained available with its 256 ms receive timeout and confirmed Forward_Close. The check also exercised a slow HTTP client while shared-memory samples continued to advance.

The headless GUI backend check queried the endpoint during a loopback capture and verified a 1 ms requested input interval, a 1.5 ms returned interval, and a 96 ms watchdog with timeout code 4. The saved run report recorded the same intervals and watchdog. The tests also exercised the loopback Host allowlist, GET-only behavior and the existing status response.

## Concurrent HTTP readers

A Windows localhost comparison used the same HTTP-only `serve` profile, 8 ms requested and negotiated in both directions, and five trials per build. Each trial left a valid status request's headers open while a second client asked for connection diagnostics. On commit `3148e32`, the second request waited until the first request completed in all five trials. Its median latency was 141.3 ms. With four request workers, the diagnostics request finished while the first request was still open in all five trials. The median was 11.0 ms and the range was 9.3–20.6 ms. The fixture delivered 501 baseline inputs and 500 candidate inputs with zero sequence gaps or export queue drops. Both runs confirmed Forward_Close.

A separate check within the candidate run filled the pending queue, observed HTTP 503 on the excess connection and received HTTP 200 again after the waiting clients closed. The HTTP, MQTT, shared-memory and CSV regression checks also exercised concurrent readers during a reconnect.

## Headless feeds without CSV

The Windows localhost fixture exercised `serve` with HTTP, MQTT, and shared memory all enabled at an 8 ms requested and negotiated interval in both directions. The five-second run accepted 467 input packets across a forced I/O timeout and a serial-pinned reconnect. HTTP history and the shared-memory reader kept their sample indexes across generations. MQTT resumed after a deliberate broker disconnect. The report counted zero export queue drops and separated the broker error from shared-memory lock misses. Both I/O generations confirmed Forward_Close. No CSV file or capture directory was created.

A separate HTTP-only run lasted 12 seconds and accepted 1,501 packets with zero sequence gaps or export queue drops. Observed receive p99 was 8.69 ms and the longest gap was 10.68 ms. The windowless Windows console test also stopped an untimed `serve` with Ctrl+Break, confirmed Forward_Close and returned exit code 130. The CSV capture, deadline, writer failure and qualification interrupt regression checks passed with the shared session runner.

## C# feed reader

The .NET 8 sample built in Release with zero warnings. The Windows localhost test started both readers before `serve`, then forced an I/O timeout and a reconnect at 8 ms. Each reader emitted 60 fresh samples spanning two connection generations. Every emitted sample matched the scanner's retained HTTP history by stream timestamp, monotonic elapsed time, packet sequences and payload bytes. The HTTP reader preserved the configured field names and the decoder's count as a string. Both scanner generations confirmed Forward_Close, with zero sequence gaps and export queue drops.

## C# HTTP history

At an 8 ms requested and negotiated interval, the Windows localhost test read 180 consecutive export indexes across a forced Class 1 reconnect. It compared all 180 history records and 120 latest-value records with the scanner's retained packets. Both scanner generations confirmed Forward_Close and the run recorded zero sequence gaps and export queue drops.

A scripted HTTP test returned a full 64-sample page, a busy response, a moved retention boundary, and a new stream identity. The reader retried the same cursor after HTTP 503, reported indexes below the oldest retained sample separately from an in-window index gap, and requested index zero after the stream changed. It also emitted retained samples while the active generation was zero.

## C# MQTT subscriber

The .NET 8 MQTTnet subscriber connected to an independent localhost MQTT 3.1.1 broker fixture at QoS 0. The fixture forced the subscriber socket closed after three forwarded publishes. The subscriber reconnected and resubscribed. With `serve` running at 8 ms requested and negotiated in both directions, all 50 emitted samples matched the scanner's retained HTTP history across two Class 1 generations. The reader excluded injected retained, stale and unsupported-schema messages. The scanner recorded zero sequence gaps, export queue drops or MQTT publisher errors.

## Future-chat documentation and skill discovery

The repository now routes future chats through AGENTS.md and the ethernetip-workbench skill to operational and development guides. The portable package includes these files and tools/install_skill.py. The installed local skill entry point points to the current source location.

The skill validator passed for the tracked skill and installed entry point. Installer checks covered repeat installation, custom directories with spaces, preservation of an unrelated skill and rejection of the canonical source as an install destination. Documentation checks resolved relative references and ran the documented offline help, EDS, preflight and payload-decoding recipes.

## GUI workflow tabs

The desktop workspace now groups Device, Inputs, Outputs, Logging, Mappings, Integration and Diagnostics into separate tabs. Headless egui tests checked 900×580 and 1240×860 layouts, isolation of page content and visible connection/profile/capture controls. Simulated pointer events opened Logging and the profile window, then stopped and flushed capture from Inputs. Rendering the owner connection review showed the staged output and configuration bytes and the ownership/Run warnings without opening a connection.

The five existing GUI backend adapter checks passed for report saving with a live HTTP sample, reconnect history, timed recording, identity mismatch before writes and a separately authorized simulated owner connection. These checks ran without a native desktop window.

## Discovery, explicit reads and keyed large I/O

The Windows localhost fixture verified UDP ListIdentity response matching, deduplication, malformed-response rejection and identity/state decoding. The TCP tests verified Get_Attribute_Single, Get_Attributes_All, 16- and 32-bit logical path segments, numeric CIP rejection status, saved requests and fallback to single attributes when Get_Attributes_All is unsupported. A UTF-16 EDS with a leading format 4 key and a 16-bit Assembly class imported with the large connection format selected.

At an 8 ms requested and negotiated RPI, a separate fixture accepted the keyed Large Forward_Open parameters, returned a 600-byte assembly and confirmed Forward_Close. The CLI wrote 126 CSV rows with a mapped value at byte offset 596, zero sequence gaps and zero buffer overruns. The independent [OpENer adapter](https://github.com/EIPStackGroup/OpENer/tree/0e1ab7ee5c272950c0dc8daf7793008d7a518912) answered discovery and a product-name read. It also completed standard and keyed large input-only sessions at 30 ms, each with 101 rows, confirmed close, zero sequence gaps and zero buffer overruns.

The release tests covered the eighth tab at the existing headless window sizes, named profile persistence for saved requests, and the five GUI backend fixture cases. The library and CLI checked successfully with the GUI feature disabled. All tests here used localhost. Physical interoperability and broadcast discovery on a bench NIC remain open.

## Routed explicit reads

The localhost bridge fixture checked the Unconnected Send wrapper, the embedded read request, and the port/link route bytes. It exercised a single backplane slot and a three-hop path with a padded IPv4 link. The CLI decoded the returned target payload, preserved separate numeric router and target rejections, rejected an empty success reply, and rejected invalid routes before connecting. The saved-read test reopened a routed TOML request and the headless GUI profile test verified route persistence. The same fixture suite also passed the direct read, discovery, and keyed Class 1 capture checks. Physical bridge validation remains open.

## Routed Class 1 input

The Windows localhost bridge fixture exercised a single backplane slot with standard Forward_Open and a three-hop path with a padded IPv4 link, exact electronic key, and Large Forward_Open. The scanner read the target Identity Object through the route. The large case verified fallback to attributes 1–7. The first bridge was the scanner-side UDP peer. Both connection paths matched the fixture byte for byte on Open and Close. The runs requested and negotiated 8 ms in both directions, saved 126 and 125 CSV samples, and confirmed Forward_Close with zero input sequence gaps or buffer overruns.

The fixture also verified that a routing rejection or a replacement serial blocks Forward_Open, and that an unexpected O→T address triggers Forward_Close before sending cyclic traffic. Offline validation rejected malformed routes, routed multicast and routed owner profiles. These checks cover the scanner's wire traffic against a simulated bridge. Physical bridge forwarding remains open.

## Variable-size Class 1 input

The Windows loopback fixture verified standard and Large Forward_Open with a variable-size T→O parameter, fixed-size O→T and a T→O Run/Idle header. It passed 4-, 7- and 12-byte inputs through HTTP, MQTT and shared memory at 8 ms requested and negotiated in both directions. The large connection accepted up to 600 bytes. Each run rejected one undersized and one oversized datagram, recorded the two sequence gaps, and confirmed Forward_Close. CSV matched the accepted values without buffer overruns. On-change capture suppressed changes confined to the unmapped tail. The offline checks also covered the EDS variable-size flag and a mapping beyond the configured minimum. Physical acceptance remains open.

## Variable-size Class 1 output

At an 8 ms requested and negotiated RPI in both directions, the Windows localhost fixture accepted variable-size owner sessions using standard Forward_Open with O→T Run/Idle and Large Forward_Open with modeless O→T. The transmitted payload lengths changed from 2 to 12 (or 600), then 5, then 2 bytes on the same connection. In each run the fixture verified the requested maximum, Connected Data Item lengths, exact payload bytes, 51 received O→T packets, zero invalid packets and zero sequence gaps. The scanner rejected an unauthorized connection and both an undersized and oversized Apply. Both sessions confirmed Forward_Close. The fixed-size owner check and EDS selection tests also passed. Physical adapter acceptance remains open.

## Class 1 multicast input

The Windows loopback fixture verified T→O multicast with unicast O→T at an 8 ms requested and negotiated RPI. It returned a different T→O connection ID and UDP port, injected a datagram with the wrong connection ID, and checked the accepted CSV values. Separate and shared receive/send socket cases passed. Missing or unicast T→O addresses and an occupied receive port each stopped before cyclic output and completed Forward_Close. The localhost EDS check verified the import of a multicast-only T→O profile.

The pinned OpENer adapter selected `239.192.1.32:2222` on Windows loopback at a requested and negotiated 30 ms in both directions. A three-second capture saved 101 rows with zero sequence gaps or buffer overruns and a confirmed Forward_Close. Physical switch and adapter verification remain open.

## Listen-only Class 1 input

The pinned OpENer adapter rejected a standalone listener with CIP status `0x01` and additional status `0x0119`. With a controlling input-only session active, it accepted the listen-only assembly path and returned the controller's multicast group and T-to-O connection ID. The upstream fixture then stopped servicing its shared UDP socket on Windows. The optional listen-only test adapts the fixture copy to retain that socket across connection opens and closes and handle `WSAEWOULDBLOCK` as a transient result.

With the adapted fixture and an independent heartbeat producer, two successive three-second listener runs at 30 ms captured 101 and 100 samples with zero sequence gaps or buffer overruns. Each listener used the same producer group and T-to-O ID, received its own O-to-T ID, and confirmed Forward_Close. The controller remained active between runs and closed separately afterward. The CLI also rejected listener settings that requested ownership, unicast input, Run/Idle, or output or configuration bytes. Physical listen-only interoperability remains open.

## Physical bench acceptance

Physical-device measurements require a connected bench adapter and verified device settings. First verify the Identity Object and the intended profile, preferably input-only. Confirm output ownership and write permissions before using a profile that owns outputs, asserts Run or sends configuration bytes. Then run the Windows GUI and logger with the requested RPIs and record the Forward_Open actual intervals separately from socket spacing percentiles, longest gap, sequence gaps, timeouts, missed send slots and capture losses. Use a capture at the NIC or switch if wire-level timestamps are required.

## Offline CSV timing audit

The Windows recovery test audited a rotated localhost capture with two connection generations and compared p50, p95, p99, and longest saved-row spacing against independently sorted CSV intervals for each generation. It also checked the timed-out attempt and requested RPI from the run report. A synthetic GUI report exercised counter separation with a wrapping packet sequence, a reconnect gap, intentional filtering, and sample-index gaps. Missing and truncated CSV parts still failed verification. A legacy report without an on-change counter passed when its row counts reconciled.

## Saved GUI qualification review

The headless Windows GUI backend generated four one-second reports and their rotated CSV parts against the localhost adapter. The offline review passed all four runs, with 125 rows at 8 ms and 942, 990, and 999 rows at 1 ms. Both directions negotiated the requested intervals. Each run confirmed Forward_Close and reported zero receive sequence gaps or CSV buffer overruns. These results cover the saved-report workflow using shortened loopback trials.

Synthetic cases verified that a repeated capture, a different serial, a negotiated interval mismatch, a timeout, and a missing rotated part each produce a per-run failure. The review retains timing for the other verified trials.

## Offline EDS RPI bounds

The CLI imported OpENer Connection2 with a 30 ms default, a 20 ms minimum, and an unspecified maximum in both directions from Param4. Offline preflight reported both 30 ms defaults within bounds and both 1 ms overrides below the minimum. The override remained eligible for connection under the scanner's own validation. The CLI regression also exercised an upper-bound warning, and the headless GUI review checked that a warning identifies the affected direction. [Upstream OpENer sample EDS](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/data/opener_sample_app.eds)

## Typed explicit reply fields

The Windows localhost fixture returned `78 56 34 12` for both direct and routed saved reads. A routed U16 selection at offset 1 with big endian and byte swapping decoded as 13398 and retained the full reply. A two-byte selection at offset 3 returned the same raw bytes with a separate decode error. The CLI rejected a saved bit index of 8 when loading the profile. An older saved read still decoded the standard serial number as `0x12345678`. Headless GUI checks verified named-profile persistence and that loading a saved request restores the reply editor.

## Named TOML with EDS metadata

A headless save and load restored the selected loopback EDS profile from the TOML alone, including its original 1 ms RPI and typed layout alongside an 8 ms effective override. The source filename was absent from the TOML and the loaded profile required fresh bench confirmation and write approval. The CLI regression loaded an embedded profile for preflight, reported an input RPI override below the embedded minimum, and used the external EDS when explicitly supplied. Plain TOML and payload decoding also passed.

## Refreshing selected EDS metadata

The headless fixture refreshed a revised EDS after the selected connection moved to the second position and its identifier changed case. The updated EDS declared a 4 ms default and 2–8 ms limits. The effective 8 ms request, staged output and configuration bytes, and CSV preferences survived the refresh. Bench and write confirmations reset. A different declared product, missing or duplicate connection IDs, changed assembly path and unsupported replacement metadata were rejected.

## Offline EDS revision comparison

The CLI compared a saved 2 ms input minimum with a revised 1 ms minimum. A file with a different declared product code still returned its changes with refresh blocked. The saved profile bytes were preserved. The CLI also rejected a plain TOML without an EDS snapshot. The Rust test moved one typed field, added one and removed one, then checked the corresponding selections and omitted values in the TOML report. Reordering an otherwise unchanged field produced no change entry.

## Batch saved CIP reads

The localhost fixture counted one registration for a batch containing direct and routed reads. A short typed reply kept its raw data. The successful batch unregistered after its three requests and exited zero. A second batch reported router and target status words, read the next field, and then retained the earlier replies when the peer closed during the fifth request. The sixth request was marked `not_attempted` and the CLI exited 1. An empty saved read list failed before opening a session.

The headless Explore test activated **Read all saved** against the localhost fixture. It verified a direct reply, a router rejection, a later routed reply, an exchange failure and an unattempted request in the displayed order. Expanding the routed reply showed its route, decoded serial and raw bytes. The fixture counted one registration and four attempted requests. The CLI regression passed against the same shared batch runner.

The same headless test checked **Save JSON** with an empty directory setting, then saved the partial batch twice into a new directory. Two separate timestamped files contained identical JSON, including the router status words, the routed serial reply and the unattempted request. The adapter still counted one registration and four attempted requests. The CLI regression passed with the same shared report formatter.

## Compatible format 4 electronic keying

The Windows localhost EDS fixture imported a compatible key from a UTF-16 file and retained its mode in preflight. It rejected a compatible TOML key with minor revision zero. A headless GUI test verified that changing the mode clears the previous write confirmation and that connection review displays the selected mode.

The pinned OpENer 2.3 adapter accepted a 2.2 compatible key using standard Forward_Open. The three-second input-only capture requested and negotiated 30 ms in both directions, wrote 101 rows with zero sequence gaps or buffer overruns, and confirmed Forward_Close. The same 2.2 revision in exact mode returned CIP general status `0x01` and extended status `0x0116` before cyclic I/O. The existing exact-key Large Forward_Open and routed fixture checks also passed. [OpENer key handling](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionmanager.c#L1263)

## Unconnected Multiple Service Packet

The localhost fixture counted one registered session and one explicit exchange for three successful reads, including both Get_Attribute_Single and Get_Attributes_All. A four-read packet returned an embedded error with general status `0x1E`. The CLI kept the failing attribute's general and extended status, decoded the subsequent replies, and exited 1. Parser tests checked count, offset and service mismatches. The pinned OpENer build returned CIP `0x08` for the whole service, which the CLI reported on stderr. The sequential direct and routed tests passed. Successful packet mode still needs a physical target test. [Rockwell CIP Multiple Service Packet format](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm020_-en-p.pdf)

## Direct Class 3 connected reads

The pinned OpENer 2.3 Windows loopback build accepted a standard Class 3 Forward_Open. It returned the product name and serial in order and confirmed Forward_Close. A separate batch returned CIP status `0x14` for an unsupported attribute between those two reads, returned the later serial and confirmed close. The CLI reported the three results and exited 1. Response tests checked that incorrect connection IDs, sequences and service codes are rejected. The direct and routed unconnected and Class 1 fixture regressions passed. [OpENer Class 3 target](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipclass3connection.c)

## Multiple Service Packet over Class 3

The Windows localhost fixture confirmed one connected exchange and an ordered result for each embedded read. A mixed reply with packet status `0x1E` retained the embedded rejection and later successful replies. A separate test kept all successful reads when Forward_Close was rejected. A 508-byte request was rejected before opening a session because the Class 3 connection allows 502 CIP bytes after the sequence field. The pinned OpENer 2.3 build returned packet-level CIP status `0x08` and confirmed Forward_Close. The CLI reported that rejection separately from close and left the individual results empty. Successful grouped reads still need a physical target test.

## Explore saved-read modes

Headless GUI tests opened the mode picker, verified that routed requests require unconnected sequential mode, and ran the existing routed batch against the localhost bridge fixture. Class 3 Multiple Service Packet retained mixed individual replies and a rejected Forward_Close from the localhost fixture, then saved them as JSON. With the pinned OpENer adapter, Explore displayed its packet-level CIP `0x08` and confirmed close and saved both with an empty results array. The GUI and CLI use the same typed batch runner and JSON formatter. The CLI fixture suite passed all four modes.

## Unconnected Multiple Service Packet through a bridge

The Windows localhost bridge fixture verified one registered session and one Unconnected Send per batch. It checked the embedded packet and route bytes for a single backplane hop and a three-hop route with a padded IPv4 link. Successful and mixed replies matched the direct packet results, with bridge and route added to each record. A bridge rejection retained extended status `0x0311` and a separate target packet rejection retained general status `0x08`. Mixed direct and routed reads and lists with different routes failed preflight. A headless Explore test treated `1/03` and `1/3` as the same route and saved three results, including the embedded CIP rejection, as JSON. Routed packet mode still needs a physical bridge and target test.

## Routed Class 3 connected reads

The Windows localhost bridge fixture verified the identical port segments and Message Router path in standard Forward_Open and Forward_Close. A single-hop batch used three connected exchanges with consecutive sequence values and kept replies on both sides of a CIP rejection. A three-hop batch with a padded IPv4 link used one connected exchange for the Multiple Service Packet and covered successful and mixed replies. A rejected open retained the route and extended status `0x0311`, sent no connected reads, and unregistered the session. A whole-packet `0x08` rejection retained the route and confirmed close in the CLI and Explore JSON. Direct Class 3 coverage still includes the pinned OpENer target. Routed operation needs physical bridge and target validation. [pycomm3 Class 3 path construction](https://github.com/ottowayi/pycomm3/blob/master/pycomm3/cip_driver.py)

## Class 3 connection sizes and Large Forward_Open

The Windows localhost fixtures accepted a 511-byte standard request and 4,002-byte Large Forward_Open requests. The direct and three-hop batches each contained 50 reads and sent one 510-byte connected data item including the sequence. Their connected replies contained 1,158 and 1,458 bytes respectively. Both batches preserved all values and confirmed close. A rejected large open returned extended words `0x0126` and `0x01F8`, and the scanner unregistered before sending any reads. Tests covered old profiles at 504 bytes, an out-of-range profile size, an oversized request, and the reply size boundary. Headless Explore reported the size and service in the results and saved JSON, and a named profile retained the configured size after reload.

The independent OpENer 2.3 software target accepted the 4,002-byte Large Forward_Open and confirmed Forward_Close after two Identity Object reads. The larger packets and routed paths were exercised with the localhost fixture. Physical size limits still need verification on the intended target and bridges.

## CIP class inspection

The Windows localhost fixture verified seven ordered Get_Attribute_Single requests at instance zero on one registered TCP session. Direct and routed runs decoded UINT fields and optional ID lists. The routed result kept target status `0x14` with extended word `0x0302`, and a separate router rejection retained `0x0311`. A short list kept its raw bytes and reported a decode error. A headless Explore check showed the routed class results, expanded the raw reply, and preserved the saved requests.

The independent OpENer 2.3 software target reported Assembly class revision 2, maximum instance ID 154 and six instances. It returned attribute-unsupported replies for both optional lists. The CLI retained both statuses and returned exit code 0 for the usable partial result. Routed class inspection was tested with the localhost bridge fixture.

## Reviewed explicit attribute writes

The Windows localhost fixture verified that preview and an incomplete identity pin opened no session. Applying a mismatched serial sent only the identity query. Direct and routed successful applies each used one registered session for the identity query and the exact four-byte Set_Attribute_Single value. Target rejection preserved general status `0x0E` and extended word `0x0302`. A router timeout preserved `0x0204` with an unconfirmed target outcome. The test that dropped the write reply recorded one request and an unconfirmed result. Both write failures and uncertain outcomes returned exit code 1.

In the headless GUI, editing a reviewed value cleared the approved identity. Two read-only review sessions preceded the single apply session. The independent OpENer 2.3 software target acknowledged a write to its dedicated explicit Assembly instance 154 and returned the same 32 bytes on a separate Get_Attribute_Single read. This verification used localhost targets.

## Optional same-session write readback

The Windows localhost fixture verified the reviewed identity, one write, and one Get_Attribute_Single in the same registered session. Direct and routed readbacks matched the four staged bytes. In other cases, the write remained acknowledged while readback reported differing bytes, target status `0x2C`, router extended status `0x0204`, or a lost reply. Those applies returned exit code 1. Readback was skipped for a blocked, rejected, or unconfirmed write. Preview with the readback option opened no connection.

Headless Explore preserved write review and displayed the matched readback separately. The independent OpENer 2.3 software target matched all 32 bytes on the write session and returned the same value on a subsequent separate read. The Explore and Class 1 CLI regression suite passed. Physical-device verification remains open.

## Typed explicit write entry

Windows CLI previews encoded a Boolean, a signed integer, an F32, a big-endian U32 with reversed word order, and fixed-width UTF-8. Invalid values and incompatible encoding options failed before opening a connection, including with `--apply`. A typed U32 apply sent `A5 16 2C 93` once and matched those bytes on the same session.

Headless Explore first reviewed the little-endian bytes. Switching byte order showed the new bytes and disabled send until another identity review. The subsequent write matched on readback. The raw entry and Explore CLI regression checks passed. The independent OpENer 2.3 localhost target acknowledged a fixed-width typed UTF-8 value and matched all 32 bytes, including zero padding, on readback.

## Current-value check before explicit writes

The Windows localhost fixtures verified one identity query, one current-value read, one write and one readback on the same registered session for successful direct and routed runs. A mismatch retained the returned bytes. A target rejection retained `0x2C` and `0x0302`, a router failure retained `0x0204`, and a dropped reply reported the current value as unavailable. These cases sent no write. The identity-mismatch case skipped the comparison. Preview and malformed expected hex opened no session.

Headless Explore invalidated the pending review after the expected bytes changed. A fresh review then displayed separate matched current-value, acknowledged-write and matched-readback results. OpENer 2.3 compared its prior explicit Assembly value, accepted the typed write and returned all 32 staged bytes. The raw and typed write checks and Explore CLI regressions also passed. Verification used localhost targets.

## Class 3 explicit writes

On Windows x64, the localhost fixture verified consecutive Class 3 sequences for identity, current-value check, Set_Attribute_Single and readback in one connection. Direct writes used standard Forward_Open. A three-hop route used a 4,002-byte Large Forward_Open and carried the same path in Forward_Close. Identity and current-value mismatches closed the connection without sending the write. Open rejection prevented connected traffic. A wrong sequence on the write reply produced an unconfirmed outcome with no retry. A rejected close preserved the write acknowledgment and matching readback, with CLI exit code 1. Offline previews and invalid sizes opened no session.

Headless Explore cleared the verified identity when switching to Class 3 and required a fresh review. It displayed the current-value match, write acknowledgment, readback match and confirmed close separately. The pinned OpENer 2.3 build independently acknowledged a direct Class 3 write on its explicit Assembly, matched all 32 staged bytes on readback and confirmed Forward_Close. Physical adapter and bridge validation remains open.

## Reusable write drafts

Headless Explore saved and reopened a Class 3 draft with the staged bytes, current-value check and readback choice. It displayed the draft's endpoint even after the active device address changed and required fresh identity review before enabling Send. The Windows CLI round-tripped both a direct unconnected draft and a routed Class 3 draft generated from a typed value. Python 3.14's TOML parser confirmed the saved field values. Existing files were preserved, and a listening socket recorded no connections from previews, saves or invalid drafts. Each fixture verified the saved identity route, then accepted one write with the saved checks.

The pinned OpENer 2.3 localhost build accepted a reopened Class 3 draft after a separate identity read and current-value match. It acknowledged the write, returned all 32 saved bytes and confirmed Forward_Close. An independent read confirmed the written bytes.
