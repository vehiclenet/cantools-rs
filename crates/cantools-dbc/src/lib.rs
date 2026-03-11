//! DBC-backed frame decoding.

use std::{fs, path::Path};

use can_dbc::{ByteOrder, Dbc, MessageId, MultiplexIndicator, Signal, ValDescription, ValueType};
use cantools_core::{CanFrame, CanId};
use thiserror::Error;

/// Decode errors returned while loading DBC data.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// DBC parsing failed.
    #[error(transparent)]
    Parse(#[from] can_dbc::DbcError),
}

/// Structured decode result for one signal.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSignal {
    /// Signal name from the DBC.
    pub name: String,
    /// Raw integer value after bit extraction and sign extension.
    pub raw: i64,
    /// Scaled numeric value.
    pub scaled: f64,
    /// Engineering unit from the DBC when present.
    pub unit: Option<String>,
    /// Enumerated display text when the DBC defines one.
    pub enum_text: Option<String>,
}

/// Decoded message payload and signal set.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedMessage {
    /// Source database label.
    pub database: String,
    /// Message name from the DBC.
    pub message_name: String,
    /// Decoded signal values.
    pub signals: Vec<DecodedSignal>,
}

/// Decode diagnostics surfaced alongside passive decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    /// No loaded DBC matched the frame identifier.
    NoMatch {
        /// Raw CAN identifier.
        id: u32,
    },
    /// More than one loaded DBC matched the frame identifier.
    AmbiguousMessage {
        /// Raw CAN identifier.
        id: u32,
        /// Database labels that matched the same identifier.
        databases: Vec<String>,
    },
    /// A signal extended past the payload length.
    PayloadTooShort {
        /// Signal name.
        signal: String,
        /// Expected payload width in bits.
        expected_bits: usize,
        /// Actual payload length in bytes.
        actual_bytes: usize,
    },
}

/// Outcome of decoding a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeOutcome {
    /// Successfully decoded message when the match is unambiguous.
    pub message: Option<DecodedMessage>,
    /// Diagnostics emitted during matching or extraction.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
struct LoadedDbc {
    name: String,
    dbc: Dbc,
}

/// Deterministic multi-DBC decoder.
#[derive(Debug, Default)]
pub struct Decoder {
    databases: Vec<LoadedDbc>,
}

impl Decoder {
    /// Construct an empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a DBC from a string and store it under a caller-provided label.
    pub fn push_named(&mut self, name: impl Into<String>, dbc_text: &str) -> Result<(), Error> {
        self.databases.push(LoadedDbc {
            name: name.into(),
            dbc: Dbc::try_from(dbc_text)?,
        });
        Ok(())
    }

    /// Load a DBC file and use the filename as the database label.
    pub fn push_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        let path = path.as_ref();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dbc")
            .to_string();
        let content = fs::read_to_string(path)?;
        self.push_named(name, &content)
    }

    /// Decode a frame against the loaded DBCs.
    pub fn decode_frame(&self, frame: &CanFrame) -> DecodeOutcome {
        let mut matches = Vec::new();
        for database in &self.databases {
            if let Some(message) = database
                .dbc
                .messages
                .iter()
                .find(|message| message.id.raw() == frame_message_id(frame.id))
            {
                matches.push((database, message));
            }
        }

        if matches.is_empty() {
            return DecodeOutcome {
                message: None,
                diagnostics: vec![Diagnostic::NoMatch { id: frame.id.raw() }],
            };
        }

        if matches.len() > 1 {
            return DecodeOutcome {
                message: None,
                diagnostics: vec![Diagnostic::AmbiguousMessage {
                    id: frame.id.raw(),
                    databases: matches
                        .iter()
                        .map(|(database, _)| database.name.clone())
                        .collect(),
                }],
            };
        }

        let (database, message) = matches.remove(0);
        let mut diagnostics = Vec::new();
        let mut signals = Vec::new();
        let mux_value = database
            .dbc
            .message_multiplexor_switch(message.id)
            .ok()
            .flatten()
            .and_then(|signal| extract_raw(&frame.data, signal).ok())
            .map(|value| value as i64);

        for signal in &message.signals {
            if !signal_active(signal, mux_value) {
                continue;
            }

            match extract_signal(database, &message.id, signal, &frame.data) {
                Ok(signal) => signals.push(signal),
                Err(DiagnosticError::PayloadTooShort {
                    signal,
                    expected_bits,
                    actual_bytes,
                }) => diagnostics.push(Diagnostic::PayloadTooShort {
                    signal,
                    expected_bits,
                    actual_bytes,
                }),
            }
        }

        DecodeOutcome {
            message: Some(DecodedMessage {
                database: database.name.clone(),
                message_name: message.name.clone(),
                signals,
            }),
            diagnostics,
        }
    }
}

