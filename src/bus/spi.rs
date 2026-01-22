//! SPI transport layer

use embedded_hal_async::spi::SpiDevice;
use embedded_hal::digital::OutputPin;
use embassy_time::{Timer, Duration};

use crate::error::{Error, Result};
use crate::types::MAX_SPI_XFER;

/// SPI transport wrapper
pub struct SpiTransport<SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    spi: SPI,
    cs: CS,
}

impl<SPI, CS> SpiTransport<SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    /// Create a new SPI transport
    pub fn new(spi: SPI, cs: CS) -> Self {
        Self { spi, cs }
    }

    /// Write data to SPI
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.cs.set_low().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        let result = self.spi.write(data).await.map_err(|_| Error::Spi);

        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_high().map_err(|_| Error::Spi)?;

        result
    }

    /// Read data from SPI
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if buffer.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.cs.set_low().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        let result = self.spi.read(buffer).await.map_err(|_| Error::Spi);

        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_high().map_err(|_| Error::Spi)?;

        result?;

        // Find actual data length (may be less than buffer size)
        Ok(buffer.len())
    }

    /// Transfer data (write and read simultaneously)
    pub async fn transfer(&mut self, tx_buffer: &[u8], rx_buffer: &mut [u8]) -> Result<()> {
        if tx_buffer.len() > MAX_SPI_XFER || rx_buffer.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.cs.set_low().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        let result = self.spi.transfer(rx_buffer, tx_buffer).await.map_err(|_| Error::Spi);

        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_high().map_err(|_| Error::Spi)?;

        result
    }

    /// Check if data is available for reading
    /// This is a placeholder - actual implementation depends on hardware signaling
    pub async fn data_available(&mut self) -> Result<bool> {
        // In a real implementation, this would check a GPIO line (e.g., READY pin)
        // or perform a status read to determine if data is available
        Ok(true)
    }

    /// Flush any pending data
    pub async fn flush(&mut self) -> Result<()> {
        // Flush implementation if needed
        Ok(())
    }
}

/// Alternative SPI transport that doesn't manage CS pin
/// (useful when SPI device handles CS automatically)
pub struct SpiTransportAuto<SPI>
where
    SPI: SpiDevice,
{
    spi: SPI,
}

impl<SPI> SpiTransportAuto<SPI>
where
    SPI: SpiDevice,
{
    /// Create a new auto-CS SPI transport
    pub fn new(spi: SPI) -> Self {
        Self { spi }
    }

    /// Write data to SPI
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.spi.write(data).await.map_err(|_| Error::Spi)
    }

    /// Read data from SPI
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if buffer.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.spi.read(buffer).await.map_err(|_| Error::Spi)?;
        Ok(buffer.len())
    }

    /// Transfer data
    pub async fn transfer(&mut self, tx_buffer: &[u8], rx_buffer: &mut [u8]) -> Result<()> {
        if tx_buffer.len() > MAX_SPI_XFER || rx_buffer.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        self.spi.transfer(rx_buffer, tx_buffer).await.map_err(|_| Error::Spi)
    }

    /// Check if data is available
    pub async fn data_available(&mut self) -> Result<bool> {
        Ok(true)
    }

    /// Flush any pending data
    pub async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
