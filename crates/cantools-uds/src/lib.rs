//! Portable UDS request and response codecs.

mod generated;

pub use generated::{
    GENERATED_NEGATIVE_RESPONSES, GENERATED_SERVICES, GeneratedNegativeResponse, GeneratedService,
};

use thiserror::Error;

/// Common UDS service identifiers used by the bootstrap implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ServiceId {
    /// Diagnostic session control.
    DiagnosticSessionControl = 0x10,
    /// ECU reset.
    EcuReset = 0x11,
    /// Read data by identifier.
    ReadDataByIdentifier = 0x22,
    /// Tester present.
    TesterPresent = 0x3e,
}

impl ServiceId {
    /// Return the generated service definition for this service identifier.
    pub fn generated(self) -> Option<&'static GeneratedService> {
        GENERATED_SERVICES
            .iter()
            .find(|service| service.sid == self as u8)
    }
}

/// Common negative response codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NegativeResponseCode {
    /// Request is not supported.
    ServiceNotSupported = 0x11,
    /// Conditions not correct.
    ConditionsNotCorrect = 0x22,
    /// Sub-function not supported.
    SubFunctionNotSupported = 0x12,
}

impl NegativeResponseCode {
    /// Return the generated table entry for this response code.
    pub fn generated(self) -> Option<&'static GeneratedNegativeResponse> {
        GENERATED_NEGATIVE_RESPONSES
            .iter()
            .find(|response| response.code == self as u8)
    }
}

/// Request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Service identifier.
    pub service: ServiceId,
    /// Service payload bytes.
    pub data: Vec<u8>,
}

impl Request {
    /// Encode the request into a payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.data.len());
        out.push(self.service as u8);
        out.extend_from_slice(&self.data);
        out
    }
}

/// Positive response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveResponse {
    /// Original service identifier.
    pub service: ServiceId,
    /// Response payload bytes.
    pub data: Vec<u8>,
}

/// Negative response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegativeResponse {
    /// Original service identifier.
    pub service: ServiceId,
    /// Negative response code.
    pub code: NegativeResponseCode,
}

/// Response envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Positive response.
    Positive(PositiveResponse),
    /// Negative response.
    Negative(NegativeResponse),
}

/// UDS codec errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum UdsError {
    /// The message was too short to decode.
    #[error("payload too short")]
    PayloadTooShort,
    /// The service identifier is not part of the bootstrap set.
    #[error("unknown service identifier {0:#x}")]
    UnknownService(u8),
    /// The negative response code is not part of the bootstrap set.
    #[error("unknown negative response code {0:#x}")]
    UnknownNegativeResponseCode(u8),
}

fn decode_service(raw: u8) -> Result<ServiceId, UdsError> {
    match raw {
        0x10 => Ok(ServiceId::DiagnosticSessionControl),
        0x11 => Ok(ServiceId::EcuReset),
        0x22 => Ok(ServiceId::ReadDataByIdentifier),
        0x3e => Ok(ServiceId::TesterPresent),
        other => Err(UdsError::UnknownService(other)),
    }
}

fn decode_negative_response_code(raw: u8) -> Result<NegativeResponseCode, UdsError> {
    match raw {
        0x11 => Ok(NegativeResponseCode::ServiceNotSupported),
        0x12 => Ok(NegativeResponseCode::SubFunctionNotSupported),
        0x22 => Ok(NegativeResponseCode::ConditionsNotCorrect),
        other => Err(UdsError::UnknownNegativeResponseCode(other)),
    }
}

/// Decode a raw UDS response payload.
pub fn decode_response(payload: &[u8]) -> Result<Response, UdsError> {
    if payload.is_empty() {
        return Err(UdsError::PayloadTooShort);
    }

    if payload[0] == 0x7f {
        if payload.len() < 3 {
            return Err(UdsError::PayloadTooShort);
        }

        return Ok(Response::Negative(NegativeResponse {
            service: decode_service(payload[1])?,
            code: decode_negative_response_code(payload[2])?,
        }));
    }

    let response_sid = payload[0];
    let service = decode_service(response_sid.saturating_sub(0x40))?;
    Ok(Response::Positive(PositiveResponse {
        service,
        data: payload[1..].to_vec(),
    }))
}

/// Minimal transport abstraction used by the bootstrap client.
pub trait Transport {
    /// Transport error type.
    type Error;

    /// Send a request and wait for the corresponding response bytes.
    fn call(&mut self, request: &[u8]) -> std::result::Result<Vec<u8>, Self::Error>;
}

/// Client-first UDS helper.
pub struct Client<T> {
    transport: T,
}

impl<T> Client<T> {
    /// Construct a client from a transport.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T, E> Client<T>
where
    T: Transport<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    /// Execute a request and decode the resulting UDS response.
    pub fn send(
        &mut self,
        request: &Request,
    ) -> std::result::Result<Response, Box<dyn std::error::Error + Send + Sync>> {
        let payload = self.transport.call(&request.encode())?;
        Ok(decode_response(&payload)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_positive_response() {
        let response = decode_response(&[0x62, 0xf1, 0x90]).expect("decode");
        assert_eq!(
            response,
            Response::Positive(PositiveResponse {
                service: ServiceId::ReadDataByIdentifier,
                data: vec![0xf1, 0x90],
            })
        );
    }

    #[test]
    fn decodes_negative_response() {
        let response = decode_response(&[0x7f, 0x22, 0x11]).expect("decode");
        assert_eq!(
            response,
            Response::Negative(NegativeResponse {
                service: ServiceId::ReadDataByIdentifier,
                code: NegativeResponseCode::ServiceNotSupported,
            })
        );
    }
}
