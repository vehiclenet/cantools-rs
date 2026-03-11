//! Portable CAN model primitives shared across the workspace.

use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

/// Workspace-wide result type for core operations.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Errors returned when building or validating portable CAN values.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The identifier exceeds the valid bit width for the requested format.
    #[error("invalid CAN identifier {value:#x} for {kind}")]
    InvalidCanId {
        /// Raw identifier value.
        value: u32,
        /// Human-readable kind name.
        kind: &'static str,
    },
    /// The payload length does not match CAN or CAN FD limits.
    #[error("payload length {length} exceeds {max} bytes for {kind}")]
    InvalidPayloadLength {
        /// Requested payload length.
        length: usize,
        /// Maximum allowed length.
        max: usize,
        /// Frame kind description.
        kind: &'static str,
    },
    /// System time values earlier than the Unix epoch cannot be represented.
    #[error("system time precedes the unix epoch")]
    TimestampBeforeEpoch,
}

/// Canonical CAN identifier representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanId {
    raw: u32,
    extended: bool,
}

impl CanId {
    /// Create a standard 11-bit identifier.
    pub fn standard(raw: u16) -> Result<Self> {
        if raw > 0x7ff {
            return Err(CoreError::InvalidCanId {
                value: raw.into(),
                kind: "standard identifier",
            });
        }

        Ok(Self {
            raw: raw.into(),
            extended: false,
        })
    }

    /// Create an extended 29-bit identifier.
    pub fn extended(raw: u32) -> Result<Self> {
        if raw > 0x1fff_ffff {
            return Err(CoreError::InvalidCanId {
                value: raw,
                kind: "extended identifier",
            });
        }

        Ok(Self {
            raw,
            extended: true,
        })
    }

    /// Returns the raw identifier value.
    pub fn raw(self) -> u32 {
        self.raw
    }

    /// Returns `true` when this is a 29-bit identifier.
    pub fn is_extended(self) -> bool {
        self.extended
    }
}

/// Coarse frame class. Detailed protocol metadata belongs elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClass {
    /// Standard or extended data frame.
    Data,
    /// Classic CAN remote frame.
    Remote,
    /// Backend-surfaced error frame.
    Error,
}

/// CAN FD flags that are orthogonal to identifier or capture metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FdFlags {
    /// Bit rate switch.
    pub bit_rate_switch: bool,
    /// Error state indicator.
    pub error_state_indicator: bool,
}

/// Raw CAN or CAN FD frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    /// Frame identifier.
    pub id: CanId,
    /// Frame class.
    pub class: FrameClass,
    /// Raw payload bytes.
    pub data: Vec<u8>,
    /// Whether the frame should be encoded as CAN FD.
    pub fd: bool,
    /// Additional CAN FD flags.
    pub fd_flags: FdFlags,
}

impl CanFrame {
    /// Build a validated frame.
    pub fn new(
        id: CanId,
        class: FrameClass,
        data: Vec<u8>,
        fd: bool,
        fd_flags: FdFlags,
    ) -> Result<Self> {
        let max = if fd { 64 } else { 8 };
        if data.len() > max {
            return Err(CoreError::InvalidPayloadLength {
                length: data.len(),
                max,
                kind: if fd {
                    "CAN FD frame"
                } else {
                    "classic CAN frame"
                },
            });
        }

        Ok(Self {
            id,
            class,
            data,
            fd,
            fd_flags,
        })
    }

    /// Return the payload length in bytes.
    pub fn dlc(&self) -> usize {
        self.data.len()
    }
}

/// Timestamp stored as seconds and nanoseconds since Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Seconds since Unix epoch.
    pub seconds: i64,
    /// Additional nanoseconds in the current second.
    pub nanos: u32,
}

impl Timestamp {
    /// Build a timestamp from a system clock sample.
    pub fn from_system_time(time: SystemTime) -> Result<Self> {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CoreError::TimestampBeforeEpoch)?;
        Ok(Self {
            seconds: duration.as_secs() as i64,
            nanos: duration.subsec_nanos(),
        })
    }
}

/// Interface reference kept outside the raw frame itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceRef {
    /// Friendly interface name when known.
    pub name: Option<String>,
    /// Numeric ifindex when known.
    pub index: Option<u32>,
}

/// Capture direction as observed by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Frame was received.
    Rx,
    /// Frame was transmitted.
    Tx,
}

/// Simple key-value annotation attached to a capture event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// Annotation scope or type.
    pub key: String,
    /// Human-readable value.
    pub value: String,
}

/// Portable capture envelope around a raw frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureEvent {
    /// Event timestamp.
    pub timestamp: Timestamp,
    /// Interface where the event was observed.
    pub interface: InterfaceRef,
    /// Capture direction.
    pub direction: Direction,
    /// Raw frame.
    pub frame: CanFrame,
    /// Optional annotations layered on top of the raw frame.
    pub annotations: Vec<Annotation>,
}

impl CaptureEvent {
    /// Build a basic capture event.
    pub fn new(
        timestamp: Timestamp,
        interface: InterfaceRef,
        direction: Direction,
        frame: CanFrame,
    ) -> Self {
        Self {
            timestamp,
            interface,
            direction,
            frame,
            annotations: Vec::new(),
        }
    }
}

/// Timing mode for replay paths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplayMode {
    /// Preserve event deltas exactly.
    Preserve,
    /// Emit events as quickly as possible.
    Immediate,
    /// Scale event deltas by the provided factor.
    Scale(f64),
}

/// Trait for frame sinks such as transmit sockets or test harnesses.
pub trait FrameSink {
    /// Backend-specific error type.
    type Error;

    /// Send a raw frame.
    fn send(&mut self, frame: &CanFrame) -> std::result::Result<(), Self::Error>;
}

/// Trait for frame sources such as receive sockets.
pub trait FrameSource {
    /// Backend-specific error type.
    type Error;

    /// Receive the next frame when available.
    fn recv(&mut self) -> std::result::Result<Option<CanFrame>, Self::Error>;
}

/// Trait for capture sinks such as log writers.
pub trait CaptureSink {
    /// Backend-specific error type.
    type Error;

    /// Persist a capture event.
    fn write_event(&mut self, event: &CaptureEvent) -> std::result::Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_standard_identifier_bounds() {
        assert!(CanId::standard(0x7ff).is_ok());
        assert!(CanId::standard(0x800).is_err());
    }

    #[test]
    fn validates_payload_length_for_classic_can() {
        let id = CanId::standard(0x123).expect("valid id");
        assert!(CanFrame::new(id, FrameClass::Data, vec![0; 8], false, FdFlags::default()).is_ok());
        assert!(
            CanFrame::new(id, FrameClass::Data, vec![0; 9], false, FdFlags::default()).is_err()
        );
    }

    #[test]
    fn timestamp_from_system_time_works() {
        let timestamp = Timestamp::from_system_time(UNIX_EPOCH + std::time::Duration::from_secs(2))
            .expect("valid time");
        assert_eq!(timestamp.seconds, 2);
        assert_eq!(timestamp.nanos, 0);
    }
}
