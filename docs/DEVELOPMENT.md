# Develop and verify EthernetIPWorkbench

Start with [AGENTS.md](../AGENTS.md) and the [repository skill](../.agents/skills/ethernetip-workbench/SKILL.md). Use [Operations](OPERATIONS.md) to reproduce a reported setup before changing behavior.

## Source map

| File | Owns |
|---|---|
| src/model.rs | Connection settings, mapping validation, endian/swap decoding and output staging |
| src/eds.rs | Supported EDS connection profiles and provenance |
| src/preflight.rs | Effective configuration, EDS comparisons and offline validation |
| src/settings_file.rs | Named profiles, workspace migration, preferences and file replacement |
| src/scanner.rs | Identity, TCP session, Forward_Open/Close, cyclic workers and per-connection diagnostics |
| src/deadline_wait.rs | Windows deadline wait resource |
| src/capture.rs and src/capture/field_summary.rs | Bounded sample queue, CSV writer, rotation and summaries |
| src/integration.rs | Common export queue/history, sample indexes, lifecycle and counters |
| src/integration/http.rs | Loopback read API and bounded history batches |
| src/integration/mqtt.rs | Latest-sample publisher and broker reconnect |
| src/integration/shared_memory.rs | Windows mapping/mutex and binary snapshot |
| src/gui.rs and src/gui/run_report.rs | Device sessions, workflow controls and final capture reports |
| src/gui/workspace.rs | Tabs, persistent actions, review/profile windows and headless layout checks |
| src/bin/eipmaster-cli.rs | Headless commands and capture supervision |
| src/bin/eipmaster-cli/qualification.rs | Four-run qualification sequence |
| src/bin/eipmaster-cli/interrupt.rs | Windows console stop handling |
| tools/join_capture.py | CSV joining and report verification |
| tools/package_windows.py | Clean-commit Windows build, manifest and license bundle |

## Data and lifecycle contracts

The explicit TCP session reads identity before opening I/O. The cyclic producer schedules from the returned O-to-T interval. The receiver validates the packet source, connection ID, payload length and sequence. It timestamps near receive and copies accepted data to the independent capture and export paths.

The CSV queue is preallocated and bounded. Samples retain sequence, monotonic timing, index and generation. Normal stop drains the queue. Preserve existing files if rotation or report creation encounters a collision. A disk failure is scoped to its capture and must remain visible.

The integration receive path copies into a bounded queue and wakes its dispatcher. Serialization, network I/O and mapped-memory access occur in export workers. History retention, queue drops, MQTT publish errors and shared-memory contention have separate meanings. Preserve the versioned JSON and EIPWB001 binary contracts when updating consumers.

Connection::start_with_publisher accepts an optional integration Publisher. Existing start methods delegate with no publisher. Rust callers that own a Connection stop and join its worker explicitly. Integration owners keep the service alive across automatic reconnect and then shut down workers. The GUI and CLI each supervise their own sessions.

Keep automatic reconnect restricted to verified input-only profiles. The retry identity includes serial. Output staging and write authorization belong to the existing GUI/application controls. Preserve the requested/actual/observed interval distinction and the separate Forward_Close result.

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
| Reconnect, concurrent capture and writer failure | python tools/test_recovery.py target/release/eipmaster-cli.exe .local/recovery-check-01 |
| HTTP, MQTT and shared memory | python tools/test_integration.py target/release/eipmaster-cli.exe .local/integration-check-01 |
| Qualification supervisor | python tools/test_cli.py target/release/eipmaster-cli.exe .local/qualify-check-01 --qualification |
| Real console signals on supported Windows | python tools/test_interrupt.py target/release/eipmaster-cli.exe .local/interrupt-check-01 |
| Independent OpENer interoperability | python tools/test_opener.py target/release/eipmaster-cli.exe .local/opener-upstream .local/opener-check-01 |

The CLI check defaults to 8 ms. --rpi-us 1000 exercises the same cases at 1 ms. A timing failure must retain its receipt and unchanged watchdog configuration for diagnosis. Failed trials remain evidence alongside any later success.

The workspace tests render egui in memory at 900×580 and 1240×860 and simulate pointer events without a native window. They check workflow isolation, visible connection controls, write-review content and capture-stop behavior.

The ignored Rust GUI backend tests need the mode named in each test's ignore annotation. They exercise Device logic without opening a GUI. Start the loopback fixture using a process with CREATE_NO_WINDOW, wait for its Ready line, run the exact test filter with -- --ignored and clean up that fixture. tools/test_cli.py exposes with_adapter for this purpose. Use --bin assembly-scanner so unrelated tests are not selected.

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
