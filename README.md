# cantools-rs

`cantools-rs` is a Rust CAN toolkit centered on a single `cantools` CLI and a
set of narrowly scoped library crates.

## Workspace overview

- `cantools-core`: canonical portable CAN data model.
- `cantools-codec`: capture/log codecs with MF4 as the preferred format and ASC /
  BLF as compatibility layers.
- `cantools-socketcan`: Linux backend for raw CAN, CAN FD, ISO-TP, and J1939.
- `cantools-dbc`, `cantools-isotp`, `cantools-j1939`, `cantools-uds`,
  `cantools-obd`: portable decode and protocol crates.
- `cantools-tui`: implementation crate behind `cantools monitor`.
- `cantools-cli`: installs the `cantools` binary.

## Quick start

```sh
cargo run -p cantools-cli -- dump --interface vcan0
cargo run -p cantools-cli -- send --interface vcan0 --id 123 --data 01020304
cargo run -p cantools-cli -- monitor --interface vcan0
```

## Capture formats

- `.mf4` is the preferred persisted capture format.
- `.asc` and `.blf` are supported as best-effort compatibility formats.
- Internal capture APIs preserve richer metadata than any on-disk format.

## Development

- Portable crates and most CLI logic build on macOS.
- Linux-only backend work and `vcan` integration tests can run via OrbStack:

```sh
orb -m debian cargo test --workspace
```

