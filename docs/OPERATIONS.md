# Configure and operate EthernetIPWorkbench

This guide is the starting point for a chat that needs to run the existing application. The [README](../README.md) contains the complete GUI and software-interface reference. [Development](DEVELOPMENT.md) maps behavior to source and tests.

## Locate the application and settings

The Windows x64 programs are assembly-scanner.exe and eipmaster-cli.exe. A source build places them in target/release. A portable ZIP places them beside samples/ and tools/. Run the commands below from that root. For a portable bundle, replace .\target\release\eipmaster-cli.exe with .\eipmaster-cli.exe.

GUI settings are automatically saved to %LOCALAPPDATA%\EIPMaster\last-session.toml. It contains the device list, named setups and selection. A named device profile is a separate TOML file saved and loaded through Profiles… → Settings TOML. The repository rename retained these runtime names for compatibility.

Each named profile contains connection fields at the root, a [selection] table, optional [[input_fields]] and [[output_fields]], and optional [app] preferences. Configuration deserialization rejects unknown fields. A runtime bundle and a source checkout may be different revisions. Check BUILD-INFO.json in a bundle before relying on a recent feature.

Keep device-specific profiles, EDS files, captures and plotting tools under ignored .local/. Confirm the ignore rule with git check-ignore when using the source checkout.

## Follow the GUI workflow

Start in Device to choose an EDS/manual connection and read identity. Connect… opens the connection review, then a successful connection shows Inputs. To log immediately, open Logging and arm CSV before connecting. Stop capture and flush is available above every tab.

Use Mappings for field definitions and CSV column selection, Outputs for staged writes, Integration for software feeds and Diagnostics for packet timing and errors. Profiles… opens saved TOML setups. Manage devices… holds rename/remove actions. Raw input bytes and advanced settings expand within their relevant tabs.

## Discover and read

Set the NIC address in Device. In Explore, enter a unicast or broadcast destination and select Discover. The scan sends one ListIdentity request and waits up to 2.2 seconds. It deduplicates replies and shows the number of invalid datagrams. The CLI also includes each responder's advertised endpoint. Broadcast discovery depends on the NIC, local firewall and adapter. Use the responder's source IP and verify the Identity Object before connecting.

Select a known attribute in the CIP browser or enter a documented class, instance and attribute. The browser uses Get_Attribute_Single or Get_Attributes_All and supports 8-, 16- and 32-bit IDs. Discovery and reads run independently of cyclic I/O and the CSV writer.

For a routed read, set the first bridge's IPv4 address in Device and enter its path in **CIP route (optional)**. A path is a sequence of decimal port/link pairs. Use up to eight hops, port numbers 1–14, and a link address in the form of a byte (0–255) or a unicast IPv4 address. For example, `1/2/2/192.0.2.100/1/0` crosses a backplane, an Ethernet link and a second backplane. Choose port and link values from your topology. The bridge must support CIP Unconnected Send. For a direct read, leave the route blank. The CLI takes the same syntax as `--route <port/link/...>` after the attribute. A routed report uses `reply_from = "router"` for a bridge rejection and `reply_from = "target"` for the destination reply, including a rejected attribute.

~~~toml
[[app.cip_reads]]
name = "Serial number"
route = "1/3"
[app.cip_reads.path]
class = 1
instance = 1
attribute = 6
~~~

The saved read uses the device or bridge address and NIC of the enclosing profile. Omit `route` for a direct read and `attribute` for Get_Attributes_All. Add this table to a named profile, or use Save / update read in Explore and then save the profile. Run `eipmaster-cli read-saved <settings.toml> "Serial number"` against that profile. This route belongs to the saved explicit read. Identity and the cyclic connection use the separate `io_route` setting.

## Start offline

~~~powershell
.\target\release\eipmaster-cli.exe --help
.\target\release\eipmaster-cli.exe eds samples\loopback.eds
.\target\release\eipmaster-cli.exe validate samples\mapped-loopback.toml samples\loopback.eds Connection1
.\target\release\eipmaster-cli.exe decode-input samples\mapped-loopback.toml "01 02 03 04"
~~~

