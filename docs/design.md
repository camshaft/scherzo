# WASM-first 3D printer firmware design

## Purpose and scope

This document proposes a 3D printer firmware architecture that treats all high-level logic as WebAssembly (WASM) components running on a small, stable Rust host. The intent is to replace traditional G-code streaming with explicitly linked components that declare their host API usage up front, enabling safer mid-print behavior, dynamic composition, and reuse of existing ecosystems (e.g., Klipper modules) via WASM.

## Core goals

- **Stable Rust core**: A minimal host that manages MCU comms, time/scheduling, motion planning interfaces, IO, safety interlocks, and host API surfacing to components.
- **WASM-first extensibility**: All printer logic, including “firmware features” and print programs, is packaged as WASM components that import only the host APIs they need.
- **Command uploads without raw G-code**: Prints are submitted as WASM components that call host commands directly, avoiding unknown/late-bound G-code mid-print.
- **Deterministic and safe execution**: Clear sandboxing, bounded memory/CPU budgets, deterministic scheduling, and auditable imports per component.
- **Composable handler model**: Components can register as handlers for specific (virtual) G-code verbs or higher-level events, enabling plug-and-play behavior without recompiling the core.
- **Interop with Klipper modules**: Ability to load (or transpile) Klipper Python modules into WASM components to leverage the ecosystem while migrating critical pieces to Rust/WASM over time.

## High-level shape

- **Host runtime (Rust)**: Owns hardware drivers, motion planning, deterministic scheduler, timebase, safety watchdogs, and the host API surface exposed to WASM imports. Provides a component registry and dispatch table for verb/event handlers.
- **WASM components**: Self-describing modules that declare required imports and exported handlers. Deployed for both print jobs and firmware features (e.g., bed leveling, thermal control policies, UI/reporting hooks).
- **Submission & linking**: Instead of streaming G-code, uploads provide a component (or set) plus a manifest of handler registrations. Host validates imports, links to the stable API surface, and schedules execution.
- **Lifecycle**: Components can be installed/uninstalled, activated per-print, and hot-reloaded when safe (between motion blocks / with quiesced hardware), with explicit state migration hooks.

## Configuration extension model

- **Host-owned config plane**: The host owns the canonical configuration store and validation pipeline. Components cannot mutate config arbitrarily; they propose schemas and receive validated config at init.
- **Component-provided schemas**: Each component may register one or more configuration schemas (e.g., JSON Schema or similar). Schemas describe required/optional fields, types, bounds, and defaults, and are versioned alongside the component.
- **Validation & injection**: The host validates user-supplied config against the declared schema before activation. On successful validation, the host injects the resolved, normalized config into the component’s initialization entrypoint.
- **Namespacing and capability scoping**: Schemas are namespaced per component to avoid collisions. Sensitive fields (e.g., pins, heaters) can be gated by capabilities/ACLs so a component can’t claim hardware it isn’t allowed to drive.
- **Hot-reload & migration**: When updating a component, the host re-validates config against the new schema and supports migration hooks to transform persisted state/config where needed.
- **UI/UX reuse**: Because schemas are explicit, UIs (web/desktop) can render forms automatically and provide guardrails; the same schemas back CLI validation and CI linting of printer configs.

## Host API surface and versioning

- **Interface definition**: Host APIs are defined in a WIT that is published versioned artifacts (e.g., wit-bindgen packages) for Rust/TS/Python targets. Components link against a specific major.minor API version and declare this in their manifest.
- **Version negotiation**: During component load, the host checks declared API version and enabled feature flags. If compatible, the host links; if not, the load is rejected with a diagnostic. The host may expose multiple minor versions concurrently to ease migration.
- **Capability-scoped handles**: Resources (pins, heaters, steppers, sensors) are exposed as opaque handles bound to capabilities granted at install time. APIs operate on handles, preventing use beyond declared scope.
- **Determinism contract**: APIs that can perturb timing (e.g., logging, host communication) are clearly marked and may require yielding to a lower-priority queue. Real-time-safe APIs are separated and enforce bounded execution.
- **Error model**: APIs return structured errors (namespaced codes, retryability hints). Components must handle errors; unhandled fatal errors trigger component unload or trip safety interlocks depending on severity.
- **Deprecation and shims**: Deprecated APIs remain for at least one minor series with warnings. The host can optionally provide shims for selected breakages to ease migration, but long-term removal follows the major-version policy.
- **Host-provided utilities**: Common services (e.g., CRC, encoding, small allocators, monotonic clock) are provided to reduce duplicate code in components and to keep deterministic characteristics known to the host.

## Component model

