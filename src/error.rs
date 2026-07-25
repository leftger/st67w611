//! Error types for the ST67W611 driver

use core::fmt;

/// Driver error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// SPI communication error
    Spi,

    /// AT command timeout
    Timeout,

    /// AT command failed (received ERROR response)
    AtCommandFailed,

    /// Invalid AT response format
    InvalidResponse,

    /// Buffer too small for operation
    BufferTooSmall,

    /// Invalid parameter provided
    InvalidParameter,

    /// Operation not supported
    NotSupported,

    /// WiFi not connected
    NotConnected,

    /// WiFi connection failed
    ConnectionFailed,

    /// Network operation failed
    NetworkError,

    /// Socket operation failed
    SocketError,

    /// Socket not available (pool exhausted)
    NoSocketAvailable,

    /// Invalid socket ID
    InvalidSocket,

    /// Socket already in use
    SocketInUse,

    /// TLS/SSL error
    TlsError,

    /// Certificate error
    CertificateError,

    /// MQTT operation failed
    MqttError,

    /// HTTP operation failed
    HttpError,

    /// Response slot pool exhausted
    NoResponseSlot,

    /// Parse error
    ParseError,

    /// Invalid state for operation
    InvalidState,

    /// Resource busy
    Busy,

    /// Operation would block
    WouldBlock,

    /// Unknown error
    Unknown,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Spi => write!(f, "SPI communication error"),
            Error::Timeout => write!(f, "Operation timeout"),
            Error::AtCommandFailed => write!(f, "AT command failed"),
            Error::InvalidResponse => write!(f, "Invalid AT response"),
            Error::BufferTooSmall => write!(f, "Buffer too small"),
            Error::InvalidParameter => write!(f, "Invalid parameter"),
            Error::NotSupported => write!(f, "Operation not supported"),
            Error::NotConnected => write!(f, "WiFi not connected"),
            Error::ConnectionFailed => write!(f, "Connection failed"),
            Error::NetworkError => write!(f, "Network error"),
            Error::SocketError => write!(f, "Socket error"),
            Error::NoSocketAvailable => write!(f, "No socket available"),
            Error::InvalidSocket => write!(f, "Invalid socket ID"),
            Error::SocketInUse => write!(f, "Socket already in use"),
            Error::TlsError => write!(f, "TLS/SSL error"),
            Error::CertificateError => write!(f, "Certificate error"),
            Error::MqttError => write!(f, "MQTT error"),
            Error::HttpError => write!(f, "HTTP error"),
            Error::NoResponseSlot => write!(f, "No response slot available"),
            Error::ParseError => write!(f, "Parse error"),
            Error::InvalidState => write!(f, "Invalid state"),
            Error::Busy => write!(f, "Resource busy"),
            Error::WouldBlock => write!(f, "Operation would block"),
            Error::Unknown => write!(f, "Unknown error"),
        }
    }
}

/// Result type alias using driver Error
pub type Result<T> = core::result::Result<T, Error>;

/// Convert from embedded-hal SPI errors
impl<E> From<E> for Error
where
    E: embedded_hal::spi::Error,
{
    fn from(_: E) -> Self {
        Error::Spi
    }
}

// Note: We don't implement From<embassy_time::TimeoutError> directly
// to avoid potential conflicts with the generic From<E> impl above.
