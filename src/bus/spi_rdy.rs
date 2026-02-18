//! SPI transport with RDY flow control for ST67W611
//!
//! Matches the X-CUBE-ST67W61 reference HAL (`spi_iface.c`):
//!
//! ## Protocol summary
//!
//! Every CS assertion is a **full-duplex** exchange — the master sends its header + payload
//! **simultaneously** with the slave's header + payload.
//!
//! ### Master-initiated write (`TXN_PENDING` in reference)
//! 1. Assert CS HIGH
//! 2. Wait for RDY HIGH — slave detects CS and responds with TXN_READY
//! 3. Full-duplex `transfer_in_place`: TX `[header | data]`, RX `[slave_header | slave_data]`
//! 4. If slave's `len > first_xfer_payload_bytes`: read remaining slave bytes (second part)
//! 5. Cache slave payload in `rx_cache` for the next `read()` call
//! 6. Wait for HDR_ACK (RDY falling edge, 100 ms) — per `SPI_WAIT_HDR_ACK_TIMEOUT_MS`
//! 7. Deassert CS
//!
//! ### Slave-initiated read (`TXN_RDY` / `SKIP_FIRST_TXN_WAIT` in reference)
//! 1. Wait for RDY HIGH (TXN_READY signal already set)
//! 2. Assert CS HIGH
//! 3. Full-duplex `transfer_in_place`: TX empty header (`len=0`), RX slave header
//! 4. Read slave payload (second part) if `slave.len > 0`
//! 5. Wait for HDR_ACK (100 ms)
//! 6. Deassert CS

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;
use heapless::Vec;

use crate::error::{Error, Result};

/// SPI header magic code (0x55AA in little-endian → bytes [AA, 55])
const SPI_HEADER_MAGIC: u16 = 0x55AA;

/// Maximum SPI payload size — matches `SPI_XFER_MTU_BYTES` in the reference
const MAX_SPI_PAYLOAD: usize = 1520;

/// Maximum total frame = header (8) + max payload (4-byte padded)
const MAX_FRAME: usize = 8 + ((MAX_SPI_PAYLOAD + 3) & !3);

/// ST67W611 SPI protocol header (8 bytes, little-endian packed)
///
/// ```text
/// Offset  Size  Field
///      0     2  magic    = 0x55AA
///      2     2  len      = actual payload bytes (NOT padded)
///      4     1  flags    = version[1:0] | rx_stall[2] | reserved[7:3]
///      5     1  type     = 0=AT, 1=STA, 2=AP, 3=HCI, 4=OT
///      6     2  reserved = 0
/// ```
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SpiHeader {
    magic: u16,
    len: u16,
    flags: u8,
    msg_type: u8,
    reserved: u16,
}

impl SpiHeader {
    /// AT-command header with `actual_len` payload bytes
    pub fn new_at_cmd(actual_len: u16) -> Self {
        Self {
            magic: SPI_HEADER_MAGIC,
            len: actual_len,
            flags: 0,
            msg_type: 0,
            reserved: 0,
        }
    }

    /// Serialise to 8 bytes (little-endian)
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

    /// Deserialise from a byte slice (must be ≥ 8 bytes)
    pub fn from_bytes(b: &[u8]) -> Self {
        Self {
            magic: u16::from_le_bytes([b[0], b[1]]),
            len: u16::from_le_bytes([b[2], b[3]]),
            flags: b[4],
            msg_type: b[5],
            reserved: u16::from_le_bytes([b[6], b[7]]),
        }
    }

    /// Returns `true` if the magic code is correct and `len` is within bounds.
    pub fn is_valid(&self) -> bool {
        let magic = self.magic;
        let len = self.len;
        magic == SPI_HEADER_MAGIC && len <= MAX_SPI_PAYLOAD as u16
    }

    /// Returns `true` if the slave indicated its RX path is stalled.
    pub fn rx_stall(&self) -> bool {
        (self.flags & 0x04) != 0
    }
}

