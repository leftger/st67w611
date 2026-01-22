//! TLS/SSL certificate management

use crate::at::processor::AtProcessor;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;
use embassy_time::Duration;

/// Certificate type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CertificateType {
    /// CA certificate
    Ca,
    /// Client certificate
    Client,
    /// Client private key
    ClientKey,
}

/// TLS manager for certificate operations
pub struct TlsManager {
    /// AT processor
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl TlsManager {
    /// Create a new TLS manager
    pub const fn new(processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self { processor, timeout }
    }

    /// Upload a certificate to the module
    pub async fn upload_certificate<SPI, CS>(
        &self,
        _spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        _cert_type: CertificateType,
        _cert_data: &[u8],
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // TODO: Implement certificate upload
        // This would use AT commands like AT+SYSFLASH to write certificates
        Err(Error::NotSupported)
    }

    /// Delete a certificate from the module
    pub async fn delete_certificate<SPI, CS>(
        &self,
        _spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        _cert_type: CertificateType,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // TODO: Implement certificate deletion
        Err(Error::NotSupported)
    }

    /// Configure SSL for a socket connection
    pub async fn configure_socket_ssl<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        link_id: u8,
        auth_mode: u8,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = crate::at::command::network::configure_ssl(link_id, auth_mode)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::TlsError);
        }

        Ok(())
    }

    /// Set SNI hostname for a socket
    pub async fn set_sni<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        link_id: u8,
        hostname: &str,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = crate::at::command::network::set_sni(link_id, hostname)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::TlsError);
        }

        Ok(())
    }
}