- **Manifest**: Each component ships with a manifest declaring:
  - Component ID, version, and required host API version/feature flags
  - Declared capabilities (resources it wants to bind: pins, heaters, steppers, sensors, storage, network)
  - Handler registrations (verbs/events), priority hints, and scheduling class
  - Config schema references (namespaced) and any required secrets/credentials gates
  - Binary metadata (hashes, signatures) for cacheability
- **Handler registration**: Components export handlers for events such as:
  - Virtual G-code verbs / high-level commands (e.g., `M104`-equivalent, `SET_FILAMENT_PROFILE`)
  - Lifecycle events (install, init, activate/deactivate, pre-unload)
  - Periodic ticks or watchdog-ping callbacks within bounded budgets
  - Telemetry/reporting hooks and diagnostics triggers
    The host builds a dispatch table from these registrations; conflicts are resolved via policy (e.g., explicit routing, highest-priority match, or composition chains).
- **Capabilities & binding**: At install time, the host evaluates the manifest’s capability requests against policy/ACLs and available hardware. Granted resources become opaque handles delivered at init. Unmet requests fail installation or are flagged for user approval.
- **Initialization contract**: Init receives validated config, granted handles, and an initialized host-API table matching the declared version. Init must be bounded-time; failures abort activation and roll back handler registration.
- **State & persistence**: Components may request durable key-value namespaces. Access is namespaced and quota-enforced. Migration hooks may run during upgrades to reshape state.
- **Observability**: Components get structured logging, metrics emitters, and tracing spans via the host API. Rate limits may apply to avoid perturbing real-time paths.

## Scheduling and determinism

- **Dual-path execution**: Split between a real-time (RT) path for motion/thermal-critical work and a best-effort (BE) path for non-critical tasks (UI, telemetry bursts, background prep). Components declare which handlers run RT vs BE.
- **Scheduling classes**:
  - _RT-ISR-backed_: Extremely small, bounded handlers (e.g., stepper edge prep, watchdog kicks) with strict per-invocation budgets; preemption-disabled sections are tightly limited.
  - _RT-threaded_: Deterministic, bounded handlers running on a high-priority thread with cooperative yield points. Used for motion planning slices, thermal PID updates, and synchronized command sequencing.
  - _BE_: Lower-priority work that can be deferred or throttled (telemetry aggregation, UI updates, file IO prep).
- **Budgets & policing**: Each handler has a declared worst-case execution time and memory footprint. The host enforces per-call and per-period budgets; overruns trigger throttling, demotion to BE, or component unload if persistent.
- **Yielding & cooperativity**: RT-threaded handlers must yield at defined points (e.g., after N iterations or microseconds). Host APIs that can block are segregated to BE; RT-safe calls are non-blocking and time-bounded.
- **Scheduling alignment**: Motion/thermal handlers align to the host timebase and planner block boundaries. Periodic handlers can request cadence (e.g., every 10ms) and phase alignment; host may coalesce or jitter within bounds to maintain slack.
- **Priority resolution**: When multiple handlers are registered for the same verb/event, the dispatch policy chooses one of: explicit routing, priority order, or fan-out with composition rules. RT path never blocks on BE work; BE fan-out may run serially or concurrently subject to CPU budget.
- **Backpressure & queues**: BE queues are bounded; overflow triggers drop or coalesce strategies depending on handler type (e.g., keep-latest for telemetry). Components are notified of drops for optional compensation.
- **Deterministic randomness**: Host supplies a seeded PRNG for components that need randomness in RT contexts to avoid nondeterministic timing.
- **Testing hooks**: A simulation mode freezes time or runs with virtual time to validate budget adherence and ordering without hardware. This mode can also provide print time estimates.

## Safety model (pragmatic, not adversarial security)

- **Trust model**: Plugins are trusted and loaded at boot; focus is on printer safety, not hostile isolation. Boundary of concern is primarily at the (virtual) G-code/command layer to prevent unsafe sequences.
- **Kill paths & safe state**: Host maintains hard kill paths that bypass components: immediate heater cut, stepper disable, motion planner flush, PSU/bed/off relays where available. An emergency stop (E-stop) can be asserted by hardware, host, or policy violations.
- **Watchdogs**:
  - MCU watchdog for firmware lockups.
  - Host scheduler watchdogs for RT overruns; repeated violations can demote handlers or trigger E-stop.
  - Thermal watchdog that enforces max temps and rate-of-rise limits independent of component logic.
- **Interlocks**:
  - Thermal: max temp per tool/bed, sensor sanity (open/short), runaway detection, cooling time enforcement before power-down.
  - Motion: soft limits, endstop validation, homing-required gating, velocity/accel jerk ceilings, planner sanity on segment timing.
  - Power: optional brownout detection, current limits, and staged power-on sequencing.
