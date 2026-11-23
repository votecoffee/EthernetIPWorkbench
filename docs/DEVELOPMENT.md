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
| src/explicit.rs and src/discovery.rs | Registered exchanges, identity, CIP reads, typed reply interpretation and UDP/TCP ListIdentity |
| src/encapsulation.rs | TCP connection bound to the selected NIC and deadline-based EtherNet/IP frame reads |
| src/discovery/services.rs and src/gui/explore/services.rs | Unicast UDP and TCP ListServices and the advertised capabilities view |
| src/explicit/class_info.rs and src/gui/explore/class_info.rs | Common class attribute inspection, decoding and the Explore details view |
| src/explicit/attributes.rs and src/gui/explore/attributes.rs | Common attribute names and same-target advertised choices in Explore |
| src/explicit/attribute_list.rs and src/gui/explore/attribute_list.rs | Direct and routed Get_Attribute_List, fixed-width reply decoding, and Explore results |
| src/explicit/object_list.rs and src/gui/explore/object_list.rs | Message Router Object List and class selection in Explore |
| src/explicit/instances.rs and src/gui/explore/instances.rs | Bounded CIP instance paging and instance selection |
| src/explicit/port_info.rs | Read-only CIP Port Object directory and instance inspection |
| src/explicit/tcpip_info.rs | Read-only TCP/IP Interface Object inspection and attribute decoding |
| src/explicit/tcpip_info/acd.rs | IPv4 conflict flags, ACD settings and retained ARP decoding |
| src/explicit/link_info.rs | Read-only Ethernet Link Object status and counters |
| src/explicit/link_info/port_details.rs | Port control, type, state, label and speed/duplex capabilities |
| src/explicit/lldp_neighbor.rs | Read-only inspection of one CIP LLDP Data Table entry |
| src/explicit/dlr_info.rs | Device Level Ring status, supervisor addresses, and optional fault details |
| src/explicit/dlr_info/participants.rs | Ordered Device Level Ring participant records and count comparison |
| src/explicit/time_sync.rs | CIP Sync state, clock offsets, grandmaster data and port states |
| src/explicit/qos_info.rs | Read-only QoS Object tagging and DSCP settings |
| src/explicit/manager_info.rs | Read-only Connection Manager Object counters |
| src/explicit/connection_data.rs and src/gui/explore/connection_data.rs | Single Connection Manager record lookup by number or Forward_Open triad |
| src/explicit/assembly_info.rs | Read-only Assembly Object declared size and explicit data |
| src/explicit/profile_assemblies.rs | Compare device-reported assembly sizes to profile payload lengths |
| src/explicit/write.rs, src/gui/explore/write.rs and src/bin/eipmaster-cli/writes.rs | Identity-checked Set_Attribute_Single, GUI review and CLI preview/apply |
| src/explicit/write_draft.rs | Versioned TOML write drafts and transport dispatch shared by Explore and the CLI |
| src/explicit/write_receipt.rs | Offline loading and read-only follow-up for completed write receipts |
| src/gui/explore/receipt.rs | Reopened receipt summaries and explicit follow-up checks in Explore |
| src/explicit/batch.rs and src/gui/explore/batch.rs | Shared saved-read execution and the Explore mode picker and results |
| src/explicit/report.rs | Shared JSON fields for explicit reads, batch results and write requests |
| src/explicit/multiple.rs | Multiple Service Packet encoding, decoding and direct or routed unconnected batches |
| src/explicit/connected.rs | Shared Class 3 connection lifecycle for direct and routed explicit messages |
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