/// SPI transport with RDY flow control (interrupt-driven)
///
/// Requires a separate task that watches WIFI_RDY edges and calls:
/// - `txn_ready_signal.signal(())` on **rising** edge (slave ready to transact)
/// - `hdr_ack_signal.signal(())`   on **falling** edge (slave acknowledged header)
pub struct SpiTransportRdy<SPI, CS>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    spi: SPI,
    cs: CS,
    txn_ready_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    hdr_ack_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    /// Slave payload captured during the most recent `write()` full-duplex exchange
    rx_cache: Vec<u8, MAX_SPI_PAYLOAD>,
}

impl<SPI, CS> SpiTransportRdy<SPI, CS>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    /// Create a new transport. Requires a separate RDY-monitor task to signal edges.
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
            rx_cache: Vec::new(),
        }
    }

    /// Send an AT-command frame and capture any slave response received in the same exchange.
    ///
    /// The captured response is stored in `rx_cache` and returned by the next `read()` call.
    pub async fn write(&mut self, data: &[u8]) -> Result<usize> {
        let payload_len = data.len();
        if payload_len > MAX_SPI_PAYLOAD {
            return Err(Error::BufferTooSmall);
        }

        // Transfer size must be 4-byte aligned (reference: `(txbuf->len + 3) & ~3`)
        let padded_len = (payload_len + 3) & !3;
        let total_len = 8 + padded_len;

        // Build TX frame: header (len = actual bytes) + payload + zero padding
        let mut frame = [0u8; MAX_FRAME];
        frame[..8].copy_from_slice(&SpiHeader::new_at_cmd(payload_len as u16).to_bytes());
        frame[8..8 + payload_len].copy_from_slice(data);
        // bytes [8+payload_len .. total_len] remain 0x00 (padding)

        #[cfg(feature = "defmt")]
        defmt::debug!("SPI TX: {} bytes (xfer {})", payload_len, total_len);

        // ── Step 1: Assert CS HIGH (master-initiated) ────────────────────────
        self.cs.set_high().map_err(|_| Error::Spi)?;

        // ── Step 2: Wait for slave TXN_READY (RDY rising edge) ───────────────
        // The slave detects CS HIGH and responds by raising RDY.
        // Timeout: 2000 ms — matches `SPI_WAIT_TXN_TIMEOUT_MS` in the reference.
        match with_timeout(
            Duration::from_millis(2000),
            self.txn_ready_signal.wait(),
        )
        .await
        {
            Ok(()) => {}
            Err(_) => {
                let _ = self.cs.set_low();
                return Err(Error::Timeout);
            }
        }

        // ── Step 3: Full-duplex transfer ─────────────────────────────────────
        // frame[0..total_len] goes out as TX; simultaneously the slave clocks in its
        // own [header | data] which overwrites frame[0..total_len] as the RX side.
        match self.spi.transfer_in_place(&mut frame[..total_len]).await {
            Ok(()) => {}
            Err(_) => {
                let _ = self.cs.set_low();
                return Err(Error::Spi);
            }
        }

        // ── Steps 4 & 5: Parse slave header; cache slave payload ─────────────
        let slave_hdr = SpiHeader::from_bytes(&frame[..8]);
        let slave_payload_len = slave_hdr.len as usize;

        #[cfg(feature = "defmt")]
        {
            let magic = slave_hdr.magic;
            defmt::trace!("slave hdr: magic={:04x} len={}", magic, slave_payload_len);
        }

        self.rx_cache.clear();
        if slave_hdr.is_valid() && slave_payload_len > 0 {
            // Slave data that arrived in the first full-duplex exchange sits at
            // frame[8..total_len].  Capture min(available, needed) bytes.
            let in_first = (total_len - 8).min(slave_payload_len);
            let _ = self.rx_cache.extend_from_slice(&frame[8..8 + in_first]);

            // If slave has more payload than fitted in the first exchange, read it now.
            // Reference: `psh->len + sizeof(struct spi_header) > xfer_size`
            if slave_payload_len > in_first {
                let remain = slave_payload_len - in_first;
                let remain_padded = (remain + 3) & !3;
                match self.spi.read(&mut frame[..remain_padded]).await {
                    Ok(()) => {
                        let _ = self.rx_cache.extend_from_slice(&frame[..remain]);
                    }
                    Err(_) => {
                        let _ = self.cs.set_low();
                        return Err(Error::Spi);
                    }
                }
            }
        }

        // ── Step 6: Wait for header acknowledgment (RDY falling edge) ────────
        // Timeout: 100 ms — matches `SPI_WAIT_HDR_ACK_TIMEOUT_MS`.
        // Per reference: timeout here is non-fatal; we break if RDY is already LOW.
        let _ = with_timeout(
            Duration::from_millis(100),
            self.hdr_ack_signal.wait(),
        )
        .await;

        // ── Step 7: Deassert CS ───────────────────────────────────────────────
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok(payload_len)
    }

    /// Read slave data.
    ///
    /// First drains any payload cached from the most recent `write()` exchange.
    /// If the cache is empty, performs a **slave-initiated** transaction:
    /// waits for RDY, exchanges an empty TX header to receive the slave's frame.
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        // Drain cache from the previous write() full-duplex exchange
        if !self.rx_cache.is_empty() {
            let n = self.rx_cache.len().min(buffer.len());
            buffer[..n].copy_from_slice(&self.rx_cache[..n]);
            self.rx_cache.clear();
            return Ok(n);
        }

        // ── Step 1: Wait for slave TXN_READY ─────────────────────────────────
        // Signal may already be set if RDY is HIGH — wait() returns immediately.
        with_timeout(
            Duration::from_millis(2000),
            self.txn_ready_signal.wait(),
        )
        .await
        .map_err(|_| Error::Timeout)?;

        // ── Step 2: Assert CS HIGH ────────────────────────────────────────────
        self.cs.set_high().map_err(|_| Error::Spi)?;

        // ── Step 3: Full-duplex — send empty TX header, receive slave header ──
        // Reference: `SPI_HEADER_INIT(&mh, 0, 0); xfer_size = sizeof(struct spi_header)`
        let mut frame = [0u8; MAX_FRAME];
        frame[..8].copy_from_slice(&SpiHeader::new_at_cmd(0).to_bytes());

        match self.spi.transfer_in_place(&mut frame[..8]).await {
            Ok(()) => {}
            Err(_) => {
                let _ = self.cs.set_low();
                return Err(Error::Spi);
            }
        }

        let slave_hdr = SpiHeader::from_bytes(&frame[..8]);
        let slave_payload_len = slave_hdr.len as usize;

        #[cfg(feature = "defmt")]
        {
            let magic = slave_hdr.magic;
            defmt::trace!("slave hdr: magic={:04x} len={}", magic, slave_payload_len);
        }

        if !slave_hdr.is_valid() {
            let _ = self.cs.set_low();
            #[cfg(feature = "defmt")]
            {
                let magic = slave_hdr.magic;
                defmt::warn!("invalid slave header magic={:04x}", magic);
            }
            return Err(Error::InvalidResponse);
        }

        // ── Step 4: Read slave payload (second part) ──────────────────────────
        let received = if slave_payload_len > 0 {
            if slave_payload_len > buffer.len() {
                let _ = self.cs.set_low();
                return Err(Error::BufferTooSmall);
            }
            let padded = (slave_payload_len + 3) & !3;
            match self.spi.read(&mut frame[..padded]).await {
                Ok(()) => {}
                Err(_) => {
                    let _ = self.cs.set_low();
                    return Err(Error::Spi);
                }
            }
            buffer[..slave_payload_len].copy_from_slice(&frame[..slave_payload_len]);
            slave_payload_len
        } else {
            0
        };

        // ── Step 5: Wait for HDR_ACK ──────────────────────────────────────────
        let _ = with_timeout(
            Duration::from_millis(100),
            self.hdr_ack_signal.wait(),
        )
        .await;

        // ── Step 6: Deassert CS ───────────────────────────────────────────────
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok(received)
    }
}
