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
- GitHub-hosted CI intentionally validates portable build, lint, and unit-test
  coverage only. Runtime SocketCAN coverage is local until a self-hosted Linux
  runner exists.
- Install the tracked Git hook to gate pushes to `main` on the full local CI
  suite:

```sh
cargo run -p xtask -- install-hooks
```

- Run the same full local gate manually when needed:

```sh
cargo run -p xtask -- ci-all-local
```

- Linux-only backend work and `vcan` validation run via OrbStack `debian`:

```sh
orb -m debian -u root sh -lc 'modprobe vcan || true; ip link add dev vcan0 type vcan || true; ip link set up vcan0'
orb -m debian sh -lc '. ~/.cargo/env && cd /Users/dylan/Developer/vehiclenet/cantools-rs && cargo test --workspace'
orb -m debian sh -lc '. ~/.cargo/env && cd /Users/dylan/Developer/vehiclenet/cantools-rs && target/debug/cantools dump --interface vcan0 --count 1'
orb -m debian sh -lc '. ~/.cargo/env && cd /Users/dylan/Developer/vehiclenet/cantools-rs && target/debug/cantools send --interface vcan0 --id 123 --data 01020304'
```

## License

Copyright 2026 Dylan Walker Brown

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