For routed Class 1, `Settings::parsed_io_route` uses the same port segment encoder as Unconnected Send and Class 3. Ports 1–14 use the compact identifier. For ports 15–65535 the encoder places the little-endian 16-bit identifier after the segment header and optional link length, before the link bytes. The same encoder uses an explicit link length for IPv4 text and raw hexadecimal links and adds a zero pad byte after an odd-length link. [CIP port segment format, Figure 2](https://www.can-cia.org/fileadmin/cia/documents/proceedings/2005_schiffer.pdf)

Route parsing enforces the one-byte link length and the 255-word route limit. Class 1 validates the combined route, key, assembly and configuration path. Class 3 checks the combined route and Message Router path before opening a session.

The scanner reads the routed target identity and stops if the read fails. The first bridge receives standard or Large Forward_Open with the port segments before the key and assembly segments, and Forward_Close reuses that path. The UDP send and receive checks use the first bridge address. Routed Class 1 uses the same multicast receive setup as direct sessions, including the returned group, port and connection ID. Routed Exclusive Owner requires write authorization and a full device pin (vendor, type, product and serial). The same session verifies that pin before Forward_Open. Optional configuration bytes are carried in a padded data segment in Forward_Open. Forward_Close uses the route and assembly path without the configuration segment. See the published originator source for connected CIP route placement and path-length encoding. [libplctag Forward_Open and Forward_Close source](https://github.com/libplctag/libplctag/blob/master/src/libplctag/protocols/ab/session.c)

Use `Settings::input_lengths()` for the payload bounds used by the receiver, recorder and export feed. With a variable T→O profile, the mapping must validate at the local minimum, and the connection request encodes the maximum plus the configured headers. The recorder reserves buffers at the maximum and keeps each sample's actual length for decoding. Published raw payloads retain their actual length. The connection flags are verified by the fixture against the standard and large layouts. [OpENer connection parameter reference](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionobject.c)

Use `Settings::output_lengths()` for O→T bounds. An owner connection with a variable-size payload negotiates the maximum plus configured headers. The producer reserves that space once, then encodes the Connected Data Item length and sends only the active bytes on each cycle. `Connection::apply_output` validates the replacement length before swapping it into the active buffer. [ODVA EDS connection parameter reference](https://www.odva.org/wp-content/uploads/2022/06/2015_ODVA_Conference_Wiberg-Schiffer_Optimizing-EDSs-for-EtherNetIP-IO-Connection-Target-Devices_FINAL_PPT.pdf#page=11)

Multicast T→O requests distribution type 1 and a T→O Sockaddr Info item with an unspecified group. Use the adapter's returned group, port and connection ID. Validate the T→O item before joining. Keep O→T on the selected NIC and preserve Forward_Close cleanup if the reply is unusable or the receiver cannot bind. Windows multicast binds are exclusive to avoid interfering with another session's unicast traffic. [OpENer multicast connection source](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipioconnection.c) · [Winsock bind ownership](https://learn.microsoft.com/en-us/windows/win32/winsock/using-so-reuseaddr-and-so-exclusiveaddruse)

The listen-only role validates a read-only heartbeat and multicast T→O. Its Forward_Open uses the adapter's listen-only application path, prefixed by the port segments when routed. The wire service and packet processing are the same as the other Class 1 roles. Use the target's existing T→O connection ID and a separate O→T connection ID. Keep automatic recovery restricted to controlling input-only profiles. [OpENer listen-only application path matching](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/appcontype.c)

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
- Use `discovery::discover(local, destination)` for UDP ListIdentity or `discover_with_transport(local, destination, DiscoveryTransport::Tcp)` for a TCP probe. Use `explicit::Reader::connect(adapter, local)` and `reader.read(ReadPath { class, instance, attribute })` for direct explicit reads. Connect to the first bridge and use `reader.read_routed(path, &route)` for a routed read. Parse `Route` from the same port/link string accepted by Explore and CLI. Router and target rejections retain numeric status and extended words in separate error variants. The target reply retains its service code, which the reader checks before decoding. [ODVA CIP routing overview](https://www.odva.org/wp-content/uploads/2020/06/PUB00123R1_Common-Industrial_Protocol_and_Family_of_CIP_Networks.pdf#page=28)
- Use `explicit::run_saved_reads_with_mode` for the four modes shared by Explore and the CLI. The returned report retains the selected mode, per-read results, any packet error and the Class 3 Forward_Close outcome.
- Use `explicit::read_class_info` to inspect common class attributes at instance zero, directly or through a route. It shares the sequential batch transport and preserves raw replies and CIP statuses. `ClassInfo::is_usable` applies the CLI exit rule for partial support.
- Use `explicit::AttributeWrite::validate` to check a proposed write offline and `read_target_identity` to obtain its identity for review. Call `apply` with the reviewed `IdentityPin` to send once. Match the `WriteOutcome` and preserve the difference between a target rejection and an unconfirmed outcome.
- Use `model::Selection::encode_value` to encode one complete typed attribute value. It shares the assembly mapping encoder. Pass its returned bytes to `AttributeWrite` for offline preview and review.
- Use `AttributeWrite::apply_with_readback` for a readable target attribute. `WriteReport::readback` retains the comparison or read failure separately, and `WriteReport::is_success` also requires a match when requested.
- Pass `WriteOptions::expected_current` to `AttributeWrite::apply_with_options` to require an exact comparison before sending. The report retains `current_value` separately from both the write outcome and any readback.
- Use `AttributeWrite::validate_connected` for offline validation and `apply_connected` with `WriteOptions` and `Class3Size` for one reviewed Class 3 write. It uses the same connection for identity and optional value checks. Inspect `WriteReport::forward_close` separately from the write and readback.
- Use `WriteDraft::load` and `save_new` to transfer the wire bytes, paths, transport, and optional value checks between tools. Both validate offline. Call `WriteDraft::read_target_identity` for a separate identity read and `WriteDraft::apply` with the reviewed pin. The versioned TOML schema rejects unknown fields.
- Use `WriteDraft::inspect_current` to read identity and the current attribute on one session. The report preserves the read outcome and any Class 3 close separately. Preparing a guard from these bytes still requires the same-session check in `apply`.
- Use `WriteDraft::check_current` for a read-only comparison against the staged bytes on a session pinned to the supplied identity. Use `SavedWriteReceipt::load` for the same check against a completed GUI or CLI receipt. Keep the historical outcome alongside the follow-up.
- Use `SavedWriteReceipt::follow_up_document` to combine the historical write reference with a completed check. `WriteCurrentCheck::checked_utc` is captured when the check returns and reused on export.
- Use `SavedWriteRecord::load` to review either an original receipt or an exported comparison offline. The loader checks the saved identity, bytes, comparison, transport and timestamp for consistency.
- Use `WriteDraft::report_document` for the JSON request fields. The CLI and Explore combine them with the serialized `WriteReport`. Explore adds `completed_utc` and exports its latest result on the job worker.
- Use `explicit::run_saved_reads` for the default ordered batch. Match the typed result for each saved request. A router or target rejection leaves later requests eligible to run. An exchange failure marks the remaining requests unattempted.
- Use `explicit::run_saved_reads_packet` to pack direct saved reads or reads with one shared route into one unconnected Message Router request. Routed packets reuse the single-read Unconnected Send envelope and router/target reply distinction. Each valid embedded reply uses the same typed outcome as the sequential runner.
- Use `explicit::run_saved_reads_connected` for one direct Class 3 batch or one batch with a shared route. It validates the reply connection ID and CIP sequence and reports the outcome of Forward_Close separately. The routed connection path contains port segments followed by the Message Router. The same path is used when closing.
- Use `explicit::run_saved_reads_connected_packet` for one Multiple Service Packet on Class 3. It shares the connection lifecycle and offset decoder with the other read modes.
- Use `explicit::run_saved_reads_with_class3_size` and a validated `Class3Size` for custom sizing. The previous entry points request 504 bytes. Class 3 and cyclic I/O share the Large Forward_Open encoder. Class 3 request and reply limits account for the two-byte sequence.

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
| Explicit write preview, typed encoding, identity pin, current-value check, readback, Class 3 close, draft portability and receipt follow-up | python tools/test_explicit_write.py target/release/eipmaster-cli.exe .local/write-check-01 |
| EDS mapping suggestions, profile assembly size comparison, discovery, CIP object and instance browsing, class and standard object inspection, direct and routed reads, saved requests, target identity, keyed and routed cyclic I/O, extended CIP ports and hexadecimal links | python tools/test_explore.py target/release/eipmaster-cli.exe .local/explore-check-01 |
| Variable-size T→O, EDS provenance, CSV and all three exports | python tools/test_variable_input.py target/release/eipmaster-cli.exe .local/variable-check-01 |
| Direct and routed multicast T→O, returned endpoints and failed setup cleanup | python tools/test_multicast.py target/release/eipmaster-cli.exe .local/multicast-check-01 |
| Default unicast UDP 2222, standard and routed Large Forward_Open, and port conflicts | python tools/test_default_unicast.py target/release/eipmaster-cli.exe .local/default-unicast-check-01 |
| Routed standard and Large listen-only against a shared OpENer multicast stream | python tools/test_routed_listen.py target/release/eipmaster-cli.exe .local/opener-upstream .local/routed-listener-check-01 |
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

Add `--explore --seconds 3` to the OpENer check for discovery, Assembly class inspection, direct unconnected and Class 3 reads and writes, keyed Large Forward_Open and a comparison of exact and compatible keys for an older minor revision. Both write transports check and read back values on OpENer's isolated explicit assembly.
Use `--multicast --seconds 3` to also verify OpENer's adapter-selected multicast group and CSV capture.
Use `--default-unicast --seconds 3` to verify OpENer's implicit 2222 input destination, unicast heartbeat, and CSV capture.

Build the headless GUI tests with `cargo test --locked --offline --release --bin assembly-scanner --no-run`. Pass the printed test executable path to `python tools/test_routed_owner.py <test-exe> .local/routed-owner-check-01`. This exercises standard and keyed Large Forward_Open, staged outputs, configuration in the open path, the replacement-serial check, and cleanup after an unexpected UDP destination. Use the same test executable with `tools/test_opener.py --owner-test <test-exe> --seconds 3` (in addition to its three positional arguments) for direct Exclusive Owner against the independent OpENer stack. The upstream sample application mirrors the 32-byte output assembly into the input assembly. [OpENer sample application](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/ports/WIN32/sample_application/sampleapplication.c)

Run the ignored test `explore_reads_class_details_without_changing_the_saved_requests` with the localhost fixture in `class-info` mode to verify routed class inspection from Explore.
Run `explore_inspects_a_routed_port_without_changing_saved_requests` with the `port-info-optional` fixture mode to verify the routed Port Object view, decoded node address and optional CIP rejection.

Run `explore_inspects_a_routed_tcpip_interface_without_changing_saved_requests` with the `tcpip-info-optional` fixture mode to verify the routed interface view, decoded network settings, and optional CIP rejection. The OpENer harness with `--explore` also checks the interface address bytes and Physical Link Object path against the independent stack.

Run `explore_reads_multicast_and_session_diagnostics_over_a_route` with `tcpip-info-diagnostics-optional` to verify the opt-in interface diagnostics, including the pending flag and a rejected timeout attribute. In reusable Rust code, `explicit::read_tcpip_diagnostics` returns the ordinary interface batch with the extra attributes. `TcpIpInfo::diagnostics()` gives the typed view.

Run `explore_reads_address_conflict_evidence_over_a_route` with `acd-info-probe` to verify the address conflict view. `explicit::read_tcpip_address_conflicts` returns the four read results, and `TcpIpInfo::address_conflicts()` gives their decoded summary. The OpENer harness with `--explore` checks the partial report when the stack lacks ACD details.

Run `explore_reads_ethernet_link_diagnostics_over_a_route` with the `link-info-optional` fixture mode to verify the routed link status and optional counter rejection. The OpENer harness with `--explore` checks the independently reported speed, flags and MAC address.

Run `explore_reads_ethernet_port_details_through_the_selected_route` with `link-info-details-optional` for the extended view, including the scrollable capability details. `explicit::read_link_port_details` adds attributes 6–11, and `LinkInfo::port_details()` returns their decoded values. The OpENer harness with `--explore` also checks the interface type and speed/duplex options.

Run `explore_selects_and_reads_an_lldp_neighbor_through_a_route` with `lldp-neighbor-gui` for the headless Explore workflow. `explicit::read_lldp_neighbor` returns the raw batch and a typed view of the target's recorded entry. The synthetic CLI fixture checks direct and routed replies, zero and missing values, and malformed attributes. The OpENer harness checks the rejection for an absent object.

Run `explore_reads_connection_manager_counters_over_a_route` with the `manager-info-optional` fixture mode to verify a routed counter and optional CIP rejection in Explore. The OpENer harness with `--explore` checks the reply handling when the stack lacks all eight optional counters.

Run `explore_finds_connection_data_by_triad_over_a_route` with `connection-data` to verify the Explore lookup. The CLI fixture checks the exact requests for both selectors. The OpENer harness with `--explore` opens an input-only connection and reads its record using both selectors before closing.

Run `gui::workspace::tests::diagnostics_looks_up_the_active_connection_across_generations` with `--ignored --exact` while the pinned Windows OpENer fixture is running on 127.0.0.2. It checks the Diagnostics lookup by verified triad even after edits to the next connection's settings. The CLI and HTTP reconnect fixture compares each `forward_open_triad` to the identifiers it received on the wire.

Run `explore_reads_ring_fault_and_supervisor_details_through_a_route` with `dlr-info-optional` to verify the ring status display, supervisor address, and an optional read rejection. The CLI fixture checks a non-default routed instance and the byte order of the node address. Pass `--dlr` to `tools/test_opener.py` to initialize the independent stack's basic DLR object from the Windows loopback launcher and check both inspection modes. The launcher calls the pinned stack's unmodified `CipDlrInit` because that revision's global DLR build option fails on Windows.

Run `explore_keeps_routed_dlr_participants_when_the_count_changes` with `dlr-participants-changed` to check the participant table and read outcomes in Explore. `explicit::read_dlr_participants` returns the list, count samples, and raw batch. The CLI fixture covers stable and changing counts, empty and malformed replies, missing counts, and a target reply-size rejection. The OpENer harness with `--dlr` checks the unsupported-list case.

Run `explore_reads_cip_sync_from_the_selected_route` with the `time-sync-optional` fixture mode to check reported synchronization, grandmaster identity, and a routed attribute rejection. `explicit::read_time_sync` returns the raw batch and a typed status. The CLI fixture covers signed nanosecond values, malformed lengths and counts, and an empty port list. The OpENer harness with `--explore` verifies that the absent object surfaces as a CIP rejection.

Run `explore_reads_qos_settings_over_a_route` with the `qos-info-optional` fixture mode to verify QoS settings and rejections in Explore's Network objects workflow. The OpENer harness with `--explore` checks the configured values for its five readable attributes.

Run `explore_shows_a_routed_assembly_size_mismatch` with the `assembly-info` fixture mode to verify the routed size comparison and raw data in Explore. The OpENer harness with `--explore` checks two instances with different lengths against the independent stack.

Run `explore_checks_profile_sizes_through_the_configured_io_route` with the `profile-assemblies` fixture mode to verify that the profile check uses the cyclic I/O route and shows each role's expected and reported sizes. The OpENer harness with `--explore` checks the input-only fixture and an owner profile with configuration bytes using read-only explicit requests.

Run `explore_lists_the_selected_endpoints_services` with the `list-services` fixture mode to verify the endpoint's advertised flags and raw unrecognized items. Run `explore_lists_services_over_tcp` with the `list-services-stream` fixture mode to verify transport selection and stream framing. The OpENer harness with `--explore` checks the Communications item over both transports before a registered session.

Run `explore_discovers_an_adapter_over_tcp` with the `list-identity-stream` fixture mode bound to `127.0.0.2`. This verifies discovery transport selection and copying the TCP peer address when the advertised endpoint differs. The OpENer harness with `--explore` compares the same adapter's ListIdentity replies over UDP and TCP.

Run `explore_selects_a_class_from_a_routed_object_list` with the `object-list` fixture mode to verify the routed list and selection of a vendor class. The OpENer harness with `--explore` checks the unsupported-attribute response independently.

Run `explore_pages_through_a_routed_instance_list` with the `instance-list-gui` fixture mode to verify two pages, a 16-bit cursor above 255, and selection of an instance. The OpENer harness with `--explore` checks the unsupported-service outcome independently.

Run `explore_browses_known_and_advertised_instance_attributes` with the `class-info` fixture mode. It verifies selection of an advertised 16-bit attribute and removal of the advertised choices when the route changes. The CLI Explore suite verifies typed optional lists against raw class replies.

Run the ignored tests `explore_requires_fresh_review_after_editing_a_staged_write` and `explore_previews_typed_bytes_and_clears_review_on_format_change` with separate localhost fixtures in `explicit-write-gui` mode. They verify review invalidation, the encoded preview, and the matched readback.

Run `explore_requires_fresh_review_after_editing_the_expected_current_value` with the `explicit-write-gui-current` fixture to verify review invalidation and the separate pre-write and readback comparisons.
Run `explore_requires_fresh_review_when_switching_to_class3_write` with the `class3-write-gui` fixture to verify review invalidation on transport change and export of the acknowledged result. Run `explore_saves_unconfirmed_write_receipt` with `class3-write-gui-unconfirmed` for a wrong-sequence acknowledgment and a later match that preserves the uncertainty. Both check that a disk failure preserves the prior receipt and the displayed outcome.
The offline `explore_saves_and_reopens_an_offline_write_draft` test checks draft round-tripping and the fresh-review gate without a fixture.
Run `explore_stages_inspected_bytes_and_rechecks_them_on_apply` with `class3-write-inspect-gui` to verify that only a deliberate copy stages the inspected bytes and a later value change blocks the write.
Run `explore_reopens_saved_write_receipt_and_checks_the_recorded_endpoint` with `class3-write-current-matches` to verify the recorded endpoint, explicit follow-up, timestamped export and offline review in a fresh Explore session. The unconfirmed in-session receipt case also exercises the export.

The release GUI tests exercise mode selection and routed-request gating headlessly. The ignored test `explore_runs_a_multiple_service_packet_on_one_route` uses the localhost fixture in `multiple-service` mode. The ignored test `explore_retains_the_class3_route_on_a_packet_rejection` uses `class3-routed-packet-unsupported`. The ignored test `explore_requests_and_reports_the_selected_large_class3_size` uses `class3-large-packet`. The ignored test `explore_displays_connected_packet_results_and_rejected_close` uses `class3-packet-close-reject`. The ignored test `explore_displays_connected_packet_rejection_and_confirmed_close` uses the pinned OpENer adapter on localhost. Run each test with its matching peer listening, using `cargo test --locked --offline --release --bin assembly-scanner <test_name> -- --ignored`.
Use `--listen-only --seconds 3` to check rejection without a controller and two sequential listener captures on the same producer. This test adapts the pinned Windows loopback build so the fixture keeps its shared UDP socket across connection opens and closes and handles `WSAEWOULDBLOCK` as a retryable receive result. The routed listener test uses the same build adjustments. Its local bridge translates the CIP route and forwards identity, open and close to OpENer over a private TCP port. The bridge and OpENer share a loopback IP for the UDP traffic. The fixture checks the path, heartbeat size and multicast flags before forwarding each open. Pass `--owner-test <test-exe>` to the routed fixture to exercise standard and Large multicast Exclusive Owner. Build the test executable with `cargo test --locked --offline --release --bin assembly-scanner --no-run`. The test checks approval and identity, stages two cyclic output values, compares OpENer's mirrored inputs, and reads its configuration Assembly after close.

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
