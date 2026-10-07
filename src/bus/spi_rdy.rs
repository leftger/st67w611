//! SPI transport with RDY flow control for ST67W611
//!
//! This implementation follows the X-CUBE-ST67W61 reference design,
//! using RDY interrupt signaling for proper flow control.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;

use crate::error::{Error, Result};

/// SPI header magic code
const SPI_HEADER_MAGIC: u16 = 0x55AA;

/// Maximum SPI transfer size
const MAX_SPI_XFER: usize = 2048;


/// ST67W611 SPI protocol header (8 bytes, little-endian)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SpiHeader {
    /// Magic code (0x55AA)
    magic: u16,
    /// Payload length (not including header)
    len: u16,
    /// Version (2 bits) + rx_stall (1 bit) + flags (5 bits)
    flags: u8,
    /// Message type (0=AT, 1=STA, 2=AP, 3=HCI, 4=OT)
    msg_type: u8,
    /// Reserved
    reserved: u16,
}

impl SpiHeader {
    /// Create a new header for AT command
    pub fn new_at_cmd(payload_len: u16) -> Self {
        Self {
            magic: SPI_HEADER_MAGIC,
            len: payload_len,
            flags: 0,
            msg_type: 0, // AT command
            reserved: 0,
        }
    }

    /// Convert header to bytes (little-endian)
    pub fn to_bytes(&self) -> [u8; 8] {
        [
            (self.magic & 0xFF) as u8,
            (self.magic >> 8) as u8,
            (self.len & 0xFF) as u8,
            (self.len >> 8) as u8,
            self.flags,
            self.msg_type,
            (self.reserved & 0xFF) as u8,
            (self.reserved >> 8) as u8,
        ]
    }

    /// Parse header from bytes
    pub fn from_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            magic: u16::from_le_bytes([bytes[0], bytes[1]]),
            len: u16::from_le_bytes([bytes[2], bytes[3]]),
            flags: bytes[4],
            msg_type: bytes[5],
            reserved: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }

    /// Check if header is valid
    pub fn is_valid(&self) -> bool {
        // Copy fields to avoid packed struct reference issues
        let magic = self.magic;
        let len = self.len;
        magic == SPI_HEADER_MAGIC && len <= MAX_SPI_XFER as u16
    }

    /// Get rx_stall flag (bit 2)
    pub fn rx_stall(&self) -> bool {
        let flags = self.flags; // Copy to avoid packed reference
        (flags & 0x04) != 0
    }
}

/// SPI transport with RDY flow control (interrupt-driven only)
///
/// This transport relies on an external RDY monitor task that signals
/// when RDY edges occur. No direct pin access needed.
pub struct SpiTransportRdy<SPI, CS>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    spi: SPI,
    cs: CS,
    /// Signal for RDY rising edge (transaction ready)
    txn_ready_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    /// Signal for RDY falling edge (header acknowledged)
    hdr_ack_signal: &'static Signal<CriticalSectionRawMutex, ()>,
}