eds lists supported connection profiles and unresolved features. validate can compare a manual/saved profile to one EDS connection. Its origins distinguish EDS values, overrides, assembly-definition sizes and manual values. An assembly-definition size needs comparison with the device's configured payload. Check connection_valid and capture_config_valid. The validate exit code follows connection_valid, so a successful exit alone does not establish that CSV selection is valid.

decode-input accepts exactly input_size bytes for fixed-size profiles or a length within the configured bounds for variable-size profiles. Exclude sequence and Run/Idle headers. It lists every mapped field's source bytes, decoded text value and CSV inclusion. The GUI has the same check under Mappings → Preview input payload. With no named input fields, the primary selection appears as value.

## Configure a device

Start from a generic sample and use the device's EDS and manual to establish each value.

| Setting | Interpretation |
|---|---|
| device_ip, local_ip | Adapter or first bridge IPv4 and IPv4 assigned to the scanner NIC |
| io_route | Optional port/link pairs for routed Class 1 input |
| local_udp_port | 0 requests an ephemeral port, otherwise bind the required local port |
| input_instance, input_size | Adapter-to-scanner assembly and payload bytes |
| output_instance, output_size | Scanner-to-adapter assembly or heartbeat connection point |
| output_min_size | Optional local lower bound for variable-size owner output |
| config_instance, config_size, config_hex | Optional configuration path and bytes sent in Forward_Open |
| output_hex | Full staged output payload |
| input_rpi_us, output_rpi_us | Separate requested intervals in microseconds |
| input_run_idle, output_run_idle | Whether each direction carries the four-byte header |
| input_only | Selects a read-only profile, including listen-only when requested |
| listen_only | Uses the adapter's listen-only heartbeat path to consume an existing multicast stream |
| timeout_multiplier_code | CIP code 0–7, default 3 for older profiles |

Sizes exclude the Class 1 sequence and optional Run/Idle header. Standard Forward_Open supports at most 509 payload plus Run/Idle bytes per direction here. The code accepts requested intervals from 1,000 to 10,000,000 microseconds.

Set the root TOML key `large_forward_open = true` when the adapter supports service 0x5B. Large Forward_Open allows up to 1,400 payload plus Run/Idle bytes here. CSV, live views, HTTP and MQTT support the larger payloads. The existing EIPWB001 shared memory format keeps its 509-byte limit. The app rejects a larger input when shared memory is enabled.

### Configure routed Class 1 input

Use **Device → CIP routing** to set the documented port/link path from the first bridge to the input target. In TOML, set `device_ip` to the first bridge's IPv4 address and `io_route = "1/3"` for a target at backplane slot 3. Leave the path blank for a direct connection. Use the target's assemblies, RPIs, transfer format and electronic key. The EDS supplies assembly settings. The route describes the installed topology and is entered manually.

This mode uses unicast input-only with zero output and configuration bytes and O→T Run/Idle disabled. The first bridge must support routed Class 1 and act as the scanner's UDP peer in both directions. Check those capabilities for the intended bridge and target before physical use.

Read identity through the configured route before connecting. The CLI equivalent is `eipmaster-cli identity <bridge-ipv4> <local-ipv4> --route 1/3`. Use the returned target identity in the capture command. The target Identity Object is read again before each Forward_Open. Forward_Open and Forward_Close carry the same routed assembly path. Preflight and capture reports show `io_route`. In `cyclic_io`, the output destination and first input source identify the scanner-side bridge.

### Configure variable-size input

In **Device**, set the maximum T→O payload in the assembly grid, enable **Variable-size input (T->O)** and set **Minimum input bytes**. In TOML, `input_size` is the maximum and `input_min_size` enables variable-size input and sets the inclusive lower bound. For example, `input_size = 12` and `input_min_size = 4` accept 4 through 12 payload bytes. The adapter sees the maximum and the variable-size flag in Forward_Open. The local minimum and maximum exclude sequence and Run/Idle bytes.

