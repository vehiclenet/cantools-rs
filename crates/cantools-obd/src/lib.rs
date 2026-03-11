//! OBD helpers layered on UDS-style CAN diagnostics.

mod generated;

pub use generated::{GENERATED_PIDS, GeneratedPid};

use thiserror::Error;

/// OBD modes used in the bootstrap implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// Current data.
    CurrentData = 0x01,
    /// Vehicle information.
    VehicleInformation = 0x09,
}

/// Common Mode 01 PIDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CurrentDataPid {
    /// Calculated engine load.
    CalculatedEngineLoad = 0x04,
    /// Engine coolant temperature.
    EngineCoolantTemperature = 0x05,
    /// Engine speed.
    EngineRpm = 0x0c,
    /// Vehicle speed.
    VehicleSpeed = 0x0d,
}

/// Common Mode 09 PIDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VehicleInformationPid {
    /// VIN.
    Vin = 0x02,
}

/// Return the generated OBD table entry for a mode/PID pair.
pub fn generated_pid(mode: u8, pid: u8) -> Option<&'static GeneratedPid> {
    GENERATED_PIDS
        .iter()
        .find(|entry| entry.mode == mode && entry.pid == pid)
}

/// Decoded OBD values from the bootstrap PID table.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Percent value.
    Percent(f64),
    /// Temperature in Celsius.
    Celsius(i16),
    /// Revolutions per minute.
    RevolutionsPerMinute(f64),
    /// Vehicle speed in km/h.
    KilometersPerHour(u8),
    /// Vehicle identification number.
    Text(String),
}

/// OBD decode errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ObdError {
    /// Response did not contain enough bytes.
    #[error("payload too short")]
    PayloadTooShort,
    /// PID is not part of the bootstrap implementation.
    #[error("unsupported pid {0:#x}")]
    UnsupportedPid(u8),
}

/// Decode a Mode 01 response payload into a typed value.
pub fn decode_current_data(pid: CurrentDataPid, payload: &[u8]) -> Result<Value, ObdError> {
    match pid {
        CurrentDataPid::CalculatedEngineLoad => payload
            .first()
            .map(|byte| Value::Percent((*byte as f64) * 100.0 / 255.0))
            .ok_or(ObdError::PayloadTooShort),
        CurrentDataPid::EngineCoolantTemperature => payload
            .first()
            .map(|byte| Value::Celsius((*byte as i16) - 40))
            .ok_or(ObdError::PayloadTooShort),
        CurrentDataPid::EngineRpm => {
            if payload.len() < 2 {
                return Err(ObdError::PayloadTooShort);
            }
            let raw = u16::from(payload[0]) << 8 | u16::from(payload[1]);
            Ok(Value::RevolutionsPerMinute(raw as f64 / 4.0))
        }
        CurrentDataPid::VehicleSpeed => payload
            .first()
            .copied()
            .map(Value::KilometersPerHour)
            .ok_or(ObdError::PayloadTooShort),
    }
}

/// Decode a Mode 09 response payload into a typed value.
pub fn decode_vehicle_information(
    pid: VehicleInformationPid,
    payload: &[u8],
) -> Result<Value, ObdError> {
    match pid {
        VehicleInformationPid::Vin => {
            if payload.is_empty() {
                return Err(ObdError::PayloadTooShort);
            }
            Ok(Value::Text(
                String::from_utf8_lossy(payload).trim().to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_engine_rpm() {
        let value = decode_current_data(CurrentDataPid::EngineRpm, &[0x1f, 0xa0]).expect("rpm");
        assert_eq!(value, Value::RevolutionsPerMinute(2024.0));
    }

    #[test]
    fn decodes_vin_text() {
        let value = decode_vehicle_information(VehicleInformationPid::Vin, b"1M8GDM9AXKP042788")
            .expect("vin");
        assert_eq!(value, Value::Text("1M8GDM9AXKP042788".to_string()));
    }
}
