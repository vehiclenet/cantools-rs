# AGENTS.md - vehiclenet/cantools-rs

## Purpose

This is the product and architecture document for development of `cantools-rs`.
It should guide long-term implementation decisions, crate boundaries, public APIs,
and quality bars.

## Project Identity

- **Name:** `cantools-rs`
- **Repo:** `github.com/vehiclenet/cantools-rs`
- **Description:** A Rust CAN toolkit with a Linux SocketCAN backend and portable
  protocol and decode layers for DBC, ISO-TP, J1939, UDS, and OBD workflows.
- **License:** Dual MIT / Apache 2.0 (SPDX: `MIT OR Apache-2.0`)
- **Author:** Dylan Walker Brown <dylan@walkerbrown.org>
- **MSRV:** Rust stable (latest - 2 releases; set explicitly in workspace
  `Cargo.toml`)
- **Primary backend today:** Linux via SocketCAN
- **Long-term direction:** portable upper-layer crates with backend-specific
  transport crates

## Product Direction

- The primary user surface is a single `cantools` CLI with descriptive verbs.
- `cantools monitor` is the canonical TUI entrypoint.
- Linux SocketCAN is the first backend, but protocol and decode crates should
  remain portable where practical.
- CAN FD is first-class throughout; never assume 8-byte payloads.
- Interoperability with existing Linux CAN tooling matters, but internal data
  models must not be constrained by any one external log format or UI.

## Architectural Principles

- Keep OS-specific APIs isolated in backend crates. The Linux backend crate is
  `cantools-socketcan`.
- Keep portable crates runtime-agnostic. Blocking, nonblocking, and poll-friendly
  APIs are fine; Tokio-specific async adapters belong in backend or integration
  crates, not portable protocol crates.
- Preserve raw data and annotate it rather than dropping it during decode or
  analysis.
- Prefer explicit user intent over magic defaults. Discovery is a convenience,
  not a replacement for explicit configuration.
- Use feature flags to keep heavyweight UI and runtime dependencies optional
  where practical.
- Keep crate responsibilities narrow and non-overlapping.
- Breaking changes before `1.0` are encouraged. No need to deprecate or document
  design changes in comments or otherwise.
- Generated protocol tables should be checked into the repository. Builds must
  not depend on network access or build-time scraping.

## Workspace Conventions

- The workspace should define shared package metadata and have member crates
  inherit from it where possible.
- Crate names should stay in the `cantools-*` family.
- Public crate dependencies should remain intentional and easy to explain.
  Avoid circular dependencies and dependency bloat.
- Deeper design rationale may live in `docs/ARCHITECTURE.md`, but this file is
  the normative source for crate boundaries and product behavior.

## Workspace Crates

### `cantools-core`

- Owns portable CAN data model primitives and shared contracts.
- Owns canonical ID, frame, frame-class, and capture-envelope types.
- Owns shared error and trait abstractions needed across multiple crates.
- Must not own Linux SocketCAN, ISO-TP socket, or J1939 socket bindings.
- Should compile without Linux-specific dependencies in its public surface.

### `cantools-socketcan`

- Owns all Linux-specific CAN backend code.
- Talks directly to the kernel `PF_CAN` API via `libc` or equivalent low-level
  bindings.
- Owns raw SocketCAN, CAN FD, ISO-TP socket, and J1939 socket backends.
- Owns Linux interface management and any backend-specific timestamp or ancillary
  data handling.
- May expose optional runtime integrations and async adapters for Linux use
  cases.
- Must not become the home for portable protocol logic.

### `cantools-dbc`

- Owns DBC-backed signal decoding, not just parsing.
- Uses `can-dbc` as the parser implementation detail.
- Public API is frame-in, decoded-signals-out with scaling, units, enums,
  multiplexing, and decode diagnostics.

### `cantools-isotp`

- Owns portable ISO 15765-2 protocol logic.
- Owns addressing, segmentation and reassembly, flow control, timing, and
  transport-facing abstractions above the backend layer.
- Must not own Linux or OS-specific ISO-TP socket bindings.

### `cantools-j1939`

- Owns portable J1939 modeling and protocol behavior.
- Owns PGN, SPN, and NAME types, address claim behavior, transport-protocol
  session logic, and passive decode helpers.
- Must not own Linux or OS-specific sockets or interface management.

### `cantools-uds`

- Owns portable ISO 14229 diagnostics types, codecs, and client-first workflows.
- Includes server-side codec and type support for emulators, mock ECUs, and tests.
- A minimal reference UDS server is acceptable, but should clearly marked 
  as unsuitable for production use.
