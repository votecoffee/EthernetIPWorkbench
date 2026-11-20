# Develop and verify EthernetIPWorkbench

Start with [AGENTS.md](../AGENTS.md) and the [repository skill](../.agents/skills/ethernetip-workbench/SKILL.md). Use [Operations](OPERATIONS.md) to reproduce a reported setup before changing behavior.

## Source map

| File | Owns |
|---|---|
| src/model.rs and src/model/mapping.rs | Settings, mapping validation, typed endian/swap decoding and output staging |
| src/eds.rs | Supported EDS connection profiles and provenance |
| src/eds/assembly.rs | Cyclic payload widths and raw input, output and writable configuration mapping candidates from direct and nested EDS members |
| src/eds/comparison.rs | Offline comparison of imported EDS revisions using the GUI refresh checks |
| src/gui/configuration.rs | Typed target configuration staging into the reviewed hex buffer |
| src/eds/identity.rs | Declared EDS identity fields and comparison with a CIP Identity Object readback |
| src/preflight.rs | Effective configuration, EDS comparisons and offline validation |
| src/settings_file.rs | Named profiles, workspace migration, preferences and file replacement |
| src/explicit.rs and src/discovery.rs | Registered TCP exchange, identity, CIP reads, typed reply interpretation and UDP ListIdentity |
| src/explicit/batch.rs and src/gui/explore/batch.rs | Shared saved-read execution and the Explore batch results |
| src/scanner.rs and src/scanner/ | Session lifecycle, Forward_Open/Close, cyclic workers and diagnostics |
| src/scanner/supervision.rs | Blocking read-only runner with identity-pinned reconnects |
| src/deadline_wait.rs | Windows deadline wait resource |
| src/capture.rs and src/capture/field_summary.rs | Bounded sample queue, CSV writer, rotation and summaries |
| src/integration.rs | Common export queue/history, sample indexes, lifecycle and counters |
| src/integration/http.rs | Loopback read API, bounded sample history and connection diagnostics |
| src/integration/mqtt.rs | Latest-sample publisher and broker reconnect |
| src/integration/shared_memory.rs | Windows mapping/mutex and binary snapshot |
| examples/dotnet/ | .NET 8 HTTP and shared-memory consumer example |
| examples/dotnet-mqtt/ | .NET 8 MQTT subscriber example and dependency lock |
| src/gui.rs and src/gui/device.rs | GUI entry point and per-device lifecycle |
| src/gui/workspace.rs | Tabs, persistent actions, review/profile windows and headless layout checks |
| src/gui/app.rs, src/gui/setup.rs and src/gui/explore.rs | Device list, setup, discovery and explicit reader UI |
| src/gui/{mapping,monitor,outputs,logging,software,run_report}.rs | Focused workflow views and capture reporting |
| src/bin/eipmaster-cli.rs | Headless commands and capture supervision |
| src/bin/eipmaster-cli/reads.rs | Single and batch explicit read reports |
| src/bin/eipmaster-cli/serve.rs | Headless telemetry service and final receipt |
| src/bin/eipmaster-cli/qualification.rs | Four-run qualification sequence |
| src/bin/eipmaster-cli/interrupt.rs | Windows console stop handling |
| tools/join_capture.py | CSV joining, report verification and saved-row timing by connection generation |
| tools/review_gui_qualification.py | Offline review of four saved GUI runs against the bench qualification checks |
| tools/benchmark_integration.py | Paired localhost capture timing and software-feed observation counts |
| tools/package_windows.py | Clean-commit Windows build, manifest and license bundle |

## Data and lifecycle contracts

The explicit TCP session reads identity before opening I/O. The cyclic producer schedules from the returned O-to-T interval. The receiver validates the packet source, connection ID, payload length and sequence. It timestamps near receive and copies accepted data to the independent capture and export paths.

