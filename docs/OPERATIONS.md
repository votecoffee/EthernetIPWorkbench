# Configure and operate EthernetIPWorkbench

This guide is the starting point for a chat that needs to run the existing application. The [README](../README.md) contains the complete GUI and software-interface reference. [Development](DEVELOPMENT.md) maps behavior to source and tests.

## Locate the application and settings

The Windows x64 programs are assembly-scanner.exe and eipmaster-cli.exe. A source build places them in target/release. A portable ZIP places them beside samples/ and tools/. Run the commands below from that root. For a portable bundle, replace .\target\release\eipmaster-cli.exe with .\eipmaster-cli.exe.

GUI settings are automatically saved to %LOCALAPPDATA%\EIPMaster\last-session.toml. It contains the device list, named setups and selection. A named device profile is a separate TOML file saved and loaded through Settings TOML. The repository rename retained these runtime names for compatibility.

Each named profile contains connection fields at the root, a [selection] table, optional [[input_fields]] and [[output_fields]], and optional [app] preferences. Configuration deserialization rejects unknown fields. A runtime bundle and a source checkout may be different revisions. Check BUILD-INFO.json in a bundle before relying on a recent feature.

Keep device-specific profiles, EDS files, captures and plotting tools under ignored .local/. Confirm the ignore rule with git check-ignore when using the source checkout.

## Start offline

~~~powershell
.\target\release\eipmaster-cli.exe --help
.\target\release\eipmaster-cli.exe eds samples\loopback.eds
.\target\release\eipmaster-cli.exe validate samples\mapped-loopback.toml samples\loopback.eds Connection1
.\target\release\eipmaster-cli.exe decode-input samples\mapped-loopback.toml "01 02 03 04"
~~~

eds lists supported connection profiles and unresolved features. validate can compare a manual/saved profile to one EDS connection. Its origins distinguish EDS values, overrides, assembly-definition sizes and manual values. An assembly-definition size needs comparison with the device's configured payload. Check connection_valid and capture_config_valid. The validate exit code follows connection_valid, so a successful exit alone does not establish that CSV selection is valid.

decode-input requires exactly input_size bytes of assembly payload with no sequence or Run/Idle headers. It lists every mapped field's source bytes, decoded text value and CSV inclusion. The GUI has the same check under Assembly mappings → Preview input payload. With no named input fields, the primary selection appears as value.

## Configure a device

Start from a generic sample and use the device's EDS and manual to establish each value.

| Setting | Interpretation |
|---|---|
| device_ip, local_ip | Adapter IPv4 and IPv4 assigned to the scanner NIC |
| local_udp_port | 0 requests an ephemeral port, otherwise bind the required local port |
| input_instance, input_size | Adapter-to-scanner assembly and payload bytes |
| output_instance, output_size | Scanner-to-adapter assembly or heartbeat connection point |
| config_instance, config_size, config_hex | Optional configuration path and bytes sent in Forward_Open |
| output_hex | Full staged output payload |
| input_rpi_us, output_rpi_us | Separate requested intervals in microseconds |
| input_run_idle, output_run_idle | Whether each direction carries the four-byte header |
| input_only | Selects the intended input-only profile and its validation |
| timeout_multiplier_code | CIP code 0–7, default 3 for older profiles |

Sizes exclude the Class 1 sequence and optional Run/Idle header. Standard Forward_Open supports at most 509 payload plus Run/Idle bytes per direction here. The code accepts requested intervals from 1,000 to 10,000,000 microseconds.

In the GUI, import the EDS, choose a connection profile, review provenance and edit overrides. Use Manual settings when entering the connection directly. Read identity and verify vendor, device type, product code and serial before connecting. A successful identity read pins the next manual connection to that unit. Changing the addresses or loading a profile clears the pin.

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

When the adapter is available and the connection is authorized, use qualify with the verified private profile, a new output directory, direct or switched, and the full expected identity. It performs a 15-second baseline at 8 ms and three 60-second trials at 1 ms. Each direction is requested separately and the receipt preserves the actual intervals and timing. A shorter override is accepted only for loopback.

The sustained physical acceptance milestone additionally requires the GUI and logger active under recorded bench conditions. Use [VERIFICATION.md](../VERIFICATION.md) for existing evidence and [ROADMAP.md](../ROADMAP.md) for outstanding work. Older physical captures and successful simulator trials do not establish acceptance for a new build.