All input mappings must fit the shortest accepted packet. Shorter or oversized packets increment the invalid-packet counter. Missing accepted sequence numbers also appear as sequence gaps. The live input view, offline decoder and software feeds use actual payload lengths. CSV retains its existing columns. On-change capture compares the selected values, so a change in an unmapped tail or its length alone adds no row.

When an EDS profile requires variable T→O, the importer enables it and initially sets the minimum equal to the imported maximum. Review and enter the documented minimum before connecting. If the EDS allows both fixed and variable input, the importer selects fixed. Omit `input_min_size` to retain fixed-size behavior.

### Configure variable-size output

Select **Exclusive owner** in **Device**. Set the documented maximum O→T payload in the assembly grid, then enable **Variable-size output (O->T)** under **Configuration and advanced timing** and set the minimum. In TOML, `output_size` is the maximum and `output_min_size` enables variable-size output and sets the inclusive lower bound. Use assembly byte counts with headers excluded. The scanner sends the maximum plus the required headers in Forward_Open.

A variable-only O→T EDS profile sets the initial minimum equal to the imported maximum. Verify the device's accepted range and update the minimum. If the EDS offers both modes, the importer selects fixed. Omit `output_min_size` for fixed-size output.

In **Outputs**, enter the exact payload as hex. Changing the number of staged bytes changes the length you will apply. Typed output fields can edit bytes present in the staged buffer. Connect review shows the initial length and output ownership. For an open owner session, review the staged bytes and length, authorize them, then use **Apply staged outputs**. The active buffer shows the length currently supplied to the cyclic producer.

### Configure multicast input

Enable **Multicast input (T->O)** in Device or set `input_multicast = true` in TOML. A multicast-only T→O EDS profile selects it automatically. When the EDS offers both distributions, it defaults to unicast. Input-only and exclusive-owner profiles retain their existing ownership and confirmation rules. O→T stays unicast.

Forward_Open returns the T→O group, UDP port and connection ID. The scanner joins that group on the configured NIC and checks the adapter's source IP and returned ID. Use **Diagnostics** or the capture report's `cyclic_io` table to see the multicast destination and local receive endpoint. The requested local UDP port still binds the O→T socket. For multicast it also supplies the port hint, which defaults to 2222 when the field is zero. The returned receive port takes precedence.

Windows uses one owning socket per local address and port. The scanner shares its own socket within a session when the returned input port matches its output socket. A port already occupied by another session or application causes an error and a Forward_Close attempt. If the connection opens but inputs time out, verify the returned group, selected NIC and multicast forwarding on the bench network.

### Configure a listen-only connection

Start from the adapter's listen-only EDS profile or select **Connection role → Listen-only** in Device. Enter that profile's documented O→T heartbeat connection point and T→O input point. The role is determined by the connection path the adapter receives. Switching into or out of Listen-only in the GUI clears the old O→T instance so you can enter the correct one.

For TOML set `listen_only = true`, `input_only = true`, and `input_multicast = true`. Use zero output bytes, zero configuration bytes and O→T Run/Idle disabled. Retain the documented configuration assembly instance in the path if the adapter requires one. Apply the RPIs and input layout required by the controlling connection. The listener sends an empty heartbeat and records the adapter's existing multicast stream without requesting output ownership.

Start the controlling multicast connection first. CIP general status `0x01` with extended status `0x0119` means the target found no controlling connection for the input assembly. Use normal GUI logging or CLI `capture`. Listen-only uses manual reconnect. CLI `qualify` changes the requested rates and requires a controlling input-only profile. If the controller or another process owns the local multicast receive address and port, use a separate supported host or NIC for the listener.

Configure the optional `[electronic_key]` table with decimal `vendor`, `device_type`, `product`, `major` and `minor` fields. The EDS importer handles a supported leading format 4 electronic key, UTF-16 with a BOM, and 16-bit Assembly class segments. Review the imported Large Forward_Open setting when the declared size exceeds the standard limit. Device payload and supported services still need checking against the adapter.