For routed Class 1, `Settings::parsed_io_route` uses the same port segment encoder as Unconnected Send. The scanner reads the routed target identity and stops if the read fails. The first bridge receives standard or Large Forward_Open with the port segments before the key and assembly segments, and Forward_Close reuses that path. The UDP send and receive checks use the first bridge address. See the published originator source for connected CIP route placement and path-length encoding. [libplctag Forward_Open and Forward_Close source](https://github.com/libplctag/libplctag/blob/master/src/libplctag/protocols/ab/session.c)

Use `Settings::input_lengths()` for the payload bounds used by the receiver, recorder and export feed. With a variable T→O profile, the mapping must validate at the local minimum, and the connection request encodes the maximum plus the configured headers. The recorder reserves buffers at the maximum and keeps each sample's actual length for decoding. Published raw payloads retain their actual length. The connection flags are verified by the fixture against the standard and large layouts. [OpENer connection parameter reference](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionobject.c)

Use `Settings::output_lengths()` for O→T bounds. An owner connection with a variable-size payload negotiates the maximum plus configured headers. The producer reserves that space once, then encodes the Connected Data Item length and sends only the active bytes on each cycle. `Connection::apply_output` validates the replacement length before swapping it into the active buffer. [ODVA EDS connection parameter reference](https://www.odva.org/wp-content/uploads/2022/06/2015_ODVA_Conference_Wiberg-Schiffer_Optimizing-EDSs-for-EtherNetIP-IO-Connection-Target-Devices_FINAL_PPT.pdf#page=11)

Multicast T→O requests distribution type 1 and a T→O Sockaddr Info item with an unspecified group. Use the adapter's returned group, port and connection ID. Validate the T→O item before joining. Keep O→T on the selected NIC and preserve Forward_Close cleanup if the reply is unusable or the receiver cannot bind. Windows multicast binds are exclusive to avoid interfering with another session's unicast traffic. [OpENer multicast connection source](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipioconnection.c) · [Winsock bind ownership](https://learn.microsoft.com/en-us/windows/win32/winsock/using-so-reuseaddr-and-so-exclusiveaddruse)

The listen-only role validates a read-only heartbeat and multicast T→O. Its Forward_Open uses the adapter's listen-only application path. The wire service and packet processing are the same as the other Class 1 roles. Use the target's existing T→O connection ID and a separate O→T connection ID. Keep automatic recovery restricted to controlling input-only profiles. [OpENer listen-only application path matching](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/appcontype.c)

The CSV queue is preallocated and bounded. Samples retain sequence, monotonic timing, index and generation. Normal stop drains the queue. Preserve existing files if rotation or report creation encounters a collision. A disk failure is scoped to its capture and must remain visible.

The integration receive path copies into a bounded queue and wakes its dispatcher. Serialization, network I/O and mapped-memory access occur in export workers. History retention, queue drops, MQTT publish errors and shared-memory contention have separate meanings. Preserve the versioned JSON and EIPWB001 binary contracts when updating consumers.

For HTTP diagnostics, connect a read-only source to the feed once when each Class 1 generation starts. Retain only the latest two sources. These references have no publisher back-reference. The HTTP worker copies the existing published diagnostics and serializes its own response after releasing the scanner locks.

The HTTP accept worker admits connections into a bounded queue. Four request workers release the queue lock before reading or writing sockets. The header deadline includes queue wait. The accept worker sends a best-effort 503 in nonblocking mode when the queue is full. Closing the accept worker drops the sender and releases readers waiting on the queue.

Connection::start_with_publisher accepts an optional integration Publisher. The blocking scanner::run_input runner drives both CLI capture and serve, collects the final diagnostics for each connection generation, and calls the caller's stop predicate while receiving or waiting to retry. The CLI owns the writer and feed lifetimes. The GUI uses the per-connection methods from its event loop.

Keep automatic reconnect restricted to verified input-only profiles. The retry identity includes serial. Output staging and write authorization belong to the existing GUI/application controls. Preserve the requested/actual/observed interval distinction and the separate Forward_Close result.

## Use the core from Rust

Depend on the `assembly-scanner` library with `default-features = false` to omit eframe. Build the standalone CLI that way with `cargo build --locked --no-default-features --bin eipmaster-cli`.

- Load effective settings with `settings_file::load_profile`. Use `load_named_profile` when you also need the embedded EDS profile for preflight or mapping suggestions. Call `Settings::input_mapping` for validated named fields, then `field.selection.decode(payload)` for a typed value. `Settings::preview_input` adds raw bytes, display values and CSV inclusion for inspections.
- Start a readback with `Connection::start_input(settings, ExpectedIdentity::Device(pin))`. It creates a session without a recorder or exporter. Use `Connection::diagnostics()` for current status and raw input, requested and negotiated intervals, and send/receive measurements. `shutdown()` requests stop and returns the final diagnostics after Forward_Close. Dropping a handle requests stop. Use `shutdown()` when teardown must finish before continuing.
- Use `discovery::discover(local, destination)` for ListIdentity. Use `explicit::Reader::connect(adapter, local)` and `reader.read(ReadPath { class, instance, attribute })` for direct explicit reads. Connect to the first bridge and use `reader.read_routed(path, &route)` for a routed read. Parse `Route` from the same port/link string accepted by Explore and CLI. Router and target rejections retain numeric status and extended words in separate error variants. The target reply retains its service code, which the reader checks before decoding. [ODVA CIP routing overview](https://www.odva.org/wp-content/uploads/2020/06/PUB00123R1_Common-Industrial_Protocol_and_Family_of_CIP_Networks.pdf#page=28)
- Use `explicit::run_saved_reads` for the same ordered batch used by Explore and the CLI. Match the typed result for each saved request. A router or target rejection leaves later requests eligible to run. An exchange failure marks the remaining requests unattempted.

## Build and choose checks

Use Windows x64, the MSVC Rust toolchain, Visual Studio C++ Build Tools and Python 3.11 or later. Use the host's configured Cargo cache and toolchain. The examples assume those are already on PATH.

~~~powershell
cargo +stable-x86_64-pc-windows-msvc fmt --all -- --check
cargo +stable-x86_64-pc-windows-msvc test --locked --offline --release
cargo +stable-x86_64-pc-windows-msvc build --locked --offline --release
~~~

Omit --offline when dependencies need downloading. Select the test that exercises the changed behavior. These functional harnesses own and clean up their local fixture processes and take a fresh directory.

| Change | Headless check |
|---|---|
| Profiles, mapping, CLI and capture | python tools/test_cli.py target/release/eipmaster-cli.exe .local/cli-check-01 |
| EDS assembly-member sizes and mapping suggestions, discovery, direct and routed reads, saved requests, target identity, keyed and routed cyclic I/O | python tools/test_explore.py target/release/eipmaster-cli.exe .local/explore-check-01 |
| Variable-size T→O, EDS provenance, CSV and all three exports | python tools/test_variable_input.py target/release/eipmaster-cli.exe .local/variable-check-01 |
| Multicast T→O, returned endpoints and failed setup cleanup | python tools/test_multicast.py target/release/eipmaster-cli.exe .local/multicast-check-01 |
| Reconnect, concurrent capture and writer failure | python tools/test_recovery.py target/release/eipmaster-cli.exe .local/recovery-check-01 |
| HTTP, MQTT and shared memory | python tools/test_integration.py target/release/eipmaster-cli.exe .local/integration-check-01 |
| Paired feed timing | python tools/benchmark_integration.py target/release/eipmaster-cli.exe .local/feed-comparison-01 --pairs 3 --seconds 5 |
| Headless feeds without CSV, including reconnect | python tools/test_integration.py target/release/eipmaster-cli.exe .local/serve-check-01 --serve |
| C# HTTP latest and history, retention boundaries, and shared-memory consumer | Build the example and run tools/test_dotnet_consumer.py per the [example verification recipe](../examples/dotnet/README.md#verify-on-windows) |
| C# MQTT subscriber and broker reconnect | Restore in locked mode and run tools/test_dotnet_mqtt.py per the [example verification recipe](../examples/dotnet-mqtt/README.md#verify-on-localhost) |
| Qualification supervisor | python tools/test_cli.py target/release/eipmaster-cli.exe .local/qualify-check-01 --qualification |
| Real console signals on supported Windows | python tools/test_interrupt.py target/release/eipmaster-cli.exe .local/interrupt-check-01 |
| Independent OpENer interoperability | python tools/test_opener.py target/release/eipmaster-cli.exe .local/opener-upstream .local/opener-check-01 |

The CLI check defaults to 8 ms. --rpi-us 1000 exercises the same cases at 1 ms. A timing failure must retain its receipt and unchanged watchdog configuration for diagnosis. Failed trials remain evidence alongside any later success.

Add `--explore --seconds 3` to the OpENer check for a short interoperability run covering discovery, a direct CIP read and keyed Large Forward_Open after the standard connection.
Use `--multicast --seconds 3` to also verify OpENer's adapter-selected multicast group and CSV capture.
Use `--listen-only --seconds 3` to check rejection without a controller and two sequential listener captures on the same producer. This test adapts the pinned Windows loopback build so the fixture keeps its shared UDP socket across connection opens and closes and handles `WSAEWOULDBLOCK` as a retryable receive result.

The workspace tests render egui in memory at 900×580 and 1240×860 and simulate pointer events without a native window. They check workflow isolation, visible connection controls, write-review content and capture-stop behavior.

Run `python tools/test_gui_qualification.py .local/gui-review-check-01` for the deterministic report and CSV review checks. The ignored Rust test `save_gui_qualification_reports_headlessly` writes four one-second localhost GUI backend reports when run with the `qualify` fixture mode and a fresh `EIP_GUI_QUALIFICATION_DIR`. Pass those four reports to the review tool with `--link-path loopback --loopback-seconds 1`.

The ignored Rust GUI backend tests need the mode named in each test's ignore annotation. They exercise Device logic without opening a GUI. Start the loopback fixture using a process with CREATE_NO_WINDOW, wait for its Ready line, run the exact test filter with -- --ignored and clean up that fixture. tools/test_cli.py exposes with_adapter for this purpose. Use --bin assembly-scanner so unrelated tests are not selected.

For variable-size O→T, run `standard_open_resizes_cyclic_owner_payload_with_run_idle` with the `variable-output` fixture mode, then `large_open_resizes_modeless_cyclic_owner_payload` with `large-variable-output`. Compare the fixture's `output_length_transitions` to 2, 12 (or 600), 5 and 2. Check for zero invalid O→T packets and confirmed Forward_Close.

tools/benchmark.py starts the actual desktop GUI. Its use requires the user's explicit request for desktop interaction in that chat. Physical-device testing also needs the task's bench authorization and verified profile.

## Package and document evidence

~~~powershell
git fetch origin
python tools/package_windows.py .local/dist --offline
~~~

The packager requires a clean main checkout matching origin/main. It builds locked Windows x64 binaries, validates executable imports, collects dependency licenses and records BUILD-INFO.json and SHA256SUMS.txt. The artifact prefix remains EIPMaster for compatibility with existing delivery tooling. Inspect its embedded commit rather than assuming that prefix is the current repository name.

For a changed dependency, check its actual Cargo archive notices. Missing license files need an upstream revision and a pinned notice under tools/preview_notices, indexed and included by the packager. The existing rumqttc notice follows this path.

Verify the ZIP hash, ZIP integrity, embedded commit and every manifest entry. Run the changed public entry point from a fresh extracted directory. Maintain VERIFICATION.md with the test conditions and meaningful timing/loss results. Source/documentation changes and simulator evidence have different implications from physical acceptance.

## Local artifact housekeeping

Keep reusable fixtures and license notices in the tracked tree. Preserve device settings, vendor material, raw captures and private plotting tools under .local/.

Completed verification runs can be compacted under .local/research-archive. Preserve their original relative paths and a manifest of file lengths and SHA-256 hashes. Verify the archive and source hashes before removing the loose files. Restore individual runs beneath the repository root after checking for existing paths.

Build intermediates under target and Python bytecode caches are reproducible. Check that no scanner or build process is using the selected directories before removing them. Retain the runnable release executables and a verified delivery ZIP when clearing intermediate build products. Keep the Cargo dependency cache outside the cleanup scope.

## Maintain future-chat discovery

The canonical skill is [.agents/skills/ethernetip-workbench/SKILL.md](../.agents/skills/ethernetip-workbench/SKILL.md). Root AGENTS.md routes repository work to it.

~~~powershell
python tools/install_skill.py
~~~

The installer adds a small local entry point under CODEX_HOME/skills/ethernetip-workbench, defaulting to ~/.codex/skills/ethernetip-workbench. That entry point points to this checkout or extracted bundle. It leaves substantive instructions in the tracked skill and docs. Rerun it after relocating the source. --destination selects another skill directory. It preserves an existing skill it did not generate.

New chats discover the local skill as $ethernetip-workbench. An already running chat can read AGENTS.md or the canonical SKILL.md directly.
