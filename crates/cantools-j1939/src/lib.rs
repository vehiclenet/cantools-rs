//! Portable J1939 modeling and helpers.

mod generated;

pub use generated::{GENERATED_PGNS, GeneratedPgn};

use cantools_core::CanId;
use thiserror::Error;

/// J1939 parsing errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum J1939Error {
    /// Only 29-bit identifiers are valid J1939 identifiers.
    #[error("J1939 requires an extended 29-bit CAN identifier")]
    RequiresExtendedId,
}

/// J1939 parameter group number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pgn(pub u32);

/// J1939 suspect parameter number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Spn(pub u32);

/// J1939 NAME value used during address claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(pub u64);

/// Decoded J1939 identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct J1939Id {
    /// Message priority.
    pub priority: u8,
    /// PGN.
    pub pgn: Pgn,
    /// Source address.
    pub source_address: u8,
    /// Destination address when using PDU1.
    pub destination_address: Option<u8>,
}

impl J1939Id {
    /// Decode a 29-bit J1939 identifier.
    pub fn from_can_id(id: CanId) -> Result<Self, J1939Error> {
        if !id.is_extended() {
            return Err(J1939Error::RequiresExtendedId);
        }

        let raw = id.raw();
        let priority = ((raw >> 26) & 0x07) as u8;
        let pf = ((raw >> 16) & 0xff) as u8;
        let ps = ((raw >> 8) & 0xff) as u8;
        let source_address = (raw & 0xff) as u8;
        let destination_address = if pf < 240 { Some(ps) } else { None };
        let pgn = if pf < 240 {
            (raw >> 8) & 0x03ff00
        } else {
            (raw >> 8) & 0x03ffff
        };

        Ok(Self {
            priority,
            pgn: Pgn(pgn),
            source_address,
            destination_address,
        })
    }
}

/// Outcome of an address-claim comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressClaimOutcome {
    /// We keep the address.
    KeepAddress,
    /// We lost arbitration and must choose another address.
    LoseAddress,
}

/// Resolve an address-claim conflict by comparing NAME values.
pub fn resolve_address_claim(local: Name, challenger: Name) -> AddressClaimOutcome {
    if local <= challenger {
        AddressClaimOutcome::KeepAddress
    } else {
        AddressClaimOutcome::LoseAddress
    }
}

/// Look up a generated PGN definition by number.
pub fn generated_pgn(pgn: Pgn) -> Option<&'static GeneratedPgn> {
    GENERATED_PGNS.iter().find(|entry| entry.pgn == pgn.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pdu2_identifier() {
        let id = CanId::extended(0x18fef100).expect("valid id");
        let decoded = J1939Id::from_can_id(id).expect("j1939");
        assert_eq!(decoded.priority, 6);
        assert_eq!(decoded.pgn, Pgn(0xfef1));
        assert_eq!(decoded.destination_address, None);
        assert_eq!(decoded.source_address, 0x00);
    }

    #[test]
    fn lower_name_wins_address_claim() {
        assert_eq!(
            resolve_address_claim(Name(1), Name(2)),
            AddressClaimOutcome::KeepAddress
        );
        assert_eq!(
            resolve_address_claim(Name(3), Name(2)),
            AddressClaimOutcome::LoseAddress
        );
    }
}
