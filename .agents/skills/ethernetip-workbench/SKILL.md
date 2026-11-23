---
name: ethernetip-workbench
description: Configure, operate, troubleshoot or extend EthernetIPWorkbench, the Windows Rust EtherNet/IP Class 1 scanner, including EDS profiles, assembly mapping, CSV capture and HTTP, MQTT or shared-memory consumers.
---

# EthernetIPWorkbench

Use this skill for this scanner application and its exported input data. Resolve paths against the checkout or unpacked bundle selected for the task. In this repository the root is three directories above this SKILL.md. A local installed entry point records the root for chats working elsewhere.

Read only the relevant references.

- For setup, EDS/manual configuration, payload decoding, capture or troubleshooting, read [docs/OPERATIONS.md](../../../docs/OPERATIONS.md).
- For HTTP, MQTT and shared-memory configuration, use [README.md, Connect other software](../../../README.md#connect-other-software). The operations guide describes freshness and capture ownership.
- For code changes, source ownership, tests and packaging, read [docs/DEVELOPMENT.md](../../../docs/DEVELOPMENT.md) and the root [AGENTS.md](../../../AGENTS.md).
- For measurement claims, inspect [VERIFICATION.md](../../../VERIFICATION.md) and the actual run receipt. [ROADMAP.md](../../../ROADMAP.md) records remaining physical acceptance.

## Start from current evidence

Find the actual checkout or bundle and the executable to run. Check Git state and remote when source is available. The repository was renamed from EIPMaster. Commands still use eipmaster-cli.exe and assembly-scanner.exe, and the saved GUI workspace retains its EIPMaster path. A source update does not update an already extracted executable.

Use the profile and installed EDS supplied for the task. Ask for device identity, NIC address, payload sizes or missing mappings only when the available files and context do not resolve them. An EDS assembly definition describes layout, which may exceed the payload currently configured on the hardware.

Review **Mappings → EDS input mapping suggestions** or the CLI `eds` report's `input_layout` for raw typed members. Explicit one-bit BOOL members report a byte offset and a zero-based bit index. Verify the proposed locations, byte order and scaling with the device documentation. Adding suggestions in the GUI preserves the CSV columns until the operator selects the new fields.

For an Exclusive Owner profile, review **EDS output mapping suggestions** or the CLI `eds` report's `output_layout` for typed output fields. Review output values through the existing staging and authorization workflow.

An EDS size with a `member widths` source was calculated from the assembly layout. Confirm it against the device configuration before opening either cyclic direction.

Check the advertised O→T and T→O RPI bounds in **Offline profile check** or the CLI `validate` report's `eds_rpi` tables. Treat a bound warning as advisory and record Forward_Open actual intervals and observed packet spacing separately during bench qualification.

A named TOML saved with an EDS connection selected includes that profile in `[eds_profile]`. Load it to restore the effective settings and the EDS comparison. Run `validate` with the TOML alone to use the snapshot, or pass the current EDS and connection ID to compare against a vendor update.

Use **Device → Refresh EDS metadata** with the current EDS path to update the selected profile while retaining effective settings and staged bytes. Review the offline comparison and save the named TOML again.

Before refreshing, use `compare-eds <settings.toml> <updated.eds>` to review changes to the imported declarations, RPI limits and typed layouts. Check `refresh_allowed` in the output. Use `validate` with the updated EDS to compare the working overrides.

For nested `AssemN` members, review the final offsets and suffixed names in the mapping suggestions. Use a payload with distinct values in each repeated group when testing the mapping.

For a supported target configuration layout, review the CLI `eds` report's `configuration_layout` and use **Device → Configuration and advanced timing → Stage typed configuration values**. Start from the complete configuration hex buffer and verify the exact staged bytes in connection review.

## Choose the entry point

- Run CLI eds, validate or decode-input for offline work. validate returns connection validity and capture validity separately. Inspect both.
- Use Explore or CLI discover, read and read-saved for approved discovery and read-only CIP inspection. For routed explicit reads, use the first bridge address and a documented port/link path. Use named TOML profiles to preserve saved requests. Use the development guide to reuse the Rust library without the GUI.
- Choose **Discovery TCP** in Explore or run `discover <local-ipv4> <destination-ipv4> --tcp` for a sessionless identity probe of a known endpoint. Use **Discovery UDP** for broadcast. Use the responder IP and **Read identity** when preparing a cyclic connection.
- Use **List services** in Explore or `list-services <adapter-or-bridge-ipv4> <local-ipv4> [--tcp]` to query the addressed endpoint's advertised encapsulation capabilities. The query uses the address from Device and defaults to unicast UDP. Choose **TCP** beside the button to use a TCP probe.
- Use **Read class info** in Explore or `class-info <adapter-or-bridge-ipv4> <local-ipv4> <class> [--route <path>]` to read the selected class's common attributes. Review each result for supported attributes, CIP statuses and raw data.
- Use **Attribute list read** in Explore or `read-attribute-list <adapter-or-bridge-ipv4> <local-ipv4> <class> <instance> <id:reply-bytes>... [--route <path>]` to read several fixed-width attributes from one CIP object. Use documented reply lengths. A partial success preserves successful values and individual CIP rejections and returns CLI exit code 1.
- Use **Browse attributes** in Explore to select a common attribute or one advertised by **Read class info** for the current class, endpoint, NIC and route. The picker fills the read form. Review the device reply after selecting **Read**.
- Use **List objects** in Explore or `list-objects <adapter-or-bridge-ipv4> <local-ipv4> [--route <path>]` for the optional Message Router Object List. Select an advertised class to populate the class read form.
- Use **Find instances** and **Next instances** in Explore, or `list-instances <adapter-or-bridge-ipv4> <local-ipv4> <class> [<after-instance>] [--route <path>]` to browse reported instances of one class. Pass the CLI's `next_after` value as the next cursor and review the per-page reply.
- Use **Read port info** after selecting class `0xF4` in Explore, or run `port-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. Use instance 0 for the directory. Review the reported port number and routing capabilities on a specific instance.
- Choose **Network objects → TCP/IP Interface · Status** and **Read interface info** in Explore, or run `tcpip-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]` with a nonzero interface instance. Review the target's reported network settings, startup method, and Physical Link Object path alongside the raw replies.
- In that inspection, enable **Include multicast and session settings** or pass `--diagnostics` to read multicast allocation and TTL, the pending flag, and the TCP encapsulation inactivity timeout. Use the Forward_Open group and cyclic receive watchdog from the capture report for Class 1 troubleshooting.
- Select **Read address conflicts** in the TCP/IP Interface view or run `acd-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. Review capability, enablement, and the current flags alongside the last recorded remote MAC and ARP packet.
- Choose **Network objects → Ethernet Link · Speed** and **Read link info** in Explore, or run `link-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]` with a nonzero link instance. Review reported speed, link flags, MAC address and any supported cumulative counters. Keep CIP rejections distinct from zero counters.
- Choose **Network objects → LLDP neighbors**, use **Find instances**, then select a row and **Read LLDP neighbor**. The CLI equivalents are `list-instances <adapter-or-bridge-ipv4> <local-ipv4> 0x10A [--route <path>]` and `lldp-neighbor <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. Read the adapter's recorded neighbor and use its local link instance for `link-info`. Review each attribute outcome when the summary value is unavailable.
- Choose **Network objects → Device Level Ring · Status** and **Read ring status** or run `dlr-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--supervisor-details] [--route <path>]`. Use **Include supervisor details** to read the fault count and reported last active nodes on supporting targets. Use the selected ring instance and review missing values alongside their CIP outcomes.
- Enable **Include port settings and capabilities** or pass `--port-details` to inspect the port label, operating and administrative states, configured link mode, and supported speed/duplex pairs alongside the live speed and flags.
- Choose **Network objects → Quality of Service · Explicit DSCP** and **Read QoS settings** in Explore, or run `qos-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. These values are the adapter's reported settings. Use a packet trace to verify actual markings.
- Choose **Connection Manager** and **Read connection counters** in Explore, or run `manager-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. Read these as cumulative device-reported counters. Preserve unsupported attributes as CIP rejections.
- Under **Connection Manager → Connection data**, enter the target's connection number or the Forward_Open triad in `connection-serial:originator-vendor:originator-serial` order. Use the same selector with `connection-data <adapter-or-bridge-ipv4> <local-ipv4> <selector> [--route <path>]`. Treat the returned interval fields as device-reported diagnostics and retain the capture's negotiated and observed timing separately.
- For a running scan, use **Diagnostics → Adapter connection record** to read with the current generation's verified selector or copy it for the CLI. Read `forward_open_triad` in saved reports or HTTP `/v1/connection` when correlating reconnect generations with packet traces.
- Choose **Common attributes → Assembly · Data** and **Read assembly info** in Explore, or run `assembly-info <adapter-or-bridge-ipv4> <local-ipv4> <instance> [--route <path>]`. Compare the declared size and explicit data length with the documented cyclic payload before connecting.
- Choose **Check profile assembly sizes** in Explore or run `check-assemblies <settings.toml>` to compare reported sizes with the profile's application payload lengths using its cyclic I/O route. Review unavailable attributes and mismatched lengths before cyclic qualification.
- Use **Stage attribute write** in Explore or `write-attribute` for a documented writable attribute. Run the CLI without `--apply` for an offline preview. A live apply requires the reviewed full identity and authorization for the exact path and value. If the target outcome is unconfirmed, verify the target state before issuing another write.
- Use **Enter typed value** or CLI `--type` for a complete attribute value. Inspect the hex preview after choosing byte order, swap or fixed UTF-8 width. Use raw hex for structured attribute data.
- Use **Require current value** or `--expect-current "<hex bytes>"` to compare a readable attribute just before sending. Check `result.current_value` for the returned bytes or read error if the comparison prevents the write.
- Use **Read current from target** under **Require current value** or run `write-draft <draft.toml> --inspect` to read identity and current bytes with the selected route and transport. Review the sample before using it as the comparison value. Apply reads the value again.
- Choose **Use Class 3** or pass `--connected` for a documented connected write. On the CLI, `--connection-size <bytes>` overrides the 504-byte default. Check `result.forward_close` separately from the write acknowledgment and any readback.
- Use **Save draft file** in write review or CLI `write-attribute --save-draft <new.toml>` to save encoded bytes and options offline. Use **Explore → Write drafts** or CLI `write-draft <draft.toml>` to reopen. Review the stored endpoint and exact bytes each time. Use `--verify` to read identity with the saved transport and supply the reviewed full pin separately for `--apply`.
- In **Attribute write result**, use **Save write receipt JSON** to retain the latest outcome in the Logging directory. Keep unconfirmed outcomes and any separate Forward_Close result in the receipt. For CLI writes, save stdout and capture the exit code separately.
- Use **Check current value** in **Attribute write result**, or `write-receipt <receipt.json> --check-current`, to compare a readable attribute against the prior staged bytes after identity verification. Review the later result alongside the original write outcome.
- Use **Explore → Saved write receipts → Open receipt or check** to review a prior GUI or CLI receipt offline. **Check receipt current value** uses the recorded endpoint and identity for the read-only follow-up.
- After a follow-up returns, use **Save current check JSON** or **Save receipt check JSON** to retain it in the Logging directory. `result.checked_utc` in the saved JSON or CLI stdout records when the read-only check finished.
- Reopen exported current-value checks with **Open receipt or check** or offline `write-receipt <check.json>`. Review `result.comparison` for the recorded outcome and use the original write receipt to run a new check.
- For an attribute that supports reads, select **Read back after write** or pass `--readback`. Read the write outcome and `result.readback` separately. A requested comparison must match for CLI apply to return exit code zero.
- In Explore, enable **Decode reply field** to save a documented byte offset, type and byte order with a request. Inspect the raw bytes and any separate `decode_error` in the CLI `read-saved` result.
- Run the whole list on one registered session with **Explore → Saved reads → Read all saved** or `read-saved-all <settings.toml>`. Check the per-request outcomes, including any unattempted requests after an exchange failure.
- Use **Read mode** in **Saved reads** to select the GUI equivalent of the CLI options below. Use **Unconnected sequential** for different routes or mixed direct and routed requests.
- For a target that supports unconnected Multiple Service Packet, pass `--multiple-service-packet` to `read-saved-all` and inspect each embedded reply. Use all direct reads or put the same route on every saved read and address the first bridge.
- Use `read-saved-all <settings.toml> --connected` for Class 3 reads on a target that supports them. For a routed batch, put the same route on every saved read and address the first bridge. Review the individual results, `connection_route` when routed, and `forward_close`.
- Set **Class 3 connection size** in Explore or `[app].cip_class3_size` to request a custom size in both directions, including the sequence. Sizes above 511 bytes require Large Forward_Open support along the path. Omitted values use 504 bytes.
- Combine `--connected --multiple-service-packet` when the target supports the packet service on Class 3. Check `packet_error` separately from any embedded result or close error.
- In **Saved read results**, use **Save JSON** to retain the batch in the directory configured in Logging. It uses the CLI's batch report fields.
- Use CLI identity for an authorized bounded hardware identity read. capture requires an input-only or listen-only heartbeat profile and an expected decimal identity. Include serial to pin a unit. qualify automates the baseline and three target runs for controlling input-only profiles.
- Compare device metadata in **Device identity** after **Read identity**, or add `--eds <file>` to CLI identity. Review differences in the declared product codes and revisions before accepting a live device.
- Use GUI profiles for manual operation and authorized output staging. Start a headless fixture for testing rather than opening the desktop application during unattended work.
- Attach software consumers to the existing connection. CLI serve starts one identity-checked read-only session with the configured feeds and no CSV writer. HTTP offers bounded history. MQTT and shared memory offer latest samples. They do not provide remote output control. C# can consume those interfaces or supervise the CLI. A native C ABI is still a proposed extension.
- Use HTTP GET /v1/connection for live state, requested and negotiated intervals, packet timing and the preceding generation's timeout and close result.
- Retry HTTP 503 or a closed request with backoff while the feed is running. Keep the last successful history cursor across reader retries.
- Start a C# integration from the [.NET 8 feed reader](../../../examples/dotnet/README.md). It shows live generation and age checks for HTTP and shared memory, and a cursor for retained HTTP history.
- Use the [.NET 8 MQTT reader](../../../examples/dotnet-mqtt/README.md) for exact-topic subscription, estimated freshness and broker reconnect.

## Preserve meaning and authorization

Use zero-based byte offsets for mappings. Apply the profile's actual endian and swap settings through model::Selection. Use decode-input with distinct synthetic field values to detect overlapping or incorrect offsets. Engineering-unit scaling belongs to the device's documented interpretation, which must be checked separately from raw integer decoding.

For variable-size T→O profiles, verify the maximum `input_size` and local `input_min_size` against the adapter documentation. Validate all input mappings at the minimum. Check the actual payload length in the input view or export before reporting a readback.

For variable-size O→T, verify the documented `output_size` maximum and local `output_min_size` before staging. The staged hex length is the payload length that an authorized owner session will send. Review the exact bytes and length before connecting or applying outputs.

For multicast T→O profiles, verify the selected NIC, the returned group and receive port in `cyclic_io`, and the Forward_Close result. O→T stays unicast and the usual ownership rules apply.

For a unicast profile that expects the default UDP port, use **Use default unicast UDP 2222** or set `default_unicast_port = true` with `local_udp_port` set to 0 or 2222. This binds UDP 2222 on the selected NIC and omits the T→O socket-address item. Verify the effective port in preflight and the actual endpoints in the capture report.

For listen-only, verify the adapter's distinct heartbeat connection point and an existing controlling multicast connection. Keep output and configuration payloads empty and O→T Run/Idle disabled. The listener uses manual reconnect.

For routed Class 1, set `device_ip` to the first bridge and `io_route` to the documented port/link pairs. Decimal port IDs 1–65535 work with byte, unicast IPv4 or documented hexadecimal links such as `4660/hex:0080FE`. Verify the target with Device → Read identity or CLI identity with --route. A controlling input-only profile can use unicast or multicast T→O. A routed listen-only profile attaches to an existing multicast producer through the bridge. For multicast confirm that the first bridge sources the input packets and check the returned group and receive port. Routed Exclusive Owner uses unicast O→T and the target profile's unicast or multicast T→O. It requires the target's full identity and serial pin plus separate output and configuration authorization. The saved Explore read routes are independent.

Review the electronic key and mode in connection review and offline preflight. Use the documented revision with `[electronic_key].compatible = true` when the adapter supports compatible revision matching. Verify the unit identity separately.

For live work, use the session's authorization and an isolated bench. Confirm the exact ownership and output/configuration actions when they are not already authorized. Configuration travels in Forward_Open and an owner may start transmitting staged output immediately.

Separate requested RPI, Forward_Open actual intervals, observed socket spacing and disk/export loss. Preserve captures and failure receipts. Do not infer that skipped producer slots are received sequence gaps. Keep .local/ private, and keep desktop interaction subject to an explicit request in the current chat.

Use `tools/join_capture.py` with `--report` and `--timing` to check existing CSV evidence without opening a connection. Saved-row spacing excludes time between generations. Interpret it with the filtering, writer loss, and socket diagnostics in the same report.

Use `tools/review_gui_qualification.py` with the verified identity and declared link path to review the four saved GUI runs together. Keep each report beside its original CSV parts.

Use `tools/benchmark_integration.py` for paired localhost timing checks with software feeds. Interpret sample age and skipped indexes as consumer observations and use scanner and fixture counters for packet timing and loss.

## Finish with a reproducible result

Give the exact profile, executable or commit and capture/report paths relevant to the task. State whether results came from offline decoding, a software adapter or a physical device. For code or documentation changes, run the relevant checks and maintain the command and schema documentation. The skill does not grant hardware, output or publishing permission beyond the user's current authorization.