- **Fault domains**: Component failures (panic/error) unload the component and unregister handlers; critical faults trip to safe state. Non-critical faults can degrade to BE or disable a feature while keeping core motion/thermal control alive.
- **State continuity**: On fault or restart, minimal state is restored (positions if homed, heater targets cleared). Components can provide restart hooks but are not trusted for safety-critical latching.
- **Diagnostics**: Structured fault codes with timestamps; last-N faults persisted for post-mortem. Optional “safe repro” mode to rerun with heaters/motion disabled for debugging.

## MCU comms and motion pipeline integration

- **Transport abstraction**: Support common links (USB CDC, UART, CAN, SPI) behind a framed, checksummed protocol with sequence numbers and retransmit. Host owns transport drivers; components interact via host motion/IO APIs, not raw transport.
- **Clocking & sync**: Host maintains a synchronized timebase with MCU(s) using periodic sync messages and drift estimation. Motion/thermal events scheduled in host time are translated to MCU ticks; jitter budgets are enforced.
- **Motion planner**: Planner lives in the host with a clear API to enqueue motion blocks. WASM components can submit high-level moves (lines/arcs/toolpaths) via a planner API; planner handles lookahead, jerk/accel limits, and segmentation to step timings.
- **Command staging**: Planner/IO commands are staged and streamed to MCU ahead of execution with queue depth monitoring. Backpressure signals prevent overruns; components are notified to throttle/segment as needed.
- **Sensing/feedback**: Sensor readings (temps, endstops, probes, load cells) are sampled by MCU and forwarded with timestamps; host aligns to its timebase and exposes a filtered view to components. Components can register interest in streams to reduce chatter.
- **Error handling**: Comms errors trigger retries; sustained faults trip safe state. MCU asserts fault codes (e.g., watchdog, overtemp, limit hit) that the host maps to interlocks and component notifications.
- **Multi-MCU**: Support multiple MCUs (toolhead, chamber, aux). Host coordinates time sync and distributes planner slices per domain, ensuring cross-device motion coherence.

## Klipper/Python interoperability strategy

