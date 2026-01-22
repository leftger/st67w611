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

impl CertificateType {
    /// Get the filename for this certificate type
    fn filename(&self) -> &'static str {
        match self {
            CertificateType::Ca => "ca_cert.pem",
            CertificateType::Client => "client_cert.pem",
            CertificateType::ClientKey => "client_key.pem",
        }
    }
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

    /// Upload a certificate to the module's filesystem
    ///
    /// Certificates are stored in the module's flash memory and can be used for
    /// TLS connections. Maximum certificate size depends on module's available flash.
    ///
    /// Note: This uploads in chunks to avoid AT command length limits.
    pub async fn upload_certificate<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        cert_type: CertificateType,
        cert_data: &[u8],
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let filename = cert_type.filename();
        const CHUNK_SIZE: usize = 512; // Write in 512-byte chunks

        // Delete existing file first
        let _ = self.delete_certificate(spi, cert_type).await;

        // Write certificate data in chunks
        let mut offset = 0;
        while offset < cert_data.len() {
            let end = core::cmp::min(offset + CHUNK_SIZE, cert_data.len());
            let chunk = &cert_data[offset..end];

            // Send write command
            let cmd = crate::at::command::filesystem::fs_write(filename, offset, chunk.len())?;
            let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

            // Wait for ready prompt ">"
            if response != crate::at::AtResponse::ReadyPrompt {
                return Err(Error::CertificateError);
            }

            // Send actual data
            {
                let mut spi_guard = spi.lock().await;
                spi_guard.write(chunk).await?;
            }

            // Wait for OK
            // In a proper implementation, we'd wait for the OK response
            // For now, we continue to next chunk
            embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;

            offset = end;
        }

        Ok(())
    }

    /// Delete a certificate from the module's filesystem
    pub async fn delete_certificate<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        cert_type: CertificateType,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let filename = cert_type.filename();

        let cmd = crate::at::command::filesystem::fs_delete(filename)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        // OK or ERROR both acceptable (file may not exist)
        match response {
            crate::at::AtResponse::Ok | crate::at::AtResponse::Error => Ok(()),
            _ => Err(Error::CertificateError),
        }
    }

    /// Read a certificate from the module's filesystem
    pub async fn read_certificate<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        cert_type: CertificateType,
        buffer: &mut [u8],
    ) -> Result<usize>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let filename = cert_type.filename();

        let cmd = crate::at::command::filesystem::fs_read(filename, 0, buffer.len())?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        // The response should contain the file data
        // Format varies - this is a simplified implementation
        if let crate::at::AtResponse::Data { prefix: _, content } = response {
            let bytes_to_copy = core::cmp::min(content.len(), buffer.len());
            buffer[..bytes_to_copy].copy_from_slice(content.as_bytes());
            return Ok(bytes_to_copy);
        }

        Err(Error::CertificateError)
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
