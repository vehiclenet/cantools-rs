//! Generated OBD tables. Regenerate with `cargo run -p xtask -- codegen`.

/// Generated OBD PID metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedPid {
    /// OBD service mode.
    pub mode: u8,
    /// OBD parameter identifier.
    pub pid: u8,
    /// Human-readable PID name.
    pub name: &'static str,
    /// Engineering unit for decoded values.
    pub unit: &'static str,
}

/// Generated OBD PIDs keyed by service mode and PID.
pub const GENERATED_PIDS: &[GeneratedPid] = &[
    GeneratedPid {
        mode: 0x01,
        pid: 0x04,
        name: "CalculatedEngineLoad",
        unit: "%",
    },
    GeneratedPid {
        mode: 0x01,
        pid: 0x05,
        name: "EngineCoolantTemperature",
        unit: "C",
    },
    GeneratedPid {
        mode: 0x01,
        pid: 0x0C,
        name: "EngineRpm",
        unit: "rpm",
    },
    GeneratedPid {
        mode: 0x01,
        pid: 0x0D,
        name: "VehicleSpeed",
        unit: "km/h",
    },
    GeneratedPid {
        mode: 0x09,
        pid: 0x02,
        name: "Vin",
        unit: "",
    },
];
