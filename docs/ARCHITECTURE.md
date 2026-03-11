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
- GitHub-hosted CI does not provision `vcan`, so hosted jobs validate portable
  checks only while SocketCAN runtime checks remain local.
- A tracked `pre-push` hook gates pushes to `main` on the full local suite,
  including OrbStack Debian build, test, `cargo deny`, and live `vcan0` smoke
  coverage.

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

## CI and local push gating

Hosted GitHub CI is intentionally weaker than the local `main` push gate. The
hosted macOS job covers formatting, Clippy, and workspace tests. The hosted
Linux job covers build, test, and `cargo deny`, but does not attempt runtime
SocketCAN validation because GitHub-hosted runner kernels do not provide a
usable `vcan` path for this project.

The authoritative local gate lives in `xtask`:

- `ci-hosted` mirrors the hosted checks that run on the developer machine.
- `ci-linux-local` provisions `vcan0` in OrbStack `debian`, then runs Linux
  build, test, `cargo deny`, and a live `cantools dump`/`cantools send` smoke
  test.
- `ci-all-local` runs both suites and is the command invoked by `.githooks/pre-push`
  when a push targets `main`.