impl<SPI, CS> SpiTransportRdy<SPI, CS>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    /// Create new SPI transport with RDY flow control
    ///
    /// Requires a separate RDY monitor task to signal edges
    pub fn new(
        spi: SPI,
        cs: CS,
        txn_ready_signal: &'static Signal<CriticalSectionRawMutex, ()>,
        hdr_ack_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    ) -> Self {
        Self {
            spi,
            cs,
            txn_ready_signal,
            hdr_ack_signal,
        }
    }

    /// Write AT command with proper RDY flow control
    ///
    /// This implements the X-CUBE SPI protocol:
    /// 1. Wait for RDY HIGH (if not already)
    /// 2. Assert CS HIGH
    /// 3. Send header + payload (4-byte aligned)
    /// 4. Wait for RDY falling edge (header ack)
    /// 5. Deassert CS LOW
    pub async fn write(&mut self, data: &[u8]) -> Result<usize> {
        if data.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        // Calculate padded length (must be 4-byte aligned)
        let padded_len = (data.len() + 3) & !3;
        let padding = padded_len - data.len();

        // Create header
        let header = SpiHeader::new_at_cmd(padded_len as u16);
        let header_bytes = header.to_bytes();

        // Build complete frame in aligned buffer
        let mut frame = [0u8; MAX_SPI_XFER + 8];
        frame[..8].copy_from_slice(&header_bytes);
        frame[8..8 + data.len()].copy_from_slice(data);
        // Padding with 0x88 as per spec
        for i in 0..padding {
            frame[8 + data.len() + i] = 0x88;
        }

        let total_len = 8 + padded_len;

        #[cfg(feature = "defmt")]
        defmt::debug!(
            "SPI TX: len={}, padded={}, total={}",
            data.len(),
            padded_len,
            total_len
        );

        // Wait for RDY HIGH via signal (timeout 2000ms per X-CUBE)
        // Note: If RDY is already HIGH, the signal may already be set from boot
        // The monitor task signals on EVERY rising edge
        let _ = with_timeout(Duration::from_millis(100), self.txn_ready_signal.wait()).await;
        // Don't error on timeout - RDY might already be HIGH from boot

        // Assert CS (HIGH = active)
        self.cs.set_high().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        // Transmit frame (header + data) - always async for now
        // TODO: Add blocking path for small transfers if performance critical
        self.spi
            .write(&frame[..total_len])
            .await
            .map_err(|_| Error::Spi)?;

        // Wait for header acknowledgment (RDY falling edge, timeout 100ms)
        let _ = with_timeout(Duration::from_millis(100), self.hdr_ack_signal.wait()).await;

        // Deassert CS (LOW = inactive)
        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok(data.len())
    }

    /// Read from SPI with RDY flow control
    ///
    /// Returns number of payload bytes read (excluding header)
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if buffer.len() > MAX_SPI_XFER {
            return Err(Error::BufferTooSmall);
        }

        // Wait for RDY HIGH via signal (timeout 2000ms)
        with_timeout(Duration::from_millis(2000), self.txn_ready_signal.wait())
            .await
            .map_err(|_| Error::Timeout)?;

        // Assert CS (HIGH = active)
        self.cs.set_high().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        // Read header first (8 bytes) - use async read
        let mut header_bytes = [0u8; 8];
        self.spi
            .read(&mut header_bytes)
            .await
            .map_err(|_| Error::Spi)?;

        // Parse and validate header
        let header = SpiHeader::from_bytes(&header_bytes);

        // Copy fields to avoid packed struct reference
        let _magic = header.magic;
        let payload_len = header.len as usize;

        #[cfg(feature = "defmt")]
        defmt::trace!("SPI RX: magic={:04x}, len={}", _magic, payload_len);

        if !header.is_valid() {
            self.cs.set_low().map_err(|_| Error::Spi)?;
            #[cfg(feature = "defmt")]
            defmt::warn!("Invalid SPI header magic: {:04x}", _magic);
            return Err(Error::InvalidResponse);
        }

        // If no payload, done
        if payload_len == 0 {
            // Wait for header ack
            let _ = with_timeout(Duration::from_millis(100), self.hdr_ack_signal.wait()).await;
            self.cs.set_low().map_err(|_| Error::Spi)?;
            return Ok(0);
        }

        // Check buffer size
        if payload_len > buffer.len() {
            self.cs.set_low().map_err(|_| Error::Spi)?;
            return Err(Error::BufferTooSmall);
        }

        // Read payload
        self.spi
            .read(&mut buffer[..payload_len])
            .await
            .map_err(|_| Error::Spi)?;

        // Wait for header acknowledgment
        let _ = with_timeout(Duration::from_millis(100), self.hdr_ack_signal.wait()).await;

        // Deassert CS
        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok(payload_len)
    }
}