- **Goal**: Reuse existing Klipper ecosystem modules with minimal rewrites while allowing migration of critical paths to Rust/WASM.
- **Approach options**:
  - _Transpile to WASM_ via [Python→WASM toolchains](https://component-model.bytecodealliance.org/language-support/building-a-simple-component/python.html) with a thin adapter to the host WIT APIs.
  - _API shims_ that emulate Klipper’s module API atop the host WIT surface, letting many modules run with minimal changes.
  - _Selective reimplementation_ for performance-critical pieces (e.g., motion/thermal loops), keeping higher-level policy in Python/WASM.
- **Bridging layer**: Provide a compatibility crate/runtime that maps common Klipper constructs (config objects, gcode/register handlers, reactor timers) onto host concepts (schemas, handler registration, scheduling classes).
- **Packaging**: Klipper-derived modules are packaged as WASM components with manifests declaring host API version and handler registrations; config schemas mirror the original module’s config.
- **Limitations**: Hard real-time pieces are better rewritten in Rust/WASM. Long-tail Python modules may incur overhead; guidance will encourage profiling and staged migration.

## Klipper/Kalico porting plan

- **Host vs component placement**

  - Rust host (built-in): MCU transport, timebase/scheduler, motion planner/queueing, thermal loops & safety interlocks, config store/validation, capability gating, logging/metrics, HTTP/WebSocket server, persistence, kill paths.
  - Rust utilities (ported from C helpers): kinematics/math aids, trapezoid/lookahead helpers, CRC/encoding, MCU framing helpers. Expose via host APIs so components can use them without FFI.
  - WASM components (default for Python logic): G-code/verb handlers, bed mesh, input shaping policy, fans/LEDs/macros, telemetry/reporting, UI-facing logic, job orchestration.
  - Optional host-builtins: extremely latency-sensitive math (e.g., input shaper/PID/autotune) if profiling shows Python/WASM overhead is too high.

- **Compat SDK for Python modules**

  - Provide a `klippy-compat` component SDK that re-creates `configfile`, `gcode.register_command`, `reactor` timers, `printer.lookup/add_object`, and logging atop the WIT APIs.
  - Map `gcode.register_command` to host handler registration with structured metadata (name, args, safety level, docs) and scheduling class hints.
  - Adapt config sections to component manifests + JSON schema; validated config is injected at init.
  - Offer an MCU proxy mapping `add_config_cmd` / `alloc_command` / `register_response` to motion/IO host APIs.
  - Packaging flow: use `componentize-py` (or similar) to compile modules to WASM with the compat SDK prelinked; auto-generate manifests from module metadata.
  - Edge: C extensions in plugins are not supported; rewrite or swap to Rust utilities.

- **Dialects: Klipper vs Kalico**
  - Identify API/config divergences between `vendor/klipper` and `vendor/kalico` (command names, defaults, helper behaviors).
  - Make the compat SDK dialect-aware: a manifest flag selects `klipper` or `kalico`, and the adapter adjusts defaults/names while keeping the WIT host surface stable.
  - Goal: existing plugins run unchanged; choosing a dialect is a manifest/config choice.

## Command documentation & REPL/autocomplete

- **Required metadata on registration**: command name/aliases, summary, args (name/type/default/units), side effects, safety class, examples, capability prerequisites.
- **Registry**: host stores docs alongside handler registrations for search and schema export.
- **HTTP/WS APIs**:
  - `/api/commands` for list/filter/search.
  - `/api/commands/{name}` for detailed docs.
  - `/api/repl` WebSocket for tab-complete, inline docs, and dry-run validation using the registry + config schemas.
- **Compat auto-fill**: `klippy-compat` extracts existing `help` strings from modules to populate docs.

## Moonraker integration (no UDS hop)

- Embed Moonraker-equivalent HTTP/WebSocket endpoints directly in the host; keep path/shape compatibility where practical for existing clients.
- Handle auth/session in-host; push events over WebSocket for status/prints/sensors/logs.
- Provide a thin compatibility plugin for any remaining Moonraker-specific endpoints using host APIs, so legacy clients keep working without the Unix domain socket indirection.

## Plugin loading and G-code macro expansion

- **Plugin loading at boot**: The host loads plugins specified in the configuration file at startup. Each plugin is a WebAssembly component that conforms to the plugin WIT interface.
- **Schema registration**: During initialization, plugins register:
  - Configuration schemas describing their settings (using JSON Schema format)
  - Command handler registrations with parameter schemas defining field names, types, requirements, defaults, and descriptions
  - Each handler declares its scheduling class (real-time vs best-effort)
- **Host registry**: The host maintains registries of all registered schemas and handlers, which become part of the active command vocabulary.
- **Macro expansion pipeline**: The key insight is that plugins provide schemas, not implementations, at job compile time:
  1. A G-code job is submitted (or compiled from raw G-code)
  2. The compiler queries the plugin registry to get all registered command schemas
  3. For each command in the job, the compiler generates builder code that validates parameters against the registered schema
  4. The result is a job WASM component that expands the high-level commands into structured calls to plugin handlers
  5. At runtime, the job component calls the builder interfaces, which dispatch to the actual plugin implementations
- **Two-phase compilation**: This creates a clean separation:
  - **Compile time**: Job is expanded and validated against schemas; produces a WASM module with explicit calls to builder interfaces
  - **Runtime**: The host links the job component to the actual plugin implementations and executes
- **Benefits of this approach**:
  - Jobs are pre-validated against plugin schemas before execution
  - The job WASM acts as a "compiled macro expansion" of the original G-code
  - Plugins can update their implementations without recompiling jobs (as long as schemas remain compatible)
  - Clear API boundary between job compilation and plugin execution
  - Enables static analysis, time estimation, and toolpath preview by analyzing the expanded job component
- **WIT interface contract**: The plugin WIT defines:
  - `scherzo:plugin/types` - Common types for schemas, field definitions, and handlers
  - `scherzo:plugin/registry` - Host-provided functions for plugins to register schemas and handlers
  - `scherzo:plugin/lifecycle` - Plugin initialization, info, and cleanup exports
- **G-code macro equivalence**: This mechanism replaces traditional G-code macros (like Klipper's `[gcode_macro]`) with a more structured, typed, and validated approach. Each command handler is essentially a typed macro with explicit parameter schemas.

## Tooling, testing, and simulation

- **Developer tooling**: CLI to scaffold components from WIT (bindings, manifest templates, config schema stubs). Local runner to execute components against a mock host.
- **Lint/CI**: Schema linting, manifest validation, WIT version checks, and deterministic budget enforcement in CI. Golden-file tests for handler dispatch and config ingestion.
- **Simulation**: Headless simulator with virtual time, synthetic sensors, and motion/thermal models, using `bach`. Supports trace capture and replay for regression, plus “dry run” of print components with no heaters/motion.
- **Profiling**: Per-handler budget metrics, queue depths, and timing histograms exported for analysis; optional flamegraph-style sampling in BE context.
- **Test data corpus**: Shared test-data directory with representative print workloads, failure traces, and module configs for regression and performance baselines.
