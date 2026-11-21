# EthernetIPWorkbench roadmap

EthernetIPWorkbench hosts Assembly Scanner, a standalone Windows x64 EtherNet/IP scanner. The current GUI can run multiple cyclic Class 1 connections with a separate setup and capture per device. It supports EDS and manual setup, saved profiles, typed assembly mappings, authorized output staging, a GUI and CLI, and selective CSV logging with rotation and loss accounting.

Simulator checks cover that workflow. Physical acceptance of sustained 1 ms capture on a bench adapter remains open. Treat the requested RPI, the intervals returned by Forward_Open, and the measured send and receive spacing as separate results.

## 0.2 Single-device preview

1. [Improve Windows 1 ms producer timing](https://github.com/votecoffee/EthernetIPWorkbench/issues/1). Completed the wait-handle reuse implementation and paired [localhost timing comparison](VERIFICATION.md#windows-wait-handle-reuse) with the GUI and logger active.
2. [Add offline profile preflight](https://github.com/votecoffee/EthernetIPWorkbench/issues/3). The GUI check and CLI `validate` report effective connection settings, output ownership, assembly bounds, EDS-derived sizes, and out-of-range RPI requests before opening I/O. The GUI and `decode-input` also preview mapped values and CSV selection from a saved payload.
3. [Qualify the physical bench path](https://github.com/votecoffee/EthernetIPWorkbench/issues/2). When the bench adapter is available, verify identity and start at 8 ms in input-only mode. A GUI identity read pins the unit for the next manual connection. Attempt three 60-second captures at 1 ms on the direct link. Record the same timing and loss metrics and qualify any intended switched path separately. The [headless qualification runner](README.md#run-the-headless-bench-sequence) collects this evidence without opening a window. It labels its results separately from GUI acceptance. Both paths report observed packet rates, spans, and counts of intervals at or above two and four times the negotiated RPI alongside packet and sample losses. The GUI and CLI enforce timed CSV cutoffs using packet timestamps from the first input. The GUI can save a run report beside the CSV after disconnect and flush. Both reports record the configured CIP timeout code, the receive watchdog calculated from the negotiated input interval, and the separate Forward_Close outcome. Timeout diagnostics also show O-to-T sends and receive-check gaps during input silence. Startup timing shows the explicit exchange, the first O-to-T send, the first received UDP datagram, and the first valid input. Both reports include actual UDP endpoints and Class 1 connection IDs for correlation with a packet trace. Each report also summarizes the values and changes in the saved input columns. The CSV join tool can verify the saved parts against either report.
4. [Package a Windows x64 preview](https://github.com/votecoffee/EthernetIPWorkbench/issues/4). Use the checked-in packager to build with locked dependencies. Test the GUI and CLI from a fresh unpacked folder. Include generic samples, licensing, build identity, and the current hardware verification status.

The GUI groups setup, inputs, outputs, logging, mappings, integrations and diagnostics into workflow tabs, with connection review and profile/device management windows.

The [operations guide](docs/OPERATIONS.md), [development guide](docs/DEVELOPMENT.md), root AGENTS.md and tracked Codex skill support future setup and maintenance chats. The portable bundle includes these references and a local skill installer.

The offline timing and profile work can proceed while the bench device is unavailable. Physical qualification requires completed captures with the GUI and logger active, clean close, and no watchdog timeout, local buffer overrun, or unexplained packet loss under the recorded bench conditions. [Single-device milestone](https://github.com/votecoffee/EthernetIPWorkbench/milestone/1)

The [offline CSV timing audit](docs/OPERATIONS.md#audit-saved-capture-timing) checks the saved capture against the run report and measures spacing within each recorded connection generation.

The [GUI qualification review](docs/OPERATIONS.md#review-saved-gui-qualification-runs) checks the four saved runs together and records issues per trial alongside the timing and CSV measurements.

## 0.3 Multiple adapters

- [Run two isolated scanner sessions](https://github.com/votecoffee/EthernetIPWorkbench/issues/5). The GUI keeps each adapter's setup, sockets, identity, staged outputs, and write authorization separate. The [dual-adapter loopback check](VERIFICATION.md#concurrent-adapter-sessions) exercises a receive timeout while the other session continues.
- [Capture and recover streams independently](https://github.com/votecoffee/EthernetIPWorkbench/issues/6). Each device has its own bounded CSV queue, rotation, loss counters, and reconnect generations. Optional reconnect verifies the adapter's identity on every attempt. The GUI run report retains the timing, errors, and final state of each capture generation. The [headless recovery check](VERIFICATION.md#headless-recovery) exercises reconnect and writer failure alongside a healthy stream.

The milestone requires concurrent loopback coverage and compatibility with existing single-device profiles. [Multiple-adapter milestone](https://github.com/votecoffee/EthernetIPWorkbench/milestone/2)

Read-only software integration offers a loopback HTTP API, MQTT publishing, and a Windows shared-memory snapshot. Headless `serve` runs the feeds without CSV and keeps them open across a bounded input-only reconnect. HTTP supports bounded concurrent readers and exposes live Class 1 diagnostics and the preceding generation's timeout and close result. Run a physical bench comparison to quantify latency and loss with these feeds enabled.

The [paired localhost benchmark](docs/OPERATIONS.md#benchmark-feeds-on-localhost) captures timing and loss with all three feeds enabled and disabled. It alternates run order and retains separate consumer observations for HTTP, MQTT, and shared memory.

The C# examples demonstrate HTTP and shared-memory latest values, retained HTTP history with cursor recovery, and MQTT subscription with broker reconnect. [HTTP and shared-memory example](examples/dotnet/README.md) · [MQTT example](examples/dotnet-mqtt/README.md)

## 0.4 Adapter compatibility

- Suggest input mappings from explicit, fixed-width layouts in EDS connection profiles. The GUI can add reviewed suggestions without changing CSV selection. The CLI reports the raw mappings for reuse in saved settings.
- Suggest output mappings from declared O→T layouts in Exclusive Owner profiles. The GUI adds reviewed field definitions for typed staging and the CLI reports the same definitions offline.
- Suggest packed Boolean mappings from explicit one-bit BOOL members in EDS input and output layouts.
- Compare the selected EDS device identity with the live adapter in GUI connection review and the CLI identity read.
- Preserve the selected EDS metadata in named TOML exports so the GUI and CLI can compare effective settings offline from the TOML alone.
- Refresh a matching EDS connection in Device while retaining the effective settings, mappings and staged bytes.
- Compare saved EDS metadata with a revised file offline, including changed typed field offsets and refresh eligibility.
- Resolve nested `AssemN` members for cyclic payload sizes and mapping suggestions. Repeated fields receive distinct names at their final payload offsets.
- Stage typed configuration fields from a supported EDS target assembly using a complete local configuration buffer.
- Resolve omitted cyclic payload sizes from complete EDS assembly member lists. The GUI and CLI identify the result as a layout width for bench review.

- [Verify a second documented adapter family](https://github.com/votecoffee/EthernetIPWorkbench/issues/7). The [OpENer software adapter check](VERIFICATION.md#opener-compatibility) covers the upstream EDS, identity, assembly decoding, and sustained unicast input-only capture. Physical device acceptance remains separate.
- Discovery, the direct CIP read browser, saved read requests and Large Forward_Open have localhost coverage. Exact and compatible format 4 keys have independent OpENer software coverage. Verify them against documented physical adapters.
- Explore and `class-info` inspect the selected class's revision, instance bounds and advertised optional attributes and services through direct or routed reads. The reports preserve optional-field rejections and decode errors.
- Explore, CLI and Rust support a reviewed Set_Attribute_Single write with a full identity check. Simulator coverage includes direct and routed writes, lost acknowledgments and independent OpENer readback. Verify any intended physical writable attributes on an authorized bench.
- Optional same-session readback compares an acknowledged write with the returned attribute bytes. Reports retain mismatches, transport errors and target or router status separately from the acknowledgment.
- Stage complete typed attribute values with configurable byte order, word swaps and fixed-width UTF-8. Explore and CLI show the exact bytes before sending.
- Require an optional exact current-value comparison on the identity-checked session before sending. Explore and CLI show the comparison separately from the write and readback outcomes.
- Run reviewed Set_Attribute_Single requests over direct or routed Class 3, with standard or Large Forward_Open and a separate close outcome. Verify on physical targets and bridges that support connected writes.
- Save exact staged write bytes and options in TOML drafts shared by Explore and CLI. Reopen drafts offline and require a fresh identity review before applying.
- Inspect the current attribute with the write draft's transport. Stage reviewed bytes as an expected value in Explore and reread them before apply.
- Export the latest Explore write receipt for audit, with the exact staged bytes, completion time and separate write, verification and Forward_Close outcomes.
- Check the current value from an Explore or saved write receipt using the reviewed device identity. Keep the later comparison separate from the original write outcome.
- Reopen and inspect saved GUI or CLI receipts offline in Explore. Start the identity-checked follow-up explicitly from the loaded receipt.
- Save completed current-value checks as timestamped JSON from both Explore receipt views. Preserve the historical outcome and any separate Class 3 close in the export.
- Reopen a saved current-value check without a device connection in Explore or the CLI. Validate the recorded identity, bytes and comparison before displaying it.
- Saved CIP reads support an optional typed reply field with zero-based offsets, byte order and swaps. The GUI and CLI retain the raw response for review.
- Run all saved CIP requests in profile order on one explicit session from Explore or the CLI. Both report replies, rejections and unattempted requests. Explore can save the batch report as JSON.
- Unconnected Multiple Service Packet groups direct reads or saved reads with one shared route in Explore, CLI and Rust. The localhost fixture covers successful and mixed replies and separate bridge and target rejections. Verify with a physical target and bridge that support the service.
- Direct Class 3 connected reads and Forward_Close have independent OpENer coverage through the CLI and Rust API. Verify on a physical adapter.
- Sequential and Multiple Service Packet reads can run over a routed Class 3 connection when the batch shares one path. Explore and CLI report the connection route and Forward_Close separately from read or packet outcomes. Verify routed operation through a physical bridge and compatible target.
- Class 3 supports saved connection sizes up to 4,002 bytes and uses Large Forward_Open above 511 bytes. Localhost tests cover direct and routed messages beyond the standard size. Verify target and bridge limits on a physical path.
- CIP routes in Explore, the CLI and the Rust API support byte, IPv4 and hexadecimal links with decimal port IDs up to 65535. Windows loopback covers extended port identifiers and hexadecimal links in Unconnected Send, Class 3, and Class 1. Verify the documented routes through a physical bridge.
- Explore and `port-info` read the CIP Port Object directory and selected instances. Verify the reported port numbers, local node addresses and routing capabilities on supported bench devices.
- Explore and `tcpip-info` read TCP/IP Interface settings on direct or routed targets. Compare reported startup mode, network addresses, hostname, and Physical Link Object path with the bench device's settings.
- Explore and `link-info` read Ethernet Link speed, flags, MAC address and 32-bit counters on direct or routed targets. On physical devices, compare the selected port and error counters against the device diagnostics.
- Explore and `qos-info` read the device's reported 802.1Q tag enable and DSCP values. Compare those settings against a packet trace on the intended physical network.
- Explore and `manager-info` read Connection Manager open, close, rejection, and timeout counters. On physical adapters that expose the counters, compare the reported totals with the device diagnostics and the scanner's session report.
- Explore and `assembly-info` read Assembly Object size and data and flag when the explicit replies disagree. On physical adapters, compare their separate lengths with the documented and negotiated cyclic payload.
- Explore and `check-assemblies` compare reported assembly sizes with a profile's fixed or variable-length payloads once per instance. Verify the read-only check on the intended physical adapter and bridge before exercising the matching cyclic profile.
- Explore and `list-services` query the addressed Ethernet endpoint for its advertised encapsulation version and capabilities over unicast UDP or TCP. Compare the result with the physical adapter's documentation and cyclic acceptance.
- Explore and `discover` support UDP broadcast and unicast ListIdentity, plus TCP ListIdentity for a known address. Compare the reported peer and advertised endpoint on the intended bench topology.
- Explore and `list-objects` read the optional Message Router Object List on direct and routed targets. Verify the reported classes on physical devices that implement the attribute.
- Variable-size T→O supports standard and Large Forward_Open, offline decoding, CSV and all three export feeds. Verify it on a physical adapter that produces variable-size assemblies.
- Multicast T→O uses the adapter's group, port and connection ID with unicast O→T. Verify group membership and capture through a physical bench switch.
- Listen-only profiles use the documented heartbeat assembly and read an existing multicast producer without taking output ownership. Verify against a physical adapter with a controlling multicast connection.
- Variable-size O→T supports standard and Large Forward_Open with staged lengths and the existing owner authorization. Verify against a physical adapter that consumes variable-size assemblies.
- Routed unicast input-only and Exclusive Owner support target identity, standard and Large Forward_Open, and matched Forward_Close through the first bridge. The owner path requires full identity and serial verification and separate write approval. The loopback bridge checks routed ownership and staged output. Direct cyclic ownership also has independent OpENer coverage. Verify both roles on a physical route where the first bridge handles scanner-side UDP traffic.
- Routed multicast T→O supports controlling input-only, listen-only and Exclusive Owner profiles. The localhost bridge exercises group selection and source validation with standard and keyed Large Forward_Open for input-only. Routed standard and Large listen-only attach to the same OpENer producer across successive listener opens and closes. Routed multicast Exclusive Owner has an OpENer test for staged outputs, configuration readback and confirmed close. Verify the bridge's multicast source and group on the intended bench network.

- Default unicast UDP 2222 supports adapters that expect Forward_Open without the T→O socket-address item. Verify the required mode on each bench adapter. Windows loopback covers direct and routed opens, and OpENer checks the default-port capture path.

For bench interoperability, record the make, model, firmware, EDS revision, scanner NIC and physical topology with each run. Start with direct input-only where supported, then repeat on the intended bridge path. With separate approval for exact outputs and configuration, check Exclusive Owner by changing one documented field and observing an independent readback. Use the [hardware verification sequence](VERIFICATION.md#physical-bench-acceptance) to record requested and negotiated intervals, packet and capture losses, and confirmed close.

Device profiles, vendor files, and raw captures belong in the ignored `.local/` directory. [Adapter-compatibility milestone](https://github.com/votecoffee/EthernetIPWorkbench/milestone/3)
