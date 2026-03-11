//! Generated J1939 tables. Regenerate with `cargo run -p xtask -- codegen`.

/// Generated J1939 PGN metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedPgn {
    /// Parameter Group Number.
    pub pgn: u32,
    /// Human-readable PGN name.
    pub name: &'static str,
}

/// Generated J1939 PGN definitions keyed by PGN.
pub const GENERATED_PGNS: &[GeneratedPgn] = &[
    GeneratedPgn {
        pgn: 0x00F004,
        name: "ElectronicEngineController1",
    },
    GeneratedPgn {
        pgn: 0x00FEE9,
        name: "VehicleDirectionSpeed",
    },
    GeneratedPgn {
        pgn: 0x00FEEB,
        name: "ComponentIdentification",
    },
];
