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

Set the NIC address in Device. In Explore, choose **Discovery UDP** for a broadcast or unicast destination. **Discover** sends one ListIdentity datagram and collects replies for up to 2.2 seconds. The report deduplicates responders and counts malformed or unrelated datagrams. Broadcast discovery depends on the NIC, local firewall and adapter.

Choose **Discovery TCP** for a known unicast endpoint. Switching from the default broadcast destination fills the field with the adapter address from Device. The TCP probe reads one ListIdentity reply before registering a session. Both transports report the responder address separately from its advertised endpoint. **Use adapter** copies the responder IP into Device. Verify the Identity Object before opening cyclic I/O. [Pinned OpENer ListIdentity implementation](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/enet_encap/encap.c)

~~~powershell
.\target\release\eipmaster-cli.exe discover 192.0.2.11
.\target\release\eipmaster-cli.exe discover 192.0.2.11 192.0.2.10 --tcp
~~~

Select a known attribute in the CIP browser or enter a documented class, instance and attribute. The browser uses Get_Attribute_Single or Get_Attributes_All and supports 8-, 16- and 32-bit IDs. Discovery and reads run independently of cyclic I/O and the CSV writer.

For a routed read, set the first bridge's IPv4 address in Device and enter its path in **CIP route (optional)**. A path is a sequence of up to eight port/link pairs. Enter a decimal port ID from 1 to 65535 and a link address as a byte (0–255) or a unicast IPv4 address. For example, `1/2/300/192.0.2.100/15/7` uses a backplane hop, an IPv4 link on extended port 300, and node 7 on extended port 15. Use the port IDs and link addresses documented for your topology. The bridge must support CIP Unconnected Send. For a direct read, leave the route blank. The CLI takes the same syntax as `--route <port/link/...>` after the attribute. A routed report uses `reply_from = "router"` for a bridge rejection and `reply_from = "target"` for the destination reply, including a rejected attribute.

Use `hex:` followed by 2–255 bytes as contiguous hex pairs for a vendor-documented multibyte link. For example, `4660/hex:0080FE` sends three link bytes in that order. Enter the address bytes alone. The scanner adds the link length and any required padding. Reports display hex links in uppercase and show a link as dotted IPv4 when its bytes are the ASCII spelling of a unicast IPv4 address. The same syntax applies to saved read routes and `io_route`. The encoded route is limited to 255 words. Class 1 and Class 3 also count the target segments in the connection path.

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

### Inspect advertised EtherNet/IP services

In **Explore**, select **List services** beside the adapter or bridge address from Device. Select **UDP** or **TCP** using the adjacent control. The query sends one unicast ListServices request to port 44818 from the selected local NIC. For TCP, it opens a short connection to the addressed endpoint before session registration. UDP is the default and the JSON report records the transport used.

~~~powershell
.\target\release\eipmaster-cli.exe list-services 192.0.2.10 192.0.2.11
.\target\release\eipmaster-cli.exe list-services 192.0.2.10 192.0.2.11 --tcp
~~~

The Communications item reports the protocol version, capability flags, CIP over TCP, and Class 0/1 cyclic I/O over UDP. The full flags and raw item bytes are kept, including unrecognized items. An encapsulation status is reported separately from a malformed reply. The advertised capabilities describe the endpoint. Confirm a particular cyclic profile with Forward_Open and capture on the intended path. [Pinned OpENer ListServices implementation](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/enet_encap/encap.c)

### Browse the CIP Object List

