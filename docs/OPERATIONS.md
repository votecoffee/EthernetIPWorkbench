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

Enable **Decode reply field** before selecting **Read** to interpret one documented field. The offset starts at zero in the CIP reply data. The editor supports the same types, bit indexes, fixed text lengths, endian and swap settings as assembly mappings. **Save / update read** retains the selection. In TOML, place the optional `selection` key within `[[app.cip_reads]]` before `[app.cip_reads.path]`.

~~~toml
selection = { offset = 2, kind = "I32", big_endian = true, swap = "word_order" }
~~~

`read-saved` returns the selection, the decoded value as text, and the raw `data_hex`. A successful CIP read with a short reply still returns its bytes and a successful exit code. The JSON includes `decoded: null` and a separate `decode_error`. With no selection the existing standard attribute descriptions still apply.

### Inspect a CIP class

In **Explore**, enter a class ID or select a common attribute to fill in its class. Set **CIP route** when reading through a bridge, then select **Read class info**. The inspection queries class instance zero with seven Get_Attribute_Single requests on one TCP session. The results show the revision, maximum instance ID, instance count, optional instance attributes and services, and maximum attribute IDs. Maximum IDs describe bounds. Use the device manual for the implemented instance and attribute addresses. [Common class attribute formats](https://assets.danfoss.com/documents/366937/BC361183001812en-000201.pdf) · [EtherNet/IP class attribute example](https://product-help.schneider-electric.com/Machine%20Expert/V1.1/en/tm3bcprg/tm3bcprg/TM3_Ethernet_Bus_Coupler/TM3_Ethernet_Bus_Coupler-6.htm)

~~~powershell
.\target\release\eipmaster-cli.exe class-info 127.0.0.1 127.0.0.1 0x01
.\target\release\eipmaster-cli.exe class-info 192.0.2.10 192.0.2.11 0x04 --route 1/3
~~~

The routed command takes the first bridge's address. The JSON keeps each reply's raw bytes, decoded value or decode error, and CIP status. The inspection returns exit code 0 when at least one field decodes and the only rejections are target statuses `0x08` or `0x14` for unsupported services or attributes. A router rejection, exchange failure, undecodable reply, or lack of a decoded value returns exit code 1.

### Review and send an attribute write

Use the documented writable class, instance and attribute. In **Explore**, enter the address and optional CIP route, and select **Stage attribute write**. Enter space-separated hex or select **Enter typed value**, choose a format, and inspect the encoded bytes. Select **Verify target identity**, then review the device's name and identity, the NIC, address, route and exact bytes before selecting **Send reviewed write**. Changing the value or its encoding clears the prior review. The send checks the full identity on the same registered session that carries the write. [Set_Attribute_Single reference](https://product-help.schneider-electric.com/Machine%20Expert/V2.2/en/EtherNetIP_Services/EtherNetIP_Services/modules/set-attribute-si.html)

Both commands below preview the same bytes offline. The typed report also retains the entered value and encoding.

~~~powershell
.\target\release\eipmaster-cli.exe write-attribute 127.0.0.1 127.0.0.1 4 154 3 "A5 16 2C 93"
.\target\release\eipmaster-cli.exe write-attribute 127.0.0.1 127.0.0.1 4 154 3 747873558 --type U32 --endian big --swap word_order
~~~

Typed entry encodes a complete attribute value. Choose `Bool`, signed or unsigned 8-, 16-, 32- or 64-bit integers such as `I16` or `U32`, `F32`, `F64`, or `Utf8`. Numeric values default to little endian. Use `--endian big` and `--swap bytes_in_words` or `--swap word_order` when specified by the device. `Utf8` encodes a fixed-width field padded with zero bytes to the width given by `--text-bytes`. Supply raw hex for a structured or length-prefixed attribute.

Add `--apply <vendor:type:product:serial>` to send to the reviewed target. All four identity fields are decimal. To write through a bridge, use its IP as the first address and pass `--route <port/link/...>`. The default transport is unconnected CIP on TCP. Live writes require authorization for the exact target and value.

For a readable attribute, select **Require current value** and enter the expected bytes in hex. Changing these bytes clears the review. The CLI accepts `--expect-current "10 20 30 40"`. Preview records those bytes offline. On apply, the Workbench verifies identity and reads the attribute once on the same session. It sends the write only when the returned bytes match exactly. The comparison and write are separate requests. Another client can change the attribute between them.

The optional `result.current_value` retains `expected_hex` and the comparison outcome, along with the returned `data_hex` or read error. A current-value mismatch or read failure produces `not_sent` and exit code 1. A requested check reports `skipped` when identity verification prevents it.

| Result | Meaning |
|---|---|
| `preview` | Validated offline. |
| `not_sent` | Session setup, identity verification or the current-value check prevented the write. |
| `acknowledged` | The target acknowledged the service. An explicit read of a readable attribute can verify the resulting value. |
| `rejected` | The target returned a CIP error with its General and Extended Status. |
| `router_error` | The router returned a CIP error. The target outcome is unconfirmed. |
| `unconfirmed` | The exchange ended without a valid target response. The value may have been applied. |

For a readable attribute, enable **Read back after write** in the review window or add `--readback` to the CLI command. The Workbench sends one Get_Attribute_Single for the same address and route on the write session after the target acknowledges the write. It compares the returned bytes with the staged value. A preview with `--readback` adds `readback_requested` to the offline JSON. [Get_Attribute_Single reference](https://product-help.schneider-electric.com/Machine%20Expert/V1.2/en/lmcprg/lmcprg/Ethernet_Configuration/Ethernet_Configuration-17.htm)

The optional `result.readback` records `matched` or `mismatch` with the returned `data_hex`. Target and router rejections retain their status words. A failed read exchange reports `unavailable`. A requested readback reports `skipped` when the write lacks a target acknowledgment. Explore shows both results and the returned bytes under **Attribute write result**.

The CLI returns exit code 0 for a valid preview or an acknowledged apply. With `--readback`, an apply also requires a byte-for-byte match for exit code 0. The write acknowledgment stays in the result even when the readback differs or fails. For an unconfirmed write, verify the target state before issuing another write. The Workbench sends each reviewed write at most once.

#### Send a Class 3 attribute write

Select **Use Class 3** in the write review or add `--connected` to `write-attribute`. The review window starts with the profile's Class 3 connection size. Set **Connection size** for this write if needed. Changing the transport or size clears the prior review. The CLI defaults to 504 bytes per direction and accepts `--connection-size <bytes>` with `--connected`. Sizes above 511 bytes use Large Forward_Open. The size includes the two-byte sequence. [Connected explicit messaging reference](https://product-help.schneider-electric.com/Machine%20Expert/V1.1/en/ESMEEtherNetIP/ESMEEtherNetIP/SoM_Industrial_Ethernet_-_EIP_Scanner_Library/SoM_Industrial_Ethernet_-_EIP_Scanner_Library-5.htm)

~~~powershell
.\target\release\eipmaster-cli.exe write-attribute 127.0.0.1 127.0.0.1 4 154 3 "A5 16 2C 93" --connected --expect-current "10 20 30 40" --readback
~~~

The example previews the write offline. Its JSON includes `class3_size` and `large_forward_open`. With `--apply`, the Workbench uses the supplied route when opening and closing the Class 3 connection. It verifies the target identity and performs any requested current-value check, the write, and any requested readback on that connection.

An applied Class 3 write reports `result.forward_close` when the scanner attempted to close the connection. Exit code 0 requires the write acknowledgment, a matching readback if requested, and a confirmed close. Check the write outcome before acting on a nonzero exit code. A failed close can accompany an acknowledged write.

#### Save and reopen a write draft

In the write review, open **Save draft file**, enter a new TOML path, and select **Save write draft**. This validates the draft offline. To reuse it, open **Explore → Write drafts**, enter the path, and select **Open write draft**. The review opens at the adapter and NIC addresses from the file. The active device profile stays unchanged. Inspect the address, route and bytes, then select **Verify target identity** before sending.

The CLI can save or preview the same files offline. Use an existing directory and a new filename for the save.

~~~powershell
.\target\release\eipmaster-cli.exe write-attribute 127.0.0.1 127.0.0.1 4 154 3 "A5 16 2C 93" --expect-current "10 20 30 40" --readback --connected --save-draft .\.local\loopback-write.toml
.\target\release\eipmaster-cli.exe write-draft .\.local\loopback-write.toml
~~~

For an authorized identity read, add `--verify` to `write-draft`. The command uses the staged transport and reports `identity_read` with the observed device identity. To send an authorized write, run `write-draft <draft.toml> --apply <vendor:type:product:serial>` with the reviewed decimal identity. Apply checks that identity again on the write session. Save, verify and apply are separate commands.

The version 1 file stores the adapter and NIC, the CIP path, an optional route, the exact encoded hex, any expected current hex, the readback choice and an optional Class 3 size. A typed value reopens as its encoded hex. Identity and approval state are session-specific. Keep device-specific drafts in ignored `.local/` or in a separate private location.

#### Read the current value before review

For a readable attribute, select **Require current value → Read current from target** in the write review. Explore reads the device identity and attribute bytes on one session using the draft's route and transport. Review the returned identity and bytes, then select **Use as expected value** to stage them for the comparison. Select **Verify target identity** and review the write before sending. Apply reads the value again immediately before sending. If it changed, the write is skipped.

Run `write-draft <draft.toml> --inspect` for the same read-only inspection from the CLI. The JSON reports `result.observed_identity` and `result.current`, including the raw `data_hex` or the target, router or exchange error. A Class 3 result also reports `result.forward_close`. Exit code 0 requires a successful read and, for Class 3, confirmed close. The draft file stays intact.

#### Save an attribute write receipt

Set the output directory in **Logging**. After an apply attempt returns, expand **Explore → Attribute write result** and select **Save write receipt JSON**. Explore saves the latest result as a new `attribute-write-*.json` file. Save a result before applying the next write if you want to retain both.

The receipt uses the CLI's write-result fields for the exact staged bytes, addresses, route, transport and result. It adds `completed_utc`, the time the Workbench finished the attempt, including any readback or Forward_Close. The result preserves the reviewed and observed identities and distinguishes the write outcome from the current-value check, readback and close. An unconfirmed outcome still means the device may have applied the write.

Saving uses the completed result already in Explore. A disk error leaves that result available so you can select a working directory and save it there. For CLI writes, retain the JSON from stdout and record the exit code separately.

#### Check the current value from a receipt

For a readable attribute, select **Check current value** under **Explore → Attribute write result**. Workbench uses the original address, route and transport. It verifies the recorded device identity and reads the attribute on that session only when the identity matches. The comparison and any Class 3 Forward_Close outcome appear separately from the original write result.

Use `write-receipt <receipt.json>` to preview a completed GUI or CLI write receipt offline. For an authorized read, add `--check-current`. The command reports the original write outcome in `original_write` and the follow-up in `result`. Check `result.identity_matches`, `result.comparison` and, for Class 3, `result.forward_close`. Exit code 0 requires matching identity and current bytes, plus confirmed close for Class 3. You can save the command's stdout as a separate follow-up record.

A later match describes the bytes at the time of that check. An original unconfirmed outcome stays unconfirmed. The check is read-only and the receipt file stays intact.

#### Reopen a write receipt

In **Explore → Saved write receipts**, enter a path to a completed GUI or CLI write receipt and select **Open receipt or check**. The panel shows the file path, recorded outcome, completion time when present, endpoint, reviewed identity, and staged bytes. Opening a file is offline.

Select **Check receipt current value** to run the read-only follow-up against the saved endpoint. The check uses the recorded NIC, route, attribute, transport and identity. The panel displays the later comparison alongside the historical outcome.

#### Save a current-value check

Set the output directory in **Logging**. After a check returns, select **Save current check JSON** under **Attribute write result** or **Save receipt check JSON** under **Saved write receipts**. Each export creates a new `write-current-check-*.json` file from the displayed result.

The GUI export and `write-receipt --check-current` stdout use the same JSON fields. `original_write` gives the recorded outcome and reviewed identity. `completed_utc` records the original write completion when available. `result.checked_utc` records when the follow-up finished, including any Class 3 close. The observed identity, byte comparison and any close outcome remain under `result`. An unconfirmed original write retains that status even if the later bytes match.

#### Review a saved current-value check

Open a `write-current-check-*.json` file or a saved copy of CLI follow-up stdout using **Explore → Saved write receipts → Open receipt or check**. The panel shows the historical write outcome and the saved check's time, identity, observed bytes, comparison and close outcome. Use the original write receipt to run a new follow-up.

Use `write-receipt <check.json>` for offline CLI review. Exit code 0 means the file passed validation. Read `result.comparison` for the recorded match, mismatch or error.

### Run all saved reads

In **Explore**, expand **Saved reads**, choose a **Read mode**, and select **Read all saved**. The batch runs on the configured adapter or bridge and NIC in **Device**. The mode starts at **Unconnected sequential** in each new application session.

| Read mode | CLI options | Use |
|---|---|---|
| Unconnected sequential | None | Direct or routed reads in profile order |
| Unconnected Multiple Service Packet | `--multiple-service-packet` | One CIP request for direct reads or reads with a shared route |
| Class 3 sequential | `--connected` | Ordered reads on one direct or routed Class 3 connection |
| Class 3 Multiple Service Packet | `--connected --multiple-service-packet` | One CIP request on a supported direct or routed Class 3 connection |

Choose **Unconnected sequential** when the list contains different routes or a mix of direct and routed reads. When the job finishes, expand **Saved read results** to see the mode used and each request's decoded value, raw bytes or error. Class 3 results show the Forward_Close outcome and the route when used. A Class 3 Multiple Service Packet error appears above the individual results.

Select **Save JSON** to save the displayed batch, including any failures, in the output directory configured under **Logging**. The filename starts with `saved-reads-` and includes the UTC save time. The app creates a new file for each save, shows its path, and reports any write error while keeping the batch available. The report uses the same JSON fields as `read-saved-all`.

For the same batch headlessly, run:

~~~powershell
.\target\release\eipmaster-cli.exe read-saved-all .local\device.toml
~~~

In the default mode, the GUI and CLI run the profile's saved requests in order on one registered explicit TCP session. Each request uses its own optional route and reply selection. The CLI's JSON `results` array preserves order and includes the request name, path, and any returned data or error.

| Status | Meaning |
|---|---|
| `success` | A valid CIP reply arrived. Check `decode_error` separately when a typed field was selected. |
| `cip_rejected` | The adapter or router returned a CIP error. Check the general and extended status. The session continues with the next saved request. |
| `exchange_failed` | The explicit exchange failed and the session ended. Earlier replies are still in the report. |
| `not_attempted` | The session ended before this request. The CLI runs at most one attempt per request. |

Exit code 0 means every CIP read returned a successful reply. A CIP rejection or lost exchange produces exit code 1 with the per-request report. If the profile contains no requests or the session fails to register, the CLI reports the setup error on stderr.

#### Combine reads in one unconnected packet

~~~powershell
.\target\release\eipmaster-cli.exe read-saved-all .local\device.toml --multiple-service-packet
~~~

Use this option with a target that supports unconnected CIP Multiple Service Packet. Leave every saved read's `route` unset for a direct batch. To route the packet, set the same `route` on every saved read and set the first bridge's address in `device_ip`. For example, assign `route = "1/3"` to every entry to read from slot 3. The scanner sends one Unconnected Send through the bridge and decodes the target's embedded replies in profile order. [Rockwell CIP Multiple Service Packet format](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm020_-en-p.pdf) · [Unconnected Send encoding in pycomm3](https://github.com/ottowayi/pycomm3/blob/master/pycomm3/packets/util.py)

Each embedded CIP rejection keeps its status words in the JSON report and produces exit code 1. Routed embedded replies also include the bridge, route, and `reply_from`. A rejection of the whole packet or an invalid reply table produces a diagnostic on stderr and exit code 1. A routed diagnostic identifies the bridge rejection or target rejection. Hosts and bridges can limit unconnected packet size. For a target that cannot accept a full packet, use the default sequential mode.

#### Run Class 3 connected reads

~~~powershell
.\target\release\eipmaster-cli.exe read-saved-all .local\device.toml --connected
~~~

The target must accept a Class 3 Forward_Open to its Message Router. The default request uses standard Forward_Open and asks for a variable size up to 504 bytes in each direction, including the two-byte sequence. The scanner sends each saved read over TCP in profile order and closes the connection after the batch. [OpENer Class 3 target](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipclass3connection.c)

For a routed batch, set `device_ip` to the first bridge's IPv4 address and put the same `route` on every saved read. The scanner puts that route before the Message Router in both Forward_Open and Forward_Close. The bridge carries the connected read traffic on the same TCP session. This route comes from the saved reads. [pycomm3 Class 3 path construction](https://github.com/ottowayi/pycomm3/blob/master/pycomm3/cip_driver.py)

The JSON includes `forward_close` separately from the read results and includes `connection_route` for routed Class 3 batches. A CIP rejection still allows later reads. An exchange failure leaves later requests unattempted. Exit code 0 requires successful CIP replies and a confirmed close. If Forward_Open fails, the CLI reports the setup error on stderr with the route when used and exits 1.

#### Choose a Class 3 connection size

Select a Class 3 read mode in **Explore → Saved reads** and edit **Class 3 connection size**. The profile saves the size, and the CLI reads the same preference when run with `--connected`. To set it by hand, add the key to the existing `[app]` table or create the table if needed.

~~~toml
[app]
cip_class3_size = 4002
~~~

The Workbench accepts 8–4,002 bytes and uses Large Forward_Open (0x5B) for values above 511. Choose a size supported by the target and every bridge in the path. The two-byte sequence counts toward the requested size. A rejected open shows the service and CIP status words. Some devices return a supported size in an additional status word. [Rockwell Large Forward_Open limits and status words](https://www.rockwellautomation.com/en-fi/docs/rslinx-classic/4-60/rslinx-classic-help-ditamap/software-development-kit/introducing-rslinx-classic-sdk/api-function-reference/cip-communications/understanding-cip-communication/dtl_cip_large_conn_open-rtn-value.html)

The Class 3 JSON fields `class3_size` and `large_forward_open` record the requested size and service used. This setting applies to saved reads over Class 3. Cyclic I/O has its own connection sizes and Forward_Open setting.

#### Combine reads on a Class 3 connection

~~~powershell
.\target\release\eipmaster-cli.exe read-saved-all .local\device.toml --connected --multiple-service-packet
~~~

The target must support Multiple Service Packet on its Class 3 connection. Both flags can appear in either order. The command sends one packet for a direct batch or a batch with one shared route. The default size leaves 502 bytes for the CIP request or reply. Deduct two bytes from a custom size to get the usable CIP limit. Oversized requests are rejected before Forward_Open, and incoming replies must fit the same requested size. [Rockwell CIP Multiple Service Packet format](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm020_-en-p.pdf)

Valid embedded replies and rejections appear in `results` in profile order. A whole-packet rejection or an invalid reply produces a `packet_error` and an empty `results` array. The report keeps the separate `forward_close` result. Exit code 0 requires successful reads and a confirmed close.

## Start offline

~~~powershell
.\target\release\eipmaster-cli.exe --help
.\target\release\eipmaster-cli.exe eds samples\loopback.eds
.\target\release\eipmaster-cli.exe validate samples\mapped-loopback.toml samples\loopback.eds Connection1
.\target\release\eipmaster-cli.exe decode-input samples\mapped-loopback.toml "01 02 03 04"
~~~

eds lists supported connection profiles and unresolved features. validate can compare a manual/saved profile to one EDS connection. Its origins distinguish EDS values, overrides, assembly-definition sizes and manual values. An assembly-definition size needs comparison with the device's configured payload. Check connection_valid and capture_config_valid. The validate exit code follows connection_valid, so a successful exit alone does not establish that CSV selection is valid.

Saving a named TOML setup with an EDS connection selected includes a snapshot of that connection under `[eds_profile]`. The top-level settings retain your effective values and overrides. You can load the TOML or run `validate` with the TOML path alone on another computer. Supply an EDS path and connection ID explicitly to compare against that file instead. To write a plain settings file for an older build, choose **Manual settings** and save a separate copy.

While disconnected and with capture stopped, enter the updated EDS path in **Device** and choose **Refresh EDS metadata**. The refresh requires a unique matching connection ID, the same declared role and assembly path, and agreement on vendor ID, device type and product code where both files declare them. It accepts revision changes. Compare the new defaults and RPI limits in **Offline profile check** and review any updated mapping suggestions in **Mappings**. Save the named profile again to keep the refreshed metadata. Refreshing keeps the effective values, mapping definitions and staged bytes. Enter any pending typed configuration edits again. Confirm bench and write permission again where required. Use **Import defaults** when you intend to load a new connection and apply its defaults.

The `eds` report includes `output_rpi_range` and `input_rpi_range` when the corresponding connection field references a UDINT parameter with usable bounds. The `validate` report puts the comparison under `output.eds_rpi` or `input.eds_rpi`. Each comparison identifies the source parameter and any declared bounds in microseconds. Its status is `within_bounds`, `below_minimum`, or `above_maximum`. These advisory results preserve the requested intervals and the validation exit code. Use **Refresh EDS metadata** to populate these bounds in an older workspace. [Rockwell EDS AOP guidelines, RPI fields](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm002_-en-d.pdf#page=47)

decode-input accepts exactly input_size bytes for fixed-size profiles or a length within the configured bounds for variable-size profiles. Exclude sequence and Run/Idle headers. It lists every mapped field's source bytes, decoded text value and CSV inclusion. The GUI has the same check under Mappings → Preview input payload. With no named input fields, the primary selection appears as value.

### Compare EDS revisions

Use a named TOML saved with an EDS connection selected and the revised vendor file.

~~~powershell
.\target\release\eipmaster-cli.exe compare-eds .local\device.toml .local\updated.eds
~~~

The command finds the same connection ID in the revised EDS and prints a TOML report of changed imported metadata. A change has `previous` and `updated` values when both sides declare it. An added item has only `updated` and a removed item has only `previous`. In `assemblies`, input, output and configuration changes list typed fields by name with their offsets and selections. A rename appears as a removal and an addition.

Check `refresh_allowed` and any `refresh_blocker`. Exit code 0 indicates that comparison completed, including when the revision would be rejected by **Refresh EDS metadata**. An unreadable file, a profile without EDS metadata, or a missing or duplicate connection ID returns a nonzero exit. The comparison runs offline and preserves both files. To see how your effective overrides compare with the revised EDS, use `validate <settings.toml> <updated.eds> <ConnectionN>`.

## Configure a device

Start from a generic sample and use the device's EDS and manual to establish each value.

| Setting | Interpretation |
|---|---|
| device_ip, local_ip | Adapter or first bridge IPv4 and IPv4 assigned to the scanner NIC |
| io_route | Optional port/link pairs for routed Class 1 I/O |
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

### Configure routed Class 1

Use **Device → CIP routing** to set the documented port/link path from the first bridge to the target. In TOML, set `device_ip` to the first bridge's IPv4 address and `io_route = "1/3"` for a target at backplane slot 3. Leave the path blank for a direct connection. Use the target's assemblies, RPIs, transfer format and electronic key. The EDS supplies assembly settings. Enter the route for the installed topology manually.

For routed input-only, use zero output and configuration bytes and disable O→T Run/Idle. Select unicast or multicast T→O according to the documented bridge and device profile. The first bridge receives the scanner's unicast heartbeat and sources the inputs on the selected NIC's segment. For multicast, verify the returned group and receive port in the run report. For routed Exclusive Owner, use unicast. Use **Read target identity** in connection review, verify the target serial, then authorize the displayed output and configuration bytes. Forward_Open can apply the configuration and start sending the staged output buffer. Automatic reconnect is available for verified input-only profiles.

The CLI can read the routed target with `eipmaster-cli identity <bridge-ipv4> <local-ipv4> --route 1/3` and capture input-only using that identity. The scanner rereads the target Identity Object before each Forward_Open and checks the reviewed serial on a routed owner connection. Forward_Open and Forward_Close carry the same routed assembly path. Preflight and capture reports show `io_route`. In the completed run report's `cyclic_io`, check that the output destination and first input source identify the scanner-side bridge.

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

Forward_Open returns the T→O group, UDP port and connection ID. The scanner joins that group on the configured NIC and checks the expected source IP and returned ID. For routed input-only, the expected source is the first bridge. Use **Diagnostics** or the capture report's `cyclic_io` table to see the multicast destination and local receive endpoint. The requested local UDP port still binds the O→T socket. For multicast it also supplies the port hint, which defaults to 2222 when the field is zero. The returned receive port takes precedence.

Windows uses one owning socket per local address and port. The scanner shares its own socket within a session when the returned input port matches its output socket. A port already occupied by another session or application causes an error and a Forward_Close attempt. If the connection opens but inputs time out, verify the returned group, selected NIC and multicast forwarding on the bench network.

### Configure a listen-only connection

Start from the adapter's listen-only EDS profile or select **Connection role → Listen-only** in Device. Enter that profile's documented O→T heartbeat connection point and T→O input point. The role is determined by the connection path the adapter receives. Switching into or out of Listen-only in the GUI clears the old O→T instance so you can enter the correct one.

For TOML set `listen_only = true`, `input_only = true`, and `input_multicast = true`. Use zero output bytes, zero configuration bytes and O→T Run/Idle disabled. Retain the documented configuration assembly instance in the path if the adapter requires one. Apply the RPIs and input layout required by the controlling connection. The listener sends an empty heartbeat and records the adapter's existing multicast stream without requesting output ownership.

Start the controlling multicast connection first. For a routed listener, enter the first bridge's IPv4 address and the target's documented `io_route`. The first bridge must carry the unicast heartbeat and source the multicast traffic on the scanner's segment. Check that the returned group, port and T→O connection ID are those of the existing producer.

CIP general status `0x01` with extended status `0x0119` means the target found no controlling connection for the input assembly. Use normal GUI logging or CLI `capture`. Listen-only uses manual reconnect. CLI `qualify` changes the requested rates and requires a controlling input-only profile. If the controller or another process owns the local multicast receive address and port, use a separate supported host or NIC for the listener.

### Configure an electronic key

In **Device → Configuration and advanced timing**, enable **Use electronic key** and enter the documented identity fields and revision. Exact mode is the default. Choose **Compatible revision matching** when the adapter supports the requested revision in that mode. The Workbench sends the mode in the format 4 key for standard and Large Forward_Open. The adapter decides whether to accept it. The live Identity Object check and serial pin still apply under their existing rules. [OpENer key encoding](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipelectronickey.c#L81) · [OpENer key matching](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionmanager.c#L1263)

Configure the optional `[electronic_key]` TOML table with decimal `vendor`, `device_type`, `product`, `major` and `minor` fields. Set `compatible = true` for compatible mode. Use a nonzero vendor, device type, product code and minor revision in compatible mode, with major revision from 1 to 127. Enter the major revision itself. The encoder sets the compatibility bit. [ODVA compatible key test notes](https://www.odva.org/wp-content/uploads/2023/10/EtherNetIPReleaseNotes.pdf)

The EDS importer preserves the mode from a leading format 4 electronic key. It also handles UTF-16 with a BOM and 16-bit Assembly class segments. Review the imported Large Forward_Open setting when the declared size exceeds the standard limit.

### Review device identity

In the GUI, import the EDS, choose a connection profile, review provenance and edit overrides. Use Manual settings when entering the connection directly. Read identity and verify vendor, device type, product code and serial before connecting. A successful identity read pins the next manual connection to that unit. Changing the addresses or cyclic route, or loading a profile, clears the pin.

Expand **EDS declared identity** in the **Device identity** panel to inspect the imported product name and numbers. After **Read identity**, the panel and **Review connection** show any differences in vendor ID, device type, product code, or major and minor revision. The comparison is advisory and uses the numeric fields available in the file. The live serial identifies the individual device for the identity pin. The product name is displayed for reference. Use **Refresh EDS metadata** to populate these fields in an older workspace. [Rockwell Automation EDS AOP guidelines, Device section](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm002_-en-d.pdf#page=34)

The CLI `eds` report includes an optional `device` table for the file. For an authorized read, run `identity <adapter-or-bridge-ipv4> <local-ipv4> --eds <file>` and add `--route <port/link/...>` when needed. Check `eds_comparison.compared_fields` and `eds_comparison.differences`. The exit status reports whether the identity read succeeded. A comparison with zero fields means the file supplied no usable numeric identity values.

For either cyclic direction, the EDS importer prefers an explicit connection size, then the declared size of its `AssemN` format. If both are blank, it can use the sum of the member bit widths when every width is known and the total is a whole number of bytes. A blank member width can use the declared byte size of a supported fixed-size `ParamN`. **Offline profile check** marks both declared and calculated assembly widths as layout sizes. Verify the configured device payload separately. [Rockwell Automation EDS AOP guidelines, Assembly Size field](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm002_-en-d.pdf#page=54)

An assembly member can also refer to another `AssemN`. If the member width is blank, the referenced assembly supplies it. The importer expands supported nested fields at their final payload offsets and gives repeated names distinct suffixes. A wider explicit member includes trailing padding. A truncated, missing, or circular reference is omitted from the suggestions. When its member width is known, subsequent fields keep their offsets. An unresolved width needs manual review. [ISO/IEC 15745-4 assembly layout, section C.4.1.4.8](https://rags.ru/documents/prod/gost-r_gosudarstvennyj-standart/50/gost_69790.html)

For a connection that names a plain `AssemN` input format with a resolved payload width, **Mappings → EDS input mapping suggestions** previews supported byte-aligned types and explicitly one-bit `BOOL` (`0xC1`) members referenced by `ParamN`. Packed BOOLs use zero-based bit indexes with bit 0 at the least significant bit. An aligned byte BOOL uses a declared one-byte parameter. Padding and unsupported members with known widths count toward later offsets. Strings, other packed types and vendor-specific formats require manual mappings. Adding suggestions preserves existing field definitions and CSV columns. If a primary readback was the only mapped value, it is retained as `value`. Use the CSV checkboxes to include new fields. [ISO/IEC 15745-4 assembly layout, section C.4.1.4.8](https://rags.ru/documents/prod/gost-r_gosudarstvennyj-standart/50/gost_69790.html) · [Rockwell Automation EDS AOP guidelines, one-bit member example](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm002_-en-d.pdf#page=42)

The offline CLI `eds` report includes `input_layout` when it can resolve the layout. Each field has the same `name` and `selection` keys as a profile's `[[input_fields]]`. Review and copy the desired entries into a profile, then use `decode-input` with a known payload to check the values. The suggestions use raw little-endian values. Confirm engineering units and any needed endian or swap overrides against the device manual.

For an Exclusive Owner profile that names a supported `AssemN` O→T format, use **Mappings → EDS output mapping suggestions** to add fields to **Named output mappings**. The preview includes suggestions that fit the configured output payload size and whose names are still available. Offsets start at the assembly payload, after any Run/Idle header. Use **Outputs → Stage typed output values** to edit the staged buffer. A packed BOOL edit sets or clears its assigned bit. Review the exact bytes separately for connection or apply authorization. The CLI reports the same suggestions under `output_layout`. Copy reviewed entries to `[[output_fields]]` in a saved profile.

For a supported target configuration `AssemN` with a known size, open **Device → Configuration and advanced timing → Stage typed configuration values**. Enter the complete configuration hex buffer first. Type the new raw value for a field and select **Stage** to encode it into that buffer. The editor offers supported writable fields and keeps the offsets of read-only members and padding. Use the adapter documentation for engineering limits and review the resulting hex in **Review connection**. Configuration is sent in Forward_Open after write authorization. Save the staged buffer as `config_hex` in a named profile. The CLI `eds` report exposes the offset and type under `configuration_layout`. [Rockwell Automation EDS AOP guidelines, Target Config Format](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm002_-en-d.pdf#page=49)

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

### Audit saved capture timing

Use the capture prefix and either the CLI `capture.toml` or the GUI's `-report.toml`.

~~~powershell
python tools/join_capture.py 'captures/capture-<UTC-prefix>' --report captures/capture.toml --timing
~~~

The command verifies the CSV parts before printing JSON. It calculates spacing from adjacent saved `elapsed_us` timestamps within each generation, using nearest-rank percentiles. It reports the sample-index gaps, writer overruns, and saved packet-sequence gaps separately. Saved-row spacing and packet gaps can reflect on-change filtering or writer loss. Use the socket counters for scanner-observed packet loss and `on_change_suppressed` to identify intentional filtering. Spacing across reconnects is excluded.

Requested and negotiated intervals and O→T/T→O socket timing come from the run report. For a GUI report, `socket` describes the final recorded connection. For a CLI report with reconnects, `socket` contains aggregate counters and final-connection timing, and `connection_attempts` preserves the available per-attempt measurements. Earlier every-packet reports without an `unchanged` counter can be checked when their seen, written, and overrun counts reconcile.

### Review saved GUI qualification runs

Keep each GUI `-report.toml` file beside its CSV parts. Pass the 8 ms baseline report followed by the three 1 ms reports. Use the verified decimal identity in vendor, device type, product code, and serial order. The example uses a loopback fixture identity.

~~~powershell
$reports = @(
    ".local/baseline-8ms/capture-report.toml"
    ".local/trial-1/capture-report.toml"
    ".local/trial-2/capture-report.toml"
    ".local/trial-3/capture-report.toml"
)
python tools/review_gui_qualification.py --link-path loopback --loopback-seconds 1 --expected 0:12:1:305419896 @reports
~~~

Use the actual report filenames. For a physical review, select `direct` or `switched` to label the bench path and supply the verified physical identity. The review checks a timed run of at least 15 seconds at 8 ms and three timed runs of at least 60 seconds at 1 ms. Shorter windows are available for loopback only.

The JSON includes an outcome, issues for each run, requested and negotiated intervals, socket spacing, and the independent CSV timing audit. A passing review requires one clean input-only connection per run, a complete every-packet capture, and matching identity, connection settings, and CSV columns across the runs. Scanner skipped send slots remain visible for review. The GUI can disconnect after the recorder detaches, so extra connection packets beyond the CSV window are expected. Use exit code 0 for a pass and 1 for a failed review. The recorded topology is supplied by the operator. Loopback evidence stays under the loopback label.

## Connect other software

Use [Connect other software](../README.md#connect-other-software) for complete profile keys and the binary layout. Enable feeds per device in [app.integration] or in the GUI. They start with the scanner session, survive automatic reconnect and end when the session ends. Distinct devices need distinct HTTP ports and shared-memory names.

| Interface | Consumer contract |
|---|---|
| HTTP GET /v1/status | Active generation and independent integration counters |
| HTTP GET /v1/connection | Connection diagnostics for the latest and immediately preceding generations |
| HTTP GET /v1/latest | Latest raw/typed sample, age and active generation |
| HTTP GET /v1/samples?after=N | Up to 64 samples from the newest 1,024 retained frames |
| MQTT | Latest new active sample at the configured period, TCP 3.1.1, QoS 0, retain disabled |
| Windows shared memory | Latest raw snapshot in a named 573-byte mapping with a named mutex |

HTTP is bound to 127.0.0.1. MQTT uses a trusted listener or a local broker with a secure bridge. Values in HTTP/MQTT are text to preserve integer width and decoder output. All named input fields are exported independently of CSV selection.

Reset an HTTP cursor when stream_start_utc_us changes. Compare sample indexes and oldest_sample_index to detect lost history. In a latest-value read, compare generation to active_generation and check timestamp/age. active_generation zero means cyclic I/O is inactive. MQTT can miss intermediate updates by design.

Shared-memory readers acquire the lock, copy and promptly release it. Discard an abandoned-mutex snapshot after releasing ownership. Close client handles before restarting a manual session with the same object names. A stale mapping may survive in a client process after the scanner exits.

For C#, use the .NET 8 example to read the latest sample over HTTP or Windows shared memory, or to catch up on retained HTTP history. CLI process supervision can also start and stop a scanner session. A C ABI for embedding the scanner is prospective work. [C# feed reader](../examples/dotnet/README.md)

Use the separate .NET 8 example to subscribe to MQTT with a locked MQTTnet dependency. [C# MQTT feed reader](../examples/dotnet-mqtt/README.md)

### Inspect a live Class 1 connection

~~~powershell
curl.exe --fail http://127.0.0.1:8765/v1/connection
~~~

The schema version 1 response identifies the stream with `stream_start_utc_us` and includes the adapter or first bridge, local IPv4, and optional CIP route. `connection` covers the latest started generation and is null before startup. `previous_connection` retains the immediately preceding generation when a retry starts. `active_generation` is zero during a connection gap. Compare it with `connection.generation` and check the state and input age before treating a session as current.

Each generation reports the Identity Object, connection state, accepted packet and anomaly counters, and the source, destination and connection IDs under `cyclic_io`. Read `requested_ot_us` and `requested_to_us` for the profile intervals, `actual_ot_us` and `actual_to_us` for the Forward_Open reply, and `send_spacing` and `receive_spacing` for measured socket timing. `input_age_us` uses the monotonic receive clock. During cyclic I/O, receive counters and spacing reflect the latest published scanner snapshot. `connection_timeout`, `receive_timeout`, and `forward_close` detail the configured watchdog, an observed timeout and the actual close result. The three error fields carry identity, CIP connection and transport messages separately. Measurements or results that are unavailable appear as null.

The endpoint lives for the feed's lifetime. Use the final GUI run report or CLI receipt for a durable record after shutdown.

### Use concurrent HTTP readers

Each feed can process four HTTP requests concurrently and queue up to eight accepted connections. The 250 ms deadline for complete request headers starts at accept time and includes time in the queue. Each response has its own 250 ms write deadline. At capacity, the listener attempts an HTTP 503 response and closes the extra connection. A connection that exceeds its deadline may close before a response arrives.

On a busy or closed connection, retry with backoff if the feed is still running. For `/v1/samples`, keep the last successfully read cursor and check `oldest_sample_index` on retry.

### Run feeds headlessly

Save an input-only or listen-only profile with at least one enabled `[app.integration]` feed. Use the verified decimal identity of the target. For a software adapter configured with the sample loopback identity, run:

~~~powershell
.\eipmaster-cli.exe serve .\feed-profile.toml "0:12:1:305419896"
~~~

The command runs until Ctrl+C, Ctrl+Break, or a terminal connection failure. An optional final duration in whole seconds starts at session startup and includes time spent opening and reconnecting. Use `auto_reconnect = true` under `[app]` for bounded, serial-pinned retries on controlling input-only profiles. No CSV writer or capture directory is created. The software feeds keep their usual fields and sample indexes.

Check HTTP `/v1/status` or the configured consumer while it runs. At shutdown, `serve` prints a TOML report with requested RPIs, per-generation actual intervals and spacing, Forward_Close outcomes, and final export counters. An orderly console interrupt exits with code 130. A timed run exits zero after receiving input on its final generation and confirming the close. Terminal connection and teardown errors exit nonzero.

### Benchmark feeds on localhost

The Windows benchmark makes paired CLI captures against the independent localhost adapter. It alternates which condition runs first and uses the same input-only profile, scheduler, RPI, duration, and CSV settings in both conditions. It creates a new directory and keeps all profiles, captures, adapter receipts, and the combined `comparison.json`.

~~~powershell
python tools/benchmark_integration.py .\target\release\eipmaster-cli.exe .\.local\feed-comparison-01 --pairs 3 --seconds 5 --rpi-us 1000
~~~

With feeds enabled, the client reads the latest HTTP and shared-memory samples, then waits 20 ms before repeating. A local MQTT broker receives the latest samples at a requested period of 25 ms. Each feed reports distinct samples, repeated reads, index gaps, and apparent ages calculated from the exported UTC anchor and elapsed time. Age includes consumer observation delay and uses the host wall clock. Latest-sample consumers can skip indexes by design.

The paired differences use enabled minus disabled. They retain scanner and simulator timing, negotiated intervals, packet sequence gaps, missed producer deadlines, CSV overruns, and export counters as separate measurements. A failed run stays in the report with its error and is excluded from paired deltas. The command exits 1 if any requested pair could not be compared. These measurements describe the localhost simulator. Use the separate bench process for physical qualification.

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
