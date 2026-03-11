# cantools-rs architecture

`AGENTS.md` is the normative product and boundary document. This file captures
the concrete implementation choices made to bootstrap the repository.

## Chosen defaults

- `cantools-core` owns the canonical portable capture model.
- `cantools-codec` owns file-format translation only. It never becomes the home
  for the in-memory event model.
- MF4 is the first-class persisted capture format. ASC and BLF are best-effort
  import/export layers with explicit fidelity diagnostics.
- `cantools-cli` uses `clap`.
- `cantools-tui` uses `ratatui` and `crossterm`.
- Local Linux validation runs through OrbStack's `debian` machine.
- `xtask` owns repository maintenance commands such as placeholder checks and
  fixture/codegen workflows.

## Capture model and codec split

The internal event model preserves richer semantics than external files:
timestamps, interface identity, direction, raw frame kind, CAN FD flags,
backend-specific diagnostics, and optional decode annotations. File codecs
translate to and from that model. When an external format cannot represent part
of the event, the codec records a fidelity note instead of silently dropping
information.

## Reduced builds

The CLI keeps `monitor` behind a feature so reduced builds can omit TUI
dependencies. When the feature is disabled the command fails clearly instead of
silently disappearing from help text.