In **Explore**, enter the target's known **CIP route** if needed, then select **List objects**. It reads the Message Router's optional Object List at class `0x02`, instance `1`, attribute `1`. The list displays the class IDs in device order. Select a row to fill the class revision address, then use **Read class info** to inspect the class. [Keyence NU-EP1 Message Router Object](https://support.rockwellautomation.com/ci/okcsFattach/get/1091470_5)

~~~powershell
.\target\release\eipmaster-cli.exe list-objects 192.0.2.10 192.0.2.11
.\target\release\eipmaster-cli.exe list-objects 192.0.2.10 192.0.2.11 --route 1/3
~~~

The CLI returns `class_ids` as numeric IDs. Expand **Object List** in Explore for raw bytes, a decode error, or the target or router CIP status. A valid empty list returns `[]`. A rejected or unreadable list returns `null` and exit code 1.

### Browse CIP instances

In **Explore**, enter a class ID or choose one from **List objects**, then select **Find instances**. The query uses the selected CIP route and requests up to 64 entries starting after instance 0. **Next instances** requests one additional page using the last instance returned. Choose an instance to fill the read form for that class. [ODVA instance discovery example](https://www.odva.org/wp-content/uploads/2023/10/2023-ODVA-Conference_Majcher_Enabling-Data-Scientist-Use-Cases.pdf)

~~~powershell
.\target\release\eipmaster-cli.exe list-instances 192.0.2.10 192.0.2.11 4
.\target\release\eipmaster-cli.exe list-instances 192.0.2.10 192.0.2.11 4 300 --route 1/3
~~~

The optional cursor is a decimal or hexadecimal UINT. Use the JSON `next_after` value as the cursor for another page. A valid terminal reply has `next_after: null`. The report retains the raw reply, a decode error, or the router or target CIP status. The command returns exit code 1 for an unsupported service, a malformed list, or an exchange error. Each request uses one registered explicit session for one page of 16-bit IDs. [ODVA CIP Object Tests, Find_Next_Object_Instance](https://ticket.hilscher.com/secure/attachment/260555/CIP_PCTS.pdf)

### Inspect a CIP class

In **Explore**, enter a class ID or select a common attribute to fill in its class. Set **CIP route** when reading through a bridge, then select **Read class info**. The inspection queries class instance zero with seven Get_Attribute_Single requests on one TCP session. The results show the revision, maximum instance ID, instance count, optional instance attributes and services, and maximum attribute IDs. Maximum IDs describe bounds. Use the device manual for the implemented instance and attribute addresses. [Common class attribute formats](https://assets.danfoss.com/documents/366937/BC361183001812en-000201.pdf) · [EtherNet/IP class attribute example](https://product-help.schneider-electric.com/Machine%20Expert/V1.1/en/tm3bcprg/tm3bcprg/TM3_Ethernet_Bus_Coupler/TM3_Ethernet_Bus_Coupler-6.htm)

~~~powershell
.\target\release\eipmaster-cli.exe class-info 127.0.0.1 127.0.0.1 0x01
.\target\release\eipmaster-cli.exe class-info 192.0.2.10 192.0.2.11 0x04 --route 1/3
~~~

The routed command takes the first bridge's address. The JSON keeps each reply's raw bytes, decoded value or decode error, and CIP status. The inspection returns exit code 0 when at least one field decodes and the only rejections are target statuses `0x08` or `0x14` for unsupported services or attributes. A router rejection, exchange failure, undecodable reply, or lack of a decoded value returns exit code 1.

### Browse CIP attributes

In **Explore**, choose **Browse attributes** after setting the class and instance. The picker names common attributes for supported object families. For a nonzero instance, **Read class info** also loads optional attribute IDs advertised for that class, endpoint, NIC, and route. Overlapping choices show the common name with an **Advertised** marker.

Selecting a choice fills the attribute and request name. Use **Read** to query the chosen value or **Save / update read** to keep the request. The device reply determines support for the selected instance.

The CLI `class-info` JSON exposes the class revision and other replies alongside numeric `optional_instance_attributes` and `optional_services` arrays. An empty array records a valid list with no entries. A `null` value records an unavailable or malformed list. The individual result shows the raw bytes, error or CIP status. [Danfoss common class attribute example](https://assets.danfoss.com/documents/366937/BC361183001812en-000201.pdf)

### Read a CIP attribute list

In **Explore**, set the class, instance, and optional route. Open **Attribute list read** and enter one space-separated `id:reply-bytes` pair per attribute. For Identity class 1, instance 1, use `1:2 4:2 6:4` for vendor ID, revision, and serial number, then select **Read attribute list**.

```powershell
.\target\release\eipmaster-cli.exe read-attribute-list 127.0.0.1 127.0.0.1 1 1 1:2 4:2 6:4
.\target\release\eipmaster-cli.exe read-attribute-list 192.0.2.10 192.0.2.11 1 1 1:2 4:2 6:4 --route 1/3
```

The target must support Get_Attribute_List on the selected object. Use documented fixed reply lengths. The request carries the attribute IDs, and the client uses the supplied widths to separate the returned values. Each entry reports its own status and raw value when available. The full reply body is available for diagnosing malformed lists or incorrect widths. [Rockwell CIP attribute list examples](https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm020_-en-p.pdf) · [Pinned OpENer Get_Attribute_List implementation](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipcommon.c#L1114)

The CLI prints JSON and exits 0 when all requested attributes succeed and the list decodes. A mixed reply returns exit code 1 and includes the successful values alongside each rejection. Review `general_status`, `extended_status`, and `reply_from` for a whole-service failure, or `decode_error` for a malformed list.

### Inspect a CIP port

In **Explore → Port Object**, choose **Directory** or **Port instance**. Enter the instance you want and select **Read port info**. Both the GUI and CLI use one registered session with bounded Get_Attribute_Single requests. Leave **CIP route** blank to inspect the directly addressed device, or enter an existing route to inspect the device at its end.

~~~powershell
.\target\release\eipmaster-cli.exe port-info 127.0.0.1 127.0.0.1 0
.\target\release\eipmaster-cli.exe port-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

Instance 0 reads the maximum instance ID, instance count, entry-port instance and ordered port type and number pairs. A nonzero instance reads its type, number, Link Object path, name, node address, and routing capability bits where supported. **Entry port instance** identifies the Port Object instance. Use the **Port number** reported by that instance when interpreting the device's CIP topology. The node address describes this device's own address on the port. [Toshiba Port Object attributes](https://toshiba.com/tic/wp-content/uploads/AS3_Embedded_Ethernet_Function_Manual.pdf) · [Schneider Electric Port Object formats and routing bits](https://productinfo.se.com/pm5500/595e2aa946e0fb0001f715da/PM5500%20user%20manual/English/PM5500SeriesUserManualv02_0000044889.ditamap/$/R_Communications_EthernetIP_MessagingExplicitExchanges_PM5500_0000178841)

The report preserves raw replies and shows individual decode errors and CIP status. As with `class-info`, successful fields can accompany unsupported target attributes. Review the device's documented topology when choosing a new route.

### Inspect a TCP/IP interface

In **Explore → Network objects**, choose **TCP/IP Interface · Status**. Enter the interface instance if it differs from the preset's instance 1, then select **Read interface info**. Leave **CIP route** blank for a direct read or enter a known route to read the interface reported by the routed target.

~~~powershell
.\target\release\eipmaster-cli.exe tcpip-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe tcpip-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

The inspection sends six Get_Attribute_Single requests in one registered session. The report shows interface status and configuration capability masks, the configured startup method, the Physical Link Object path, IPv4 address, mask, gateway, primary and secondary DNS servers, domain name, and hostname. Use the reported startup method alongside the live address fields. Each attribute retains its raw reply and any decode error or CIP rejection. [Schneider Electric TCP/IP Interface attributes](https://productinfo.se.com/tesys_tera_master/tesys-tera-motor-management-system-ethernet-communication-guide/EN/DOCA0258_Tera_Ethernet_guide_DD00602307.XML.xml/$/Tera_TCP_IPObject_Ethernet_DD00809230.XML)

#### Inspect multicast and session settings

Enable **Include multicast and session settings** before selecting **Read interface info**. The same registered session adds Get_Attribute_Single reads for attributes 8, 9, and 13. The CLI supplies the decoded values under `network_diagnostics` and preserves the raw bytes, CIP status, or decode error under each result. A missing or malformed attribute has a `null` decoded value. [Rockwell TCP/IP Interface attributes](https://literature.rockwellautomation.com/idc/groups/literature/documents/um/193-um014_-en-p.pdf#page=106) · [Belden TCP/IP Interface attributes](https://www.belden.com/-/media/project/belden/catalog-assets/assets/2026/02/07/20/30/pdf1001143.pdf?rev=59f39d8442364a889d1f05610991e474)

~~~powershell
.\target\release\eipmaster-cli.exe tcpip-info 127.0.0.2 127.0.0.1 1 --diagnostics
.\target\release\eipmaster-cli.exe tcpip-info 192.0.2.10 192.0.2.11 1 --diagnostics --route 1/3
~~~

The multicast pending flag comes from interface status bit 4. The allocation reply includes the control code, number of addresses, and starting address. For a cyclic capture, use the group returned by Forward_Open and recorded under `cyclic_io`. The reported encapsulation inactivity timeout applies to idle TCP sessions. Zero disables that timer. Review the separate receive watchdog in the cyclic capture report for Class 1 I/O timeouts. [Belden TCP inactivity timeout](https://www.belden.com/-/media/project/belden/catalog-assets/assets/2026/02/07/20/30/pdf1001143.pdf?rev=59f39d8442364a889d1f05610991e474) · [Pinned OpENer multicast encoding](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/ciptcpipinterface.c#L336)

#### Inspect IPv4 address conflicts

In **Explore → Network objects**, choose **TCP/IP Interface · Status**, select the interface instance, and click **Read address conflicts**. Enter a documented CIP route to inspect a target behind a bridge.

~~~powershell
.\target\release\eipmaster-cli.exe acd-info 127.0.0.2 127.0.0.1 1
.\target\release\eipmaster-cli.exe acd-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

On one registered session, the inspection reads the interface status and capability flags, the ACD selection, and the last conflict record. The JSON `address_conflict_detection` fields show support, enabled state, conflict and fault flags, and the last recorded event. Expand **Last address conflict · Reply** in Explore to see the recorded phase, remote MAC, and sender and target IPv4 addresses when the saved PDU is Ethernet/IPv4 ARP. [Rockwell TCP/IP Interface attributes](https://support.rockwellautomation.com/cc/okcsFattachCustom/get/1041842_5) · [Hilscher ACD and ARP field definitions](https://www.hilscher.com/fileadmin/cms_upload/de/Resources/pdf/EtherNetIP_Adapter_Protocol_API_20_EN.pdf#page=62)

Unavailable values appear as `null` in the summary. Activity code 0 means the device reports no retained conflict since the record was last cleared. The individual result preserves CIP rejections, malformed payloads, and raw bytes. A historical conflict can help explain an interruption. Use the interface flags and a bench packet trace to establish when a conflict occurred.

### Inspect Ethernet Link diagnostics

In **Explore → Network objects**, choose **Ethernet Link · Speed**, enter the Ethernet Link Object instance, and select **Read link info**. Use the device documentation or the Physical Link Object path from TCP/IP Interface inspection to identify the instance. The path `20 F6 24 01` points to Ethernet Link instance 1. Leave **CIP route** blank for a direct read or enter the documented route to the target.

~~~powershell
.\target\release\eipmaster-cli.exe link-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe link-info 192.0.2.10 192.0.2.11 2 --route 1/3
~~~

The inspection reads five attributes on one registered session using Get_Attribute_Single. It reports speed, the full flags word with link state, active duplex and negotiation status code, and the MAC address. Supported ports also show cumulative 32-bit interface and media counters, including discards, FCS errors, and collisions. Their values can wrap. Each result retains the raw reply or CIP rejection. Inspecting these attributes leaves the device counters intact. [Schneider Electric EtherNet/IP device objects](https://product-help.schneider-electric.com/Machine%20Expert/V1.2/en/lmcprg/lmcprg/Ethernet_Configuration/Ethernet_Configuration-17.htm) · [Pinned OpENer Ethernet Link counter layout](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipethernetlink.h)

#### Inspect Ethernet port settings

Enable **Include port settings and capabilities** before selecting **Read link info**. The registered session also reads Ethernet Link attributes 6–11. The CLI puts the decoded values under `port_details` and preserves the raw results and CIP statuses.

~~~powershell
.\target\release\eipmaster-cli.exe link-info 192.0.2.10 192.0.2.11 1 --port-details
.\target\release\eipmaster-cli.exe link-info 192.0.2.10 192.0.2.11 2 --port-details --route 1/3
~~~

Compare the live speed and duplex with the configured automatic or forced mode. Forced speed and duplex apply when auto-negotiation is disabled. The report keeps the operational and administrative states separate and accepts the documented one-byte and two-byte label lengths. Capabilities describe the interface's supported features and advertised speed/duplex pairs. Unavailable values are `null`, with the underlying rejection or decode error retained in the per-attribute result. [Hilscher Ethernet Link attribute definitions](https://www.hilscher.com/fileadmin/cms_upload/de/Resources/pdf/EtherNetIP_Adapter_Protocol_API_20_EN.pdf#page=68) · [Hirschmann Ethernet Link attribute definitions](https://www.doc.hirschmann.com/pdf/Manual-IO_LioN-X_EtherNetIP_IO-Link-Master_DE_3V1_2025-09-09.pdf#page=139) · [Pinned OpENer capabilities encoder](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipethernetlink.c#L477)

### Inspect LLDP neighbors

In **Explore → Network objects**, choose **LLDP neighbors**. Select **Find instances**, choose a reported instance, and select **Read LLDP neighbor**. You can also enter a known instance directly when the device doesn't support listing. Enter the documented route when reading a device through a CIP bridge.

~~~powershell
.\target\release\eipmaster-cli.exe list-instances 192.0.2.10 192.0.2.11 0x10A
.\target\release\eipmaster-cli.exe lldp-neighbor 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe lldp-neighbor 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

The report reads the target's recorded neighbor entry using nine Get_Attribute_Single requests on one explicit session. Use the returned local Ethernet Link instance to inspect the corresponding port with `link-info`. The neighbor's advertised validity interval, capabilities, and CIP identification come from the target's LLDP table. Treat them as recorded information from the target. Last Change is shown as the raw counter because vendor manuals differ on its units. [Hirschmann LLDP Data Table reference](https://www.doc.hirschmann.com/pdf/Manual-IO_LioN-X_EtherNetIP_IO-Link-Master_EN_3V1_2025-09-09.pdf#page=139) · [Siemens LLDP Data Table reference](https://docs.tia.siemens.cloud/r/simatic_et_200eco_pn_manual_collection_enus_20/function-manuals/communication-function-manuals/multifieldbus/ethernet/ip/supported-cip-objects-for-ethernet/ip/lldp-data-table-object)

The CLI emits JSON with the decoded `neighbor` and each raw reply, CIP rejection, or decode error. An empty `management_ipv4` array means the target reported zero addresses. A `null` there means the attribute was unavailable or could not be decoded. The command exits with code 1 when it receives no usable reply or encounters a malformed or failed read.

### Inspect Device Level Ring status

In **Explore → Network objects**, choose **Device Level Ring · Status**, select the documented instance, and click **Read ring status**. The preset starts at instance 1. Enter the documented CIP route for a target reached through a bridge.

~~~powershell
.\target\release\eipmaster-cli.exe dlr-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe dlr-info 192.0.2.10 192.0.2.11 2 --supervisor-details --route 1/3
~~~

The inspection reads network topology and status, the active supervisor address, and capability flags from class `0x47`. Select **Include supervisor details** for the reported supervisor state, fault count, last active node on each supervisor port, participant count, and active supervisor precedence. Compare the reported fault endpoints with the ring diagram and the active supervisor's diagnostics. The addresses are observations reported by the target. [Rockwell Automation DLR diagnostics guide](https://literature.rockwellautomation.com/idc/groups/literature/documents/at/enet-at007_-en-p.pdf#page=90)

The CLI reports structured values under `dlr`. Review the per-attribute result for any unreported, unsupported, or malformed field. A zero count is a reported value. A successful address reply with zero IP and MAC fields shows **No address reported** and retains the original bytes. The inspection uses Get_Attribute_Single on one registered session and leaves the ring settings and counters intact.

### Read the Device Level Ring participant list

In **Explore → Network objects → Device Level Ring · Status**, point Device at a supporting active supervisor, select the documented DLR instance, and choose **Read ring participants**. For a supervisor reached through a CIP bridge, use the first bridge address and its documented route.

~~~powershell
.\target\release\eipmaster-cli.exe dlr-participants 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe dlr-participants 192.0.2.10 192.0.2.11 2 --route 1/3
~~~

The command reads the count, list, and count again on one registered TCP session using Get_Attribute_Single. Addresses stay in the order returned by the supervisor, including participants reported without an IPv4 address. The single list reply is subject to the target's reply-size limit. [Rockwell Automation participant list reference](https://literature.rockwellautomation.com/idc/groups/literature/documents/at/290e-at001_-en-p.pdf#page=51)

CLI exit code 0 means both reported counts agree with the decoded list length. A count change, length mismatch, unavailable field, or malformed reply returns code 1 with the report and any decoded entries. A successful zero-entry reply yields an empty list. Matching counts check the length across the three requests. Membership can change while the size stays the same. Expand **Participant replies** to review the raw replies, including any CIP rejection from the supervisor or route.

### Inspect CIP Sync clock status

In **Explore → Network objects**, choose **CIP Sync · Clock status** and select **Read clock status**. The preset uses Time Sync Object class `0x43`, instance 1. Enter a documented CIP route to inspect the device at the end of that route.

~~~powershell
.\target\release\eipmaster-cli.exe time-sync-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe time-sync-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

Compare the reported synchronization state, PTP domain, and grandmaster identity across the devices of interest. The report includes the device's offset and mean path delay in nanoseconds, maximum offset, PTP port states, and steps removed. The grandmaster fields retain the original UTC offset word and time-property flags for review. The inspection reads each attribute once on one registered session, so the values may reflect different instants. The capture timestamps continue to use the scanner's receive clock. [Rockwell Automation grandmaster verification](https://www.rockwellautomation.com/en-id/docs/technical/i-o/current/5034-pointmax/_online/5034-um004-ditamap/io-link-master-module/cip-sync-time.html) · [Micro-Epsilon Time Sync attributes](https://www.micro-epsilon.com.cn/fileadmin/download/manuals/man--optoNCDT-1900-IE-EtherNet-IP--en.pdf#page=82)

The CLI report puts decoded fields under `time_sync` and retains every raw reply, CIP rejection, and decode error. A reported `false` synchronization flag or an empty port list is a decoded value. A missing or malformed field appears as `null` in the summary. The inspection leaves device clock settings intact.

### Inspect Quality of Service settings

In **Explore → Network objects**, choose **Quality of Service · Explicit DSCP** and select **Read QoS settings**. Use instance 1 unless the device documents another nonzero instance. An optional CIP route addresses the device at the end of a known path.

~~~powershell
.\target\release\eipmaster-cli.exe qos-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe qos-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

One registered session reads the 802.1Q tag enable setting and the configured DSCP values for PTP event and general messages, the four cyclic priorities, and explicit traffic with Get_Attribute_Single. DSCP values are reported as code points from 0 to 63. Each attribute retains its raw reply or CIP rejection. Compare a packet trace with the report when verifying what the device or switch puts on the wire. [Schneider Electric QoS Object reference](https://productinfo.se.com/tesys_tera_ecg/viewer?docidentity=QoSObject-9CA56F93&extension=xml&lang=en&manualidentity=EthernetCommunicationGuideTeSysActi-82F1CBFF)

### Inspect Connection Manager counters

In **Explore**, choose **Connection Manager** and select **Read connection counters**. The preset selects instance 1. Enter another nonzero instance if documented by the device. Leave **CIP route** blank to inspect the directly addressed device, or enter a known route to inspect the target at its end.

~~~powershell
.\target\release\eipmaster-cli.exe manager-info 192.0.2.10 192.0.2.11 1
.\target\release\eipmaster-cli.exe manager-info 192.0.2.10 192.0.2.11 1 --route 1/3
~~~

One registered explicit session reads attributes 1–8 with Get_Attribute_Single. The counters are unsigned 16-bit totals from the selected Connection Manager. They cover requests from other originators and can wrap. Review them alongside the scanner's connection diagnostics. Reads leave the counters intact. [Siemens Connection Manager attribute definitions](https://cache.industry.siemens.com/dl/files/201/109758201/att_988551/v1/manual_Ethernet_IP_communication_module_3RW5_en-US.pdf?download=true)

Each result retains its raw reply, decoding error, or CIP status. The CLI accepts a usable partial report when at least one counter decodes and the target marks the others unsupported. If every counter is unsupported, the CLI retains all rejections in JSON and returns exit code 1.

### Inspect one active connection

In **Explore**, choose **Connection Manager** and open **Connection data**. Use the target's connection number for Get_Connection_Data (`0x56`). To use Search_Connection_Data (`0x57`), enter the Forward_Open triad as `connection-serial:originator-vendor:originator-serial`. Use decimal or `0x` hexadecimal for each identifier. A supporting device assigns its own connection number. Get it from the target's diagnostics or from a successful triad lookup. The query uses Connection Manager instance 1 and the optional **CIP route**. [IEC Type 2 service definitions](https://webstore.iec.ch/en/publication/4695) · [Pinned OpENer Connection Manager implementation](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipconnectionmanager.c#L762)

~~~powershell
.\target\release\eipmaster-cli.exe connection-data 192.0.2.10 192.0.2.11 4660
.\target\release\eipmaster-cli.exe connection-data 192.0.2.10 192.0.2.11 0xABCD:1:0x12345678 --route 1/3
~~~

Each request reads one record over a registered explicit session. The result includes the state code, the recorded triad, and the reported port, connection IDs, interval fields, and timeout multiplier codes. The RPI and API fields are decoded in the protocol's microsecond units. For timing analysis, use the Forward_Open values and observed packet spacing from the capture report. [IEC Type 2 protocol specification](https://webstore.iec.ch/en/publication/4695)

The CLI exits 0 for a decoded record matching the requested number or triad. It exits 1 for a CIP rejection, transport failure, mismatched record, or malformed reply. The JSON retains raw bytes for decode errors and identifies target and bridge rejections separately.

For a running Workbench scan, open **Diagnostics → Adapter connection record**. **Read active connection record** uses the verified triad, NIC, and route from the running session over a separate explicit TCP connection. **Copy lookup selector** copies the triad in CLI format. After reconnect, the panel shows the new generation's selector and waits for a new read before showing a record.

The GUI run report, CLI capture report, and CLI serve attempts retain `forward_open_triad` for each accepted Forward_Open. Its fields are `connection_serial`, `originator_vendor`, and `originator_serial`. Use those values to match a packet trace or pass a selector to `connection-data` while the applicable connection is active.

### Inspect an assembly

In **Explore → Common attributes**, choose **Assembly · Data** to start from the profile's input assembly instance. Change the instance to inspect another documented assembly, then select **Read assembly info**. Leave **CIP route** blank for a direct read or enter a known route.

~~~powershell
.\target\release\eipmaster-cli.exe assembly-info 192.0.2.10 192.0.2.11 101
.\target\release\eipmaster-cli.exe assembly-info 192.0.2.10 192.0.2.11 101 --route 1/3
~~~

One registered session reads the declared byte count from attribute 4 and the raw data from attribute 3 with separate Get_Attribute_Single requests. The CLI exposes `declared_size_bytes`, `data_size_bytes`, and `sizes_match`. A missing or malformed reply leaves its size unavailable. An empty data reply counts as zero bytes. A mismatch is visible in both interfaces even when both reads succeeded. Compare the reported values with the documented cyclic payload, including any device-specific size or header rules, before choosing Forward_Open settings. [Pinned OpENer Assembly Object reference](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipassembly.c)

### Check profile assembly sizes

In **Explore**, select **Check profile assembly sizes**. The check uses the adapter, local NIC, cyclic I/O route, and effective assembly settings from Device. The CLI loads the same settings from a saved profile.

~~~powershell
.\target\release\eipmaster-cli.exe check-assemblies .\samples\opener-loopback.toml
~~~

One registered explicit session reads the declared size from Assembly attribute 4 once per distinct instance. The report compares each input and output role against its configured fixed size or variable-size range. An input-only heartbeat expects zero output bytes. A configuration comparison is included when the profile carries configuration bytes. The comparisons use application payload lengths before connected I/O headers are added. This is a read-only check of the target-reported sizes and is separate from cyclic acceptance. [Pinned OpENer Assembly size attribute](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipassembly.c)

Expand **Assembly size replies** for raw data and CIP errors. The CLI returns exit code 0 when every reported size is within its profile range. A mismatch or unavailable size returns exit code 1 and retains each comparison and reply in JSON.

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
| default_unicast_port | Bind UDP 2222 and omit the unicast T→O socket-address item in Forward_Open |
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

### Use default unicast UDP 2222

For a direct or routed unicast profile that expects the default EtherNet/IP I/O port, select **Use default unicast UDP 2222** in Device or set `default_unicast_port = true` with `local_udp_port = 0` or `2222` in TOML. The scanner binds UDP 2222 on the selected NIC and omits the T→O Sockaddr Info item from Forward_Open. Forward_Open can still return a separate O→T destination port. When the reply omits that item, the scanner sends to UDP 2222 on the adapter or first bridge. [OpENer point-to-point UDP port handling](https://github.com/EIPStackGroup/OpENer/blob/0e1ab7ee5c272950c0dc8daf7793008d7a518912/source/src/cip/cipioconnection.c)

Offline preflight reports the effective local port. Capture reports record the mode and the actual UDP endpoints. If another application owns the port on that NIC, the bind fails before Forward_Open. Configure multicast using the usual multicast profile controls.

### Configure routed Class 1

Use **Device → CIP routing** to set the documented port/link path from the first bridge to the target. In TOML, set `device_ip` to the first bridge's IPv4 address and `io_route = "1/3"` for a target at backplane slot 3. Leave the path blank for a direct connection. Use the target's assemblies, RPIs, transfer format and electronic key. The EDS supplies assembly settings. Enter the route for the installed topology manually.

For routed input-only, use zero output and configuration bytes and disable O→T Run/Idle. Select unicast or multicast T→O according to the documented bridge and device profile. The first bridge receives the scanner's unicast heartbeat and sources the inputs on the selected NIC's segment. For multicast, verify the returned group and receive port in the run report. For routed Exclusive Owner, select the documented T→O distribution. O→T outputs use unicast to the first bridge. Use **Read target identity** in connection review, verify the target serial, then authorize the displayed output and configuration bytes. Forward_Open can apply configuration before the scanner joins the multicast group. Cyclic output starts after the scanner validates its UDP endpoints. Automatic reconnect is available for verified controlling input-only profiles.

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

Forward_Open returns the T→O group, UDP port and connection ID. The scanner joins that group on the configured NIC and checks the expected source IP and returned ID. For routed Class 1, the expected source is the first bridge. Use **Diagnostics** or the capture report's `cyclic_io` table to see the multicast destination and local receive endpoint. The requested local UDP port still binds the O→T socket. For multicast it also supplies the port hint, which defaults to 2222 when the field is zero. The returned receive port takes precedence.

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

Each generation reports the Identity Object, connection state, accepted packet and anomaly counters, and the source, destination and connection IDs under `cyclic_io`. The `forward_open_triad` field preserves the three identifiers from the accepted Forward_Open for the corresponding generation. Read `requested_ot_us` and `requested_to_us` for the profile intervals, `actual_ot_us` and `actual_to_us` for the Forward_Open reply, and `send_spacing` and `receive_spacing` for measured socket timing. `input_age_us` uses the monotonic receive clock. During cyclic I/O, receive counters and spacing reflect the latest published scanner snapshot. `connection_timeout`, `receive_timeout`, and `forward_close` detail the configured watchdog, an observed timeout and the actual close result. The three error fields carry identity, CIP connection and transport messages separately. Measurements or results that are unavailable appear as null.

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