#[derive(Debug)]
enum DiagnosticError {
    PayloadTooShort {
        signal: String,
        expected_bits: usize,
        actual_bytes: usize,
    },
}

fn signal_active(signal: &Signal, mux_value: Option<i64>) -> bool {
    match signal.multiplexer_indicator {
        MultiplexIndicator::Plain => true,
        MultiplexIndicator::Multiplexor => true,
        MultiplexIndicator::MultiplexedSignal(expected) => mux_value == Some(expected as i64),
        MultiplexIndicator::MultiplexorAndMultiplexedSignal(expected) => {
            mux_value.is_none() || mux_value == Some(expected as i64)
        }
    }
}

fn extract_signal(
    database: &LoadedDbc,
    message_id: &MessageId,
    signal: &Signal,
    payload: &[u8],
) -> Result<DecodedSignal, DiagnosticError> {
    let raw = extract_raw(payload, signal)?;
    let signed = if matches!(signal.value_type, ValueType::Signed) {
        sign_extend(raw, signal.size as u32)
    } else {
        raw as i64
    };
    let scaled = (signed as f64 * signal.factor) + signal.offset;
    let enum_text = database
        .dbc
        .value_descriptions_for_signal(*message_id, &signal.name)
        .and_then(|descriptions| {
            descriptions
                .iter()
                .find(|value: &&ValDescription| value.id == signed)
                .map(|value| value.description.clone())
        });

    Ok(DecodedSignal {
        name: signal.name.clone(),
        raw: signed,
        scaled,
        unit: if signal.unit.is_empty() {
            None
        } else {
            Some(signal.unit.clone())
        },
        enum_text,
    })
}

fn extract_raw(payload: &[u8], signal: &Signal) -> Result<u64, DiagnosticError> {
    let start = signal.start_bit as usize;
    let length = signal.size as usize;
    let max_bit = max_signal_bit(signal);
    if max_bit >= payload.len() * 8 {
        return Err(DiagnosticError::PayloadTooShort {
            signal: signal.name.clone(),
            expected_bits: max_bit + 1,
            actual_bytes: payload.len(),
        });
    }

    Ok(match signal.byte_order {
        ByteOrder::LittleEndian => extract_little_endian(payload, start, length),
        ByteOrder::BigEndian => extract_big_endian(payload, start, length),
    })
}

fn max_signal_bit(signal: &Signal) -> usize {
    let start = signal.start_bit as usize;
    let length = signal.size as usize;
    match signal.byte_order {
        ByteOrder::LittleEndian => start + length.saturating_sub(1),
        ByteOrder::BigEndian => {
            let mut bit = start;
            for _ in 1..length {
                bit = if bit.is_multiple_of(8) {
                    bit + 15
                } else {
                    bit - 1
                };
            }
            bit
        }
    }
}

fn bit_at(payload: &[u8], bit: usize) -> u8 {
    (payload[bit / 8] >> (bit % 8)) & 1
}