- Depends on ISO-TP abstractions, not Linux socket APIs.

### `cantools-obd`

- Owns OBD over CAN/ISO-TP functionality.
- May build on `cantools-uds` where the layering makes sense.
- Does not promise legacy transport families such as ISO 9141, KWP, or K-line.

### `cantools-cli`

- Installs a single `cantools` binary.
- Is the only canonical user-facing executable package.
- Uses clear verbs such as `send`, `dump`, `gen`, `sequence`, `play`, and
  `monitor`.
- Should enable TUI support by default for normal installs.
- Should support smaller non-TUI builds that omit `monitor` and avoid `ratatui`
  or Tokio dependencies when those features are disabled.

### `cantools-tui`

- Owns the operator-facing terminal UI used by `cantools monitor`.
- Is an implementation crate behind `cantools monitor`, not a parallel
  user-facing product surface.
- Should be an optional dependency of `cantools-cli`.
- Focuses on live inspection, filtering, decode, and capture workflows.
- Owns `ratatui`, terminal event handling, and any runtime-specific monitor-loop
  integration needed by the TUI.
- Reuses backend and protocol crates rather than duplicating logic.

## Dependency Intent

- `cantools-core` is the portable foundation.
- `cantools-socketcan`, `cantools-dbc`, `cantools-isotp`, and `cantools-j1939`
  depend on `cantools-core`.
- `cantools-uds` depends on `cantools-isotp`.
- `cantools-obd` depends on `cantools-uds`.
- `cantools-cli` integrates the library crates above and should depend on
  `cantools-tui` only through an optional feature.
- `cantools-tui` may depend on backend and runtime crates needed for interactive
  monitoring.
- Portable crates must not depend on `cantools-socketcan`; backend dependencies
  point inward, not upward.

## Public API Style

- Lower-level libraries are runtime-agnostic. Do not make Tokio or another async
  runtime part of the public contract for portable crates.
- Blocking, nonblocking, and poll-friendly APIs are acceptable. Optional async
  adapters belong in `cantools-socketcan` or other backend or integration crates.
- `ratatui`, terminal UI state management, and Tokio-specific event-loop code
  should stay in `cantools-tui` or other integration layers, not in portable
  protocol crates or general CLI code paths.
- All public APIs must have rustdoc and runnable examples or doctests where
  practical. Use `no_run` only when hardware or kernel access makes execution
  impossible in docs.
- Library crates should prefer strong types over stringly typed identifiers.
- Error types use `thiserror` in library crates and `anyhow` in binary crates.
- All `unsafe` blocks must have `// SAFETY:` comments explaining the invariant.

## Code Style and Commentary

- Prefer a literate coding style with more commentary than is typical in Rust
  codebases.
- Write comments to explain intent, protocol context, invariants, and why a
  piece of code is structured the way it is, not only what it does.
- Favor explanatory comments around bit layout logic, state machines, transport
  sequencing, parsing rules, error handling decisions, and other areas where the
  code reflects an external specification.
- Use short module-level or function-level overviews when they help a reader
  build the right mental model before reading the implementation.
- ASCII diagrams are encouraged when they make packet layouts, state
  transitions, or control flow easier to understand.
- Comments should still be concrete and informative. Avoid filler comments that
  add words without adding understanding.
- Avoid self-reflective comments on changes in strategy or approach.

## CAN Data Model

- The raw frame model must represent:
  - standard and extended data frames
  - classic RTR frames
  - backend-surfaced error frames
  - CAN FD payloads and CAN FD flags such as BRS and ESI
- Metadata such as timestamp, interface name or index, direction, and capture
  context belongs in a separate capture or event envelope, not embedded in every
  protocol type.
- Higher-layer protocol crates should operate on portable frame and event
  abstractions, not OS-specific structs.
- Passive tools must preserve and surface raw frames even when higher-level
  decode fails.

## DBC Decode Semantics

- Support explicit DBC paths and documented discovery. Explicit user input takes
  precedence over discovery.
- Support multiple DBCs loaded in user-specified order.
- Matching must be deterministic. Overlapping definitions must surface ambiguity
  diagnostics rather than silently picking an arbitrary result.
- Decoded signals should be rich structured values carrying raw extraction,
  scaled value, unit, enum or display metadata, and other formatting context
  needed by callers.
- Decode misses or malformed payloads should annotate the frame or event rather
  than dropping it or aborting passive inspection.
- Multiplexing, endianness, signedness, scaling, offsets, ranges, and enum
  resolution are first-class parts of the decode contract.