In the GUI, import the EDS, choose a connection profile, review provenance and edit overrides. Use Manual settings when entering the connection directly. Read identity and verify vendor, device type, product code and serial before connecting. A successful identity read pins the next manual connection to that unit. Changing the addresses or cyclic route, or loading a profile, clears the pin.

For input-only logging the profile must have zero output and configuration sizes and O-to-T Run/Idle disabled. The adapter must support the selected heartbeat connection point. An input-only connection still sends periodic O-to-T heartbeats at the returned O-to-T interval.

An owner connection can take output control and send the staged buffer immediately. Confirm device, ownership and specific output/configuration bytes against the current task authorization. Named output fields stage typed values into output_hex. Apply staged outputs changes the active cyclic buffer. Configuration bytes take effect during a new Forward_Open.

## Map and select values

Offsets are zero-based byte positions. A bit is numbered 0–7 within its byte, least significant first. Integer types are U8/I8 through U64/I64. F32 and F64 use IEEE representation. Bool tests a byte or a selected bit. Utf8 uses string_length bytes, stops decoding at NUL and zero-pads staged text.

big_endian defaults to false. swap may be none, bytes_in_words or word_order. Reading applies the swap before interpreting endian order. Test a distinct value in every mapped field to expose incorrect offsets.

~~~toml
capture_mode = "every_packet"
log_fields = ["measurement", "ready"]

[[input_fields]]
name = "measurement"
selection = { offset = 0, kind = "I16", big_endian = true }

[[input_fields]]
name = "ready"
selection = { offset = 2, kind = "Bool", bit = 0 }
~~~

Add root keys before TOML table declarations. Use the existing sample structure when editing a profile. log_fields omitted means all mapped inputs. An empty array selects none, which is useful for preview but prevents CSV capture. on_change writes a row when any selected field changes and at the first packet of a new connection generation. Decoder output preserves raw values. Check device-specific decimal scaling and units separately.

## Capture and verify CSV

A headless localhost check starts and cleans up its own independent adapter.

~~~powershell
python tools/test_cli.py .\target\release\eipmaster-cli.exe .\.local\cli-check-01
~~~

For an authorized bench capture, read identity first using the supplied adapter/NIC addresses. Use the returned decimal values in vendor:type:product:serial. The illustrative loopback identity is 0:12:1:305419896.

~~~powershell
.\target\release\eipmaster-cli.exe identity 127.0.0.1 127.0.0.1
.\target\release\eipmaster-cli.exe capture samples\loopback.toml .\.local\capture-01 10 0:12:1:305419896
~~~

The two loopback commands require a fresh tools/loopback_adapter.py instance for each connection. A physical capture uses the verified private profile and real identity. The output directory must be new and its parent must exist. Capture starts its time window at the first accepted input and accepts timestamps strictly before the deadline. Supported CLI duration is 1–3,600 seconds. Ctrl+C or Ctrl+Break requests Forward_Close, flushes CSV and returns exit code 130.

The GUI supports manual capture or Stop after seconds, where zero means manual. The GUI capture timer stops CSV and flushes independently of Disconnect. Disconnect before saving the final run report.

| [app] preference | Meaning |
|---|---|
| directory | GUI capture directory, overridden by the CLI output-directory argument |
| capacity | Preallocated CSV sample capacity, 1–1,000,000 |
| rotate_bytes, rotate_seconds | Optional rotation limits, zero disables each |
| gui_capture_seconds | GUI timed recording limit, 0–3,600 |
| schedule | deadline, deadline_above_normal or tokio |
| auto_reconnect | Opt in to bounded input-only recovery with identity verification |

Each CSV row contains capture_start_utc, elapsed_us, sample_index, connection_generation, packet_sequence, cip_sequence and selected field columns. Capture start UTC anchors monotonic elapsed time. A reconnect retains the CSV origin and sample indexes and changes the generation.

The CLI writes capture.toml beside the parts. The GUI saves a separate -report.toml after disconnect and flush. Use the actual csv_prefix from the report.