fn extract_little_endian(payload: &[u8], start: usize, length: usize) -> u64 {
    let mut value = 0_u64;
    for offset in 0..length {
        value |= u64::from(bit_at(payload, start + offset)) << offset;
    }
    value
}

fn extract_big_endian(payload: &[u8], start: usize, length: usize) -> u64 {
    let mut value = 0_u64;
    let mut bit = start;
    for _ in 0..length {
        value = (value << 1) | u64::from(bit_at(payload, bit));
        bit = if bit.is_multiple_of(8) {
            bit + 15
        } else {
            bit - 1
        };
    }
    value
}

fn sign_extend(raw: u64, width: u32) -> i64 {
    let shift = 64 - width;
    ((raw << shift) as i64) >> shift
}

fn frame_message_id(id: CanId) -> u32 {
    if id.is_extended() {
        0x8000_0000 | id.raw()
    } else {
        id.raw()
    }
}

#[cfg(test)]
mod tests {
    use cantools_core::{CanFrame, CanId, FdFlags, FrameClass};

    use super::{DecodeOutcome, Decoder, Diagnostic};

    const SIMPLE_DBC: &str = r#"
VERSION ""
NS_ :
BS_:
BU_: Vector__XXX
BO_ 291 ExampleMessage: 8 Vector__XXX
 SG_ Speed : 0|16@1+ (0.1,0) [0|250] "km/h" Vector__XXX
 SG_ Mode M : 16|8@1+ (1,0) [0|3] "" Vector__XXX
 SG_ Temp m1 : 24|8@1- (1,-40) [-40|215] "C" Vector__XXX
VAL_ 291 Mode 0 "Idle" 1 "Drive";
"#;

    #[test]
    fn decodes_signals_and_enums() {
        let mut decoder = Decoder::new();
        decoder.push_named("vehicle.dbc", SIMPLE_DBC).expect("dbc");

        let frame = CanFrame::new(
            CanId::standard(0x123).expect("id"),
            FrameClass::Data,
            vec![0x10, 0x27, 0x01, 0x50],
            false,
            FdFlags::default(),
        )
        .expect("frame");

        let outcome = decoder.decode_frame(&frame);
        let message = outcome.message.expect("decoded message");
        assert!(outcome.diagnostics.is_empty());
        assert_eq!(message.message_name, "ExampleMessage");
        assert_eq!(message.signals.len(), 3);
        assert_eq!(message.signals[0].name, "Speed");
        assert_eq!(message.signals[0].scaled, 1000.0);
        assert_eq!(message.signals[1].enum_text.as_deref(), Some("Drive"));
    }

    #[test]
    fn reports_ambiguity_across_databases() {
        let mut decoder = Decoder::new();
        decoder.push_named("a.dbc", SIMPLE_DBC).expect("dbc");
        decoder.push_named("b.dbc", SIMPLE_DBC).expect("dbc");

        let frame = CanFrame::new(
            CanId::standard(0x123).expect("id"),
            FrameClass::Data,
            vec![0; 8],
            false,
            FdFlags::default(),
        )
        .expect("frame");

        let outcome = decoder.decode_frame(&frame);
        assert!(outcome.message.is_none());
        assert_eq!(
            outcome.diagnostics,
            vec![Diagnostic::AmbiguousMessage {
                id: 0x123,
                databases: vec!["a.dbc".to_string(), "b.dbc".to_string()],
            }]
        );
    }

    #[test]
    fn reports_short_payloads() {
        let mut decoder = Decoder::new();
        decoder.push_named("vehicle.dbc", SIMPLE_DBC).expect("dbc");

        let frame = CanFrame::new(
            CanId::standard(0x123).expect("id"),
            FrameClass::Data,
            vec![0x01],
            false,
            FdFlags::default(),
        )
        .expect("frame");

        let outcome = decoder.decode_frame(&frame);
        assert!(matches!(
            outcome,
            DecodeOutcome {
                message: Some(_),
                ref diagnostics,
            } if !diagnostics.is_empty()
        ));
    }
}
