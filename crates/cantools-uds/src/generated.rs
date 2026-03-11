//! Generated UDS tables. Regenerate with `cargo run -p xtask -- codegen`.

/// Generated UDS service metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedService {
    /// ISO 14229 service identifier.
    pub sid: u8,
    /// Human-readable service name.
    pub name: &'static str,
}

/// Generated UDS negative response metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedNegativeResponse {
    /// ISO 14229 negative response code.
    pub code: u8,
    /// Human-readable negative response name.
    pub name: &'static str,
}

/// Generated UDS services keyed by service identifier.
pub const GENERATED_SERVICES: &[GeneratedService] = &[
    GeneratedService {
        sid: 0x10,
        name: "DiagnosticSessionControl",
    },
    GeneratedService {
        sid: 0x11,
        name: "EcuReset",
    },
    GeneratedService {
        sid: 0x22,
        name: "ReadDataByIdentifier",
    },
    GeneratedService {
        sid: 0x3E,
        name: "TesterPresent",
    },
];

/// Generated UDS negative responses keyed by response code.
pub const GENERATED_NEGATIVE_RESPONSES: &[GeneratedNegativeResponse] = &[
    GeneratedNegativeResponse {
        code: 0x11,
        name: "ServiceNotSupported",
    },
    GeneratedNegativeResponse {
        code: 0x12,
        name: "SubFunctionNotSupported",
    },
    GeneratedNegativeResponse {
        code: 0x22,
        name: "ConditionsNotCorrect",
    },
];
