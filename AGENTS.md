# Working on EthernetIPWorkbench

Read the repository skill at [.agents/skills/ethernetip-workbench/SKILL.md](.agents/skills/ethernetip-workbench/SKILL.md) when configuring, operating, integrating or changing this application. Use [docs/OPERATIONS.md](docs/OPERATIONS.md) for command recipes and [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for source navigation and verification.

## Scope and names

- The repository is EthernetIPWorkbench. The Rust package and GUI executable are still named assembly-scanner, the CLI is eipmaster-cli, and the saved GUI workspace lives in the EIPMaster application-data directory. Follow those existing names in commands and compatibility work.
- This code operates as a Class 1 EtherNet/IP scanner/originator. HTTP, MQTT and shared memory export its inputs. A C-compatible FFI and an EtherNet/IP adapter stack are separate future work.
- Verify checkout state, applicable instructions and remote before editing. Keep unrelated changes intact. Use the verified origin when publishing authorized work.

## Operational boundaries

- Prefer offline validation, payload decoding and localhost fixtures for implementation. A request to modify code is not an instruction to connect to a physical device.
- Use the current session's hardware authorization. Before sending live output or configuration bytes, verify that authorization covers the device, ownership and exact proposed writes. Input-only heartbeat settings require zero output and configuration bytes and O-to-T Run/Idle disabled.
- Keep tests headless. Desktop input, desktop/window screenshots and screen capture require an explicit request in the current chat.
- Store vendor files, physical device settings, captures and private plotting tools under ignored .local/. Check the ignore rule before creating local artifacts. Public documentation and sample profiles use generic devices and loopback addresses.

## Implementation contracts

- Keep cyclic scheduling, receive timestamps, CSV writing and integration transports on their existing independent execution paths. Slow consumers must have bounded impact and visible loss counters.
- Preserve the difference between requested RPI, Forward_Open actual packet intervals and observed socket spacing. A clean localhost test does not establish physical 1 ms acceptance.
- Preserve old profiles, CSV columns and sample indexes across reconnects. Distinguish packet sequence gaps, duplicates, late/invalid traffic, CSV overruns, export queue drops and intentional on-change filtering.
- Keep output staging separate from applying an output buffer. Keep automatic reconnect restricted to identity-pinned input-only sessions.
- Update the relevant operator documentation, skill routing and verification notes when changing a public command, profile key, export schema or lifecycle.

## Delivery

Run checks proportionate to the change using [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). Use fresh ignored directories for fixtures and captures. Record physical-device and simulator evidence with the conditions under which each was obtained. Build portable packages from a clean commit matching origin/main using tools/package_windows.py.
