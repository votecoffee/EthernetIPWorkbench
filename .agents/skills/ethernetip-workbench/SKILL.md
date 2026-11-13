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

## Choose the entry point

- Run CLI eds, validate or decode-input for offline work. validate returns connection validity and capture validity separately. Inspect both.
- Use CLI identity for an authorized bounded hardware identity read. capture requires an input-only profile and an expected decimal identity. Include serial to pin a unit. qualify automates the baseline and three target runs.
- Use GUI profiles for manual operation and authorized output staging. Start a headless fixture for testing rather than opening the desktop application during unattended work.
- Attach software consumers to the existing connection. HTTP offers bounded history. MQTT and shared memory offer latest samples. They do not provide remote output control. C# can consume those interfaces or supervise the CLI. A native C ABI is still a proposed extension.

## Preserve meaning and authorization

Use zero-based byte offsets for mappings. Apply the profile's actual endian and swap settings through model::Selection. Use decode-input with distinct synthetic field values to detect overlapping or incorrect offsets. Engineering-unit scaling belongs to the device's documented interpretation, which must be checked separately from raw integer decoding.

For live work, use the session's authorization and an isolated bench. Confirm the exact ownership and output/configuration actions when they are not already authorized. Configuration travels in Forward_Open and an owner may start transmitting staged output immediately.

Separate requested RPI, Forward_Open actual intervals, observed socket spacing and disk/export loss. Preserve captures and failure receipts. Do not infer that skipped producer slots are received sequence gaps. Keep .local/ private, and keep desktop interaction subject to an explicit request in the current chat.

## Finish with a reproducible result

Give the exact profile, executable or commit and capture/report paths relevant to the task. State whether results came from offline decoding, a software adapter or a physical device. For code or documentation changes, run the relevant checks and maintain the command and schema documentation. The skill does not grant hardware, output or publishing permission beyond the user's current authorization.