## Protocol Coverage Matrix

- `cantools-isotp`
  - In scope: portable ISO-TP frame handling, segmentation and reassembly,
    addressing behavior, flow control, and timing-sensitive transport logic.
  - Out of scope: Linux socket APIs and runtime-specific async integrations.
- `cantools-j1939`
  - In scope: 29-bit J1939 ID interpretation, PGN, SPN, and NAME modeling,
    address claim behavior, transport protocol session logic, and passive decode.
  - Out of scope: OS-specific sockets and interface management.
- `cantools-uds`
  - In scope: generated service and negative-response definitions, request and
    response codecs, tester-oriented client flows, and server-side message types
    useful for emulators and tests.
  - Out of scope: full ECU application frameworks or backend socket ownership.
- `cantools-obd`
  - In scope: generated mode and PID definitions and helpers for OBD over
    CAN/ISO-TP.
  - Out of scope: legacy OBD transport stacks.

## CLI

- The canonical executable is `cantools`.
- Commands should use descriptive verbs rather than abbreviations.
- `cantools monitor` is the canonical entrypoint for live terminal monitoring.
- Default builds should include TUI support.
- `cantools monitor` may be feature-gated in reduced builds; when disabled, the
  CLI should fail clearly rather than implicitly pulling in TUI dependencies.
- Default output should optimize for operator readability.
- Machine-readable output may exist on a best-effort basis, but AGENTS guidance
  does not promise stable CLI schemas as a public compatibility contract.
- Common concepts such as interface selection, DBC input, and capture paths
  should use consistent flag names across subcommands where practical.
- Invalid input, transport failures, and protocol failures should produce clear
  error messages and non-zero exit codes.

## TUI

- The TUI is an operator tool for live sniffing, filtering, decode, and capture.
- It should work as `cantools monitor` even if implemented by a dedicated
  `cantools-tui` crate behind the scenes.
- The TUI should remain an optional layer so non-interactive CLI builds can stay
  smaller and avoid terminal UI runtime requirements.
- Live raw traffic visibility is the baseline experience.
- DBC and J1939 decoding are enhancements to raw visibility, not replacements
  for it.
- Recording and export are first-class workflows.
- The TUI should degrade gracefully when no DBC is loaded or when protocol decode
  is unavailable.

## Platform and Backend Policy

- Linux via `cantools-socketcan` is the primary supported backend today.
- Portable crates should avoid direct OS dependencies and remain buildable on
  non-Linux targets where practical.
- Additional backend crates may be added later without changing the
  responsibilities of portable crates.
- New OS-specific API surface belongs in backend crates, not in `cantools-core`,
  `cantools-isotp`, `cantools-j1939`, `cantools-uds`, or `cantools-obd`.

## Capture, Replay, and Interoperability

- Internal capture models should be richer than any one external file format.
- Capture data should be able to carry timestamps, interface metadata, direction,
  raw frame data, error events, and optional decode annotations.
- Import and export with can-utils-compatible formats should be supported where
  practical for interoperability with existing Linux CAN tooling.
- External format compatibility must not force lossy internal APIs. If an export
  format cannot preserve all metadata, the loss should be explicit.
- Replay workflows should be based on raw captured events, not only on decoded or
  protocol-specific views.

## Testing, Documentation, and Codegen

- Every library crate should have focused unit tests for pure logic.
- Public APIs should have examples or doctests that stay in sync with the code.
- DBC, J1939, ISO-TP, UDS, and OBD logic should have fixture-driven tests.
- Linux backend behavior and end-to-end flows that depend on kernel semantics
  should have `vcan` integration coverage.
- CI should cover:
  - stable and MSRV toolchains
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo build --workspace`
  - `cargo test --workspace`
  - `cargo deny check`
- Integration tests that require `vcan` or kernel support should be isolated so
  they can run in a dedicated job or explicitly provisioned environment.
- UDS and OBD specification tables should come from checked-in source data or
  scripts. Generated Rust should be checked into the repository, and regeneration
  must be documented and reviewable.

## README Expectations

- The README should describe the single-binary `cantools` CLI model.
- It should show quick-start usage for several clear verbs, including
  `cantools monitor`.
- It should document that TUI support is enabled by default and explain how to
  disable it in smaller feature-gated builds.
- It should document backend expectations, DBC usage, and major feature areas
  such as CAN FD, DBC decoding, J1939, UDS, and OBD.
- It should explain interoperability with existing Linux CAN tooling without
  presenting compatibility aliases as the primary UX.