~~~powershell
python tools/join_capture.py 'captures/capture-<UTC-prefix>' --report captures/capture.toml
python tools/join_capture.py 'captures/capture-<UTC-prefix>' joined.csv --report captures/capture.toml
~~~

Report checking validates the recorded parts, counts and values. Preserve all parts and the failure receipt if joining fails. Normal capture completion requires a confirmed final Forward_Close and consistent row accounting.

## Connect other software

Use [Connect other software](../README.md#connect-other-software) for complete profile keys and the binary layout. Enable feeds per device in [app.integration] or in the GUI. They start with the scanner session, survive automatic reconnect and end when the session ends. Distinct devices need distinct HTTP ports and shared-memory names.

| Interface | Consumer contract |
|---|---|
| HTTP GET /v1/status | Active generation and independent integration counters |
| HTTP GET /v1/latest | Latest raw/typed sample, age and active generation |
| HTTP GET /v1/samples?after=N | Up to 64 samples from the newest 1,024 retained frames |
| MQTT | Latest new active sample at the configured period, TCP 3.1.1, QoS 0, retain disabled |
| Windows shared memory | Latest raw snapshot in a named 573-byte mapping with a named mutex |

HTTP is bound to 127.0.0.1. MQTT uses a trusted listener or a local broker with a secure bridge. Values in HTTP/MQTT are text to preserve integer width and decoder output. All named input fields are exported independently of CSV selection.

Reset an HTTP cursor when stream_start_utc_us changes. Compare sample indexes and oldest_sample_index to detect lost history. In a latest-value read, compare generation to active_generation and check timestamp/age. active_generation zero means cyclic I/O is inactive. MQTT can miss intermediate updates by design.

Shared-memory readers acquire the lock, copy and promptly release it. Discard an abandoned-mutex snapshot after releasing ownership. Close client handles before restarting a manual session with the same object names. A stale mapping may survive in a client process after the scanner exits.

The implemented interfaces export telemetry. For C#, HTTP/shared memory or CLI process supervision can consume the scanner today. A C ABI for embedding the scanner is prospective work.

## Interpret timing and failures

| Observation | Read it with |
|---|---|
| Requested RPI | Profile input_rpi_us and output_rpi_us |
| Returned intervals | actual_ot_us and actual_to_us from Forward_Open |
| Measured spacing | Per-direction receive_spacing and send_spacing |
| Terminal input silence | receive_timeout, including O-to-T sends and receive-check gaps |
| Local producer delay | skipped_deadlines and send_timing |
| Network anomalies | sequence_gaps, duplicates, late and invalid |
| CSV loss | buffer_overruns, disk_error and unconfirmed rows |
| Export loss/contention | queue_drops, mqtt_errors and shared_memory_busy |

accepted describes the connection, while seen/written describe the recording window. after_capture_deadline counts accepted packets excluded after the CSV deadline. Intentional unchanged filtering is separate from overruns. A zero receive sequence-gap count can coexist with missed producer slots.

For cyclic input, the local watchdog is the returned input interval times (4 << timeout_multiplier_code). Before the first accepted input the local wait is the larger of two seconds and twice the returned input interval. Review receive_timeout and startup_timing for the failure phase. cyclic_io records actual endpoints and connection IDs for a packet trace. Physical link and wire-timestamp evidence require their own measurements.

## Bench acceptance

When the adapter is available and the connection is authorized, use qualify with the verified private profile, a new output directory, direct or switched, and the full expected identity. It performs a 15-second baseline at 8 ms and three 60-second trials at 1 ms. Each direction is requested separately and the receipt preserves the actual intervals and timing. With a routed profile, direct or switched describes the first Ethernet segment and the report also records the CIP route. A shorter override is accepted only for loopback.

The sustained physical acceptance milestone additionally requires the GUI and logger active under recorded bench conditions. Use [VERIFICATION.md](../VERIFICATION.md) for existing evidence and [ROADMAP.md](../ROADMAP.md) for outstanding work. Older physical captures and successful simulator trials do not establish acceptance for a new build.
