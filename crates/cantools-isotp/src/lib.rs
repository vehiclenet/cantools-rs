//! Portable ISO-TP segmentation and reassembly helpers.

use cantools_core::CanId;
use thiserror::Error;

/// ISO-TP addressing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressingMode {
    /// Normal 11-bit or 29-bit addressing.
    Normal,
    /// Extended addressing with a leading address byte.
    Extended(u8),
}

/// ISO-TP address pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    /// Request identifier.
    pub tx_id: CanId,
    /// Response identifier.
    pub rx_id: CanId,
    /// Addressing mode.
    pub mode: AddressingMode,
}

/// Flow-control status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStatus {
    /// Receiver is ready for more data.
    ContinueToSend,
    /// Receiver needs more time.
    Wait,
    /// Receiver aborted the transfer.
    Overflow,
}

/// ISO-TP protocol errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IsotpError {
    /// Single-frame payload exceeds the available capacity.
    #[error("payload exceeds single-frame capacity")]
    SingleFrameTooLarge,
    /// A consecutive frame was received before a first frame.
    #[error("consecutive frame arrived without an active transfer")]
    UnexpectedConsecutiveFrame,
    /// Sequence number mismatch.
    #[error("expected sequence number {expected}, received {actual}")]
    SequenceNumberMismatch {
        /// Expected sequence number.
        expected: u8,
        /// Actual sequence number received.
        actual: u8,
    },
    /// Flow control cannot represent the requested block size or separation time.
    #[error("invalid flow control parameters")]
    InvalidFlowControl,
}

/// Result type for ISO-TP helpers.
pub type Result<T> = std::result::Result<T, IsotpError>;

/// Encoded protocol control information and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolDataUnit {
    /// Single-frame transfer.
    SingleFrame(Vec<u8>),
    /// First frame of a multi-frame transfer.
    FirstFrame {
        /// Total transfer length.
        total_len: usize,
        /// First-frame payload bytes.
        payload: Vec<u8>,
    },
    /// Consecutive frame.
    ConsecutiveFrame {
        /// Consecutive-frame sequence number.
        sequence_number: u8,
        /// Consecutive-frame payload bytes.
        payload: Vec<u8>,
    },
    /// Flow-control frame.
    FlowControl {
        /// Receiver status.
        status: FlowStatus,
        /// Requested block size.
        block_size: u8,
        /// Minimum separation time in milliseconds.
        separation_time_ms: u8,
    },
}

impl ProtocolDataUnit {
    /// Encode a PDU into a raw CAN payload using the provided frame size.
    pub fn encode(&self, frame_size: usize) -> Result<Vec<u8>> {
        match self {
            Self::SingleFrame(payload) => {
                if payload.len() > frame_size.saturating_sub(1) {
                    return Err(IsotpError::SingleFrameTooLarge);
                }

                let mut out = Vec::with_capacity(frame_size);
                out.push(payload.len() as u8);
                out.extend_from_slice(payload);
                Ok(out)
            }
            Self::FirstFrame { total_len, payload } => {
                let mut out = Vec::with_capacity(frame_size);
                out.push(0x10 | (((total_len >> 8) & 0x0f) as u8));
                out.push((total_len & 0xff) as u8);
                out.extend_from_slice(payload);
                Ok(out)
            }
            Self::ConsecutiveFrame {
                sequence_number,
                payload,
            } => {
                let mut out = Vec::with_capacity(frame_size);
                out.push(0x20 | (sequence_number & 0x0f));
                out.extend_from_slice(payload);
                Ok(out)
            }
            Self::FlowControl {
                status,
                block_size,
                separation_time_ms,
            } => {
                let status_nibble = match status {
                    FlowStatus::ContinueToSend => 0x00,
                    FlowStatus::Wait => 0x01,
                    FlowStatus::Overflow => 0x02,
                };
                Ok(vec![0x30 | status_nibble, *block_size, *separation_time_ms])
            }
        }
    }
}

/// Split a payload into ISO-TP PDUs for a given frame payload capacity.
pub fn segment(payload: &[u8], frame_payload_capacity: usize) -> Result<Vec<ProtocolDataUnit>> {
    if payload.len() <= frame_payload_capacity.saturating_sub(1) {
        return Ok(vec![ProtocolDataUnit::SingleFrame(payload.to_vec())]);
    }

    let first_capacity = frame_payload_capacity.saturating_sub(2);
    let consecutive_capacity = frame_payload_capacity.saturating_sub(1);
    let mut pdus = Vec::new();

    let first_end = payload.len().min(first_capacity);
    pdus.push(ProtocolDataUnit::FirstFrame {
        total_len: payload.len(),
        payload: payload[..first_end].to_vec(),
    });
    let mut offset = first_end;

    let mut seq = 1;
    while offset < payload.len() {
        let end = (offset + consecutive_capacity).min(payload.len());
        pdus.push(ProtocolDataUnit::ConsecutiveFrame {
            sequence_number: seq,
            payload: payload[offset..end].to_vec(),
        });
        offset = end;
        seq = (seq + 1) & 0x0f;
    }

    Ok(pdus)
}

/// Reassembles ISO-TP PDUs into full payloads.
#[derive(Debug, Default)]
pub struct Reassembler {
    total_len: Option<usize>,
    next_seq: u8,
    buffer: Vec<u8>,
}

impl Reassembler {
    /// Ingest one PDU. Returns a completed payload when the transfer finishes.
    pub fn ingest(&mut self, pdu: &ProtocolDataUnit) -> Result<Option<Vec<u8>>> {
        match pdu {
            ProtocolDataUnit::SingleFrame(payload) => Ok(Some(payload.clone())),
            ProtocolDataUnit::FirstFrame { total_len, payload } => {
                self.total_len = Some(*total_len);
                self.next_seq = 1;
                self.buffer.clear();
                self.buffer.extend_from_slice(payload);
                Ok(self.take_if_complete())
            }
            ProtocolDataUnit::ConsecutiveFrame {
                sequence_number,
                payload,
            } => {
                if self.total_len.is_none() {
                    return Err(IsotpError::UnexpectedConsecutiveFrame);
                }
                if *sequence_number != self.next_seq {
                    return Err(IsotpError::SequenceNumberMismatch {
                        expected: self.next_seq,
                        actual: *sequence_number,
                    });
                }

                self.buffer.extend_from_slice(payload);
                self.next_seq = (self.next_seq + 1) & 0x0f;
                Ok(self.take_if_complete())
            }
            ProtocolDataUnit::FlowControl { .. } => Ok(None),
        }
    }

    fn take_if_complete(&mut self) -> Option<Vec<u8>> {
        let total = self.total_len?;
        if self.buffer.len() < total {
            return None;
        }

        let mut payload = self.buffer.split_off(total);
        std::mem::swap(&mut payload, &mut self.buffer);
        self.total_len = None;
        Some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_frame_round_trip() {
        let data = vec![1, 2, 3];
        let frames = segment(&data, 8).expect("segments");
        assert_eq!(frames, vec![ProtocolDataUnit::SingleFrame(data)]);
    }

    #[test]
    fn multi_frame_reassembly() {
        let payload: Vec<u8> = (0..20).collect();
        let frames = segment(&payload, 8).expect("segments");
        assert_eq!(frames.len(), 3);

        let mut reassembler = Reassembler::default();
        let mut completed = None;
        for frame in &frames {
            completed = reassembler.ingest(frame).expect("valid pdu");
        }

        assert_eq!(completed.expect("payload"), payload);
    }
}
