//! Full-duplex SPI transfer engine.
//!
//! The ST67W611 does not have separate "write" and "read" transactions. Every
//! time the host asserts chip-select, the module answers with its own 8-byte
//! header on MISO while the host clocks its header (and, if there is room, its
//! payload) out on MOSI. This module implements that exchange, faithfully to
//! `spi_xfer_one()` in the reference `Driver/W61_bus/spi_iface.c`:
//!
//! 1. wait for the module's RDY line to be asserted,
//! 2. assert CS,
//! 3. one full-duplex transfer of `HEADER_LEN + align4(payload)`,
//! 4. if the module announced a longer frame than was clocked, a second,
//!    read-only transfer for the remainder (still inside the same CS),
//! 5. de-assert CS and wait (leniently) for the header acknowledgement.
//!
//! Two details matter for correctness and are easy to get wrong:
//!
//! * When the module reports `rx_stall`, the host must **stop attaching
//!   payloads**: the next transaction carries a bare header until the module
//!   clears the bit. The engine tracks that state and returns
//!   [`Received::tx_deferred`] so the caller can retry the payload later.
//! * Chip-select is active **high** (`spi_port_set_cs(1)` asserts it).

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;

use crate::bus::frame::{
    aligned_len, frame_len, Header, HeaderError, TrafficType, HEADER_LEN, MAX_PAYLOAD, PAD_BYTE,
};
use crate::error::{Error, Result};

/// How long to wait for the module to assert RDY before a transaction.
///
/// `SPI_WAIT_TXN_TIMEOUT_MS` in `spi_port.h`.
pub const TXN_READY_TIMEOUT: Duration = Duration::from_millis(2000);

/// How long to wait for a single SPI transfer to complete.
///
/// `SPI_WAIT_MSG_XFER_TIMEOUT_MS` in `spi_port.h`.
pub const MSG_XFER_TIMEOUT: Duration = Duration::from_millis(500);

/// How long to wait (leniently) for the header acknowledgement.
///
/// `SPI_WAIT_HDR_ACK_TIMEOUT_MS` in `spi_port.h`.
pub const HDR_ACK_TIMEOUT: Duration = Duration::from_millis(100);

/// How many read-only frames to clock while the peer reports its RX stalled
/// before giving up on ever getting our own frame out.
pub const STALL_DRAIN_MAX: usize = 4;

/// Minimum CS setup/hold time around a transfer.
const CS_DELAY: Duration = Duration::from_micros(1);

/// Minimum delay between a CS deassert and the next assert.
///
/// ST's port refuses to re-assert CS until 2 us have elapsed since the previous
/// deassert; see the `spi_port_set_cs` comment in [`Self::exchange`].
pub const CS_GAP: Duration = Duration::from_micros(2);

/// Scratch size shared by the TX and RX staging buffers.
const SCRATCH: usize = HEADER_LEN + MAX_PAYLOAD;

/// One outbound frame handed to [`Engine::exchange`].
#[derive(Debug, Clone, Copy)]
pub struct Outbound<'a> {
    /// Traffic type to tag the frame with.
    pub traffic_type: TrafficType,
    /// Payload; the header (length, traffic type) is added by the engine.
    pub payload: &'a [u8],
}

impl<'a> Outbound<'a> {
    /// Build an outbound AT-command frame.
    pub const fn at(payload: &'a [u8]) -> Self {
        Self {
            traffic_type: TrafficType::AtCommand,
            payload,
        }
    }
}

/// Result of a single [`Engine::exchange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Received {
    /// Raw traffic type byte the module reported.
    pub traffic_type: u8,
    /// Number of payload bytes copied into the caller's buffer.
    pub len: usize,
    /// The module's frame (or the caller's buffer) was larger than what fit.
    pub truncated: bool,
    /// The module had `rx_stall` set, so the outbound payload was **not** sent.
    /// Retry it after [`Engine::rx_stalled`] clears.
    pub tx_deferred: bool,
}

impl Received {
    /// Decoded traffic type, if known.
    pub fn traffic_type(&self) -> Option<TrafficType> {
        TrafficType::from_u8(self.traffic_type)
    }
}

/// Size of the first, full-duplex transfer for a given outbound payload.
///
/// When `rx_stall` is set the reference sends a bare header, so only the
/// header is clocked out.
pub const fn first_xfer_len(payload_len: usize, rx_stall: bool) -> usize {
    if rx_stall {
        HEADER_LEN
    } else {
        frame_len(payload_len)
    }
}

/// Number of *extra* bytes to read after the first transfer.
///
/// The module's frame can be longer than what was clocked in the first
/// transfer; the remainder is read in a second transfer, rounded up to the
/// 4-byte boundary like the reference does.
pub const fn second_xfer_len(slave_payload_len: usize, first_len: usize) -> usize {
    let slave_frame_len = HEADER_LEN + slave_payload_len;
    if slave_frame_len > first_len {
        aligned_len(slave_frame_len - first_len)
    } else {
        0
    }
}

/// Full-duplex SPI transfer engine for one ST67W611.
///
/// `SPI` is a raw [`SpiBus`] (the engine drives chip-select itself, since it
/// must stay asserted across the two-part transfer). `CS` is the active-high
/// chip-select pin.
///
/// `ready` is signalled by the RDY EXTI handler on every *rising* edge;
/// `rdy_level` mirrors the pin's current level so the engine can skip waiting
/// when RDY is already asserted (the common case right after boot).
/// `hdr_ack` is signalled on the *falling* edge.
pub struct Engine<SPI, CS> {
    spi: SPI,
    cs: CS,
    ready: &'static Signal<CriticalSectionRawMutex, ()>,
    hdr_ack: &'static Signal<CriticalSectionRawMutex, ()>,
    rdy_level: &'static AtomicBool,
    rx_stall: bool,
    tx_scratch: [u8; SCRATCH],
    rx_scratch: [u8; SCRATCH],
}

impl<SPI, CS> Engine<SPI, CS>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    /// Create an engine around a raw SPI bus and chip-select pin.
    pub fn new(
        spi: SPI,
        cs: CS,
        ready: &'static Signal<CriticalSectionRawMutex, ()>,
        hdr_ack: &'static Signal<CriticalSectionRawMutex, ()>,
        rdy_level: &'static AtomicBool,
    ) -> Self {
        Self {
            spi,
            cs,
            ready,
            hdr_ack,
            rdy_level,
            rx_stall: false,
            tx_scratch: [0; SCRATCH],
            rx_scratch: [0; SCRATCH],
        }
    }

    /// Whether the module currently has its RX path stalled.
    pub const fn rx_stalled(&self) -> bool {
        self.rx_stall
    }

    /// Run one exchange, optionally sending `tx` and copying any received
    /// payload into `out`.
    ///
    /// Returns how many bytes were copied. `out` may be empty when only a
    /// header needs to be clocked.
    ///
    /// The peer's RX-stall handshake is handled here; see [`Self::one_exchange`].
    pub async fn exchange(&mut self, tx: Option<Outbound<'_>>, out: &mut [u8]) -> Result<Received> {
        // When the module reports its RX stalled it will not accept a payload
        // yet, so the frame has to wait. ST's driver keeps the frame queued and
        // sends it on the first transfer after the stall clears (spi_iface.c:
        // `rx_restore` drops the "free the txbuf" branch so the same frame goes
        // out again next time). We have no queue, so wait it out here: clock
        // read-only frames until the module is willing to receive, then send it
        // for real.
        //
        // Without this the frame is deferred forever and never actually sent,
        // because every caller ignores `tx_deferred`. That is exactly why only
        // the first exchange after boot ever worked: the first command went out,
        // its reply set the stall bit, and every command after that was silently
        // dropped while the AT layer waited for a reply to a command that had
        // never left the host.
        if tx.is_some() {
            for _ in 0..STALL_DRAIN_MAX {
                if !self.rx_stall {
                    break;
                }
                // Frames clocked while stalled are not this command's response;
                // they are events or banners nobody is waiting for, so discard
                // them. Returning one would hand the caller an answer to a
                // question it never asked.
                let _ = self.one_exchange(None, out).await;
            }
            if self.rx_stall {
                return Err(Error::Timeout);
            }
        }

        self.one_exchange(tx, out).await
    }

    /// One SPI transfer, honouring the stall bit reported by the peer.
    async fn one_exchange(&mut self, tx: Option<Outbound<'_>>, out: &mut [u8]) -> Result<Received> {
        let tx_deferred = self.rx_stall && tx.is_some();

        // Build the outbound frame and decide how many bytes to clock.
        let (send_payload, payload_len) = match tx {
            Some(frame) if !self.rx_stall => {
                if frame.payload.len() > MAX_PAYLOAD {
                    return Err(Error::BufferTooSmall);
                }
                (true, frame.payload.len())
            }
            _ => (false, 0),
        };
        let first_len = first_xfer_len(payload_len, !send_payload);

        {
            let header = Header::new(
                if send_payload {
                    tx.as_ref().map(|f| f.traffic_type).unwrap_or(TrafficType::AtCommand)
                } else {
                    TrafficType::AtCommand
                },
                payload_len as u16,
            );
            self.tx_scratch[..HEADER_LEN].copy_from_slice(&header.to_bytes());
            if send_payload {
                let payload = tx.as_ref().expect("send_payload implies payload").payload;
                self.tx_scratch[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
                // Pad the final partial word with 0x88, like `spi_buffer_alloc`.
                for b in &mut self.tx_scratch[HEADER_LEN + payload.len()..first_len] {
                    *b = PAD_BYTE;
                }
            }
        }

        // Wait for the module to be ready for a transaction — for EVERY transfer,
        // sends included.
        //
        // I previously changed this to gate only receive-only exchanges, having
        // misread spi_do_xfer: its `(txbuf != NULL) || (rx_pending == 1)` is only
        // the condition for *entering* the loop, not the readiness test. Inside,
        // spi_xfer_one() waits for SPI_EVT_TXN_RDY whenever wait_txn_rdy != 0 —
        // which is the default; SPI_XFER_F_SKIP_FIRST_TXN_WAIT exists solely to
        // skip that wait for the very first transaction. And SPI_EVT_TXN_RDY is
        // set by the RDY pin ISR (spi_on_txn_data_ready on the rising edge) and at
        // init if the pin is already high.
        //
        // So RDY means "the module will accept a transaction now", and the
        // correct behaviour is to gate everything on it. Re-check the level after
        // every wake-up rather than trusting a single signal.
        let deadline = Instant::now() + TXN_READY_TIMEOUT;
        while !self.rdy_level.load(Ordering::Acquire) {
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            let _ = with_timeout(deadline - now, self.ready.wait()).await;
        }

        // Mandatory inter-transfer gap. ST's port refuses to assert CS until at
        // least 2 us have passed since the previous deassert:
        //
        //   static int32_t last_falling_tick;
        //   if (state == 1) WAIT_FROM(last_falling_tick, MICROSECOND_TO_TICK(2));
        //   ...
        //   else { ...deassert...; last_falling_tick = SYSTICK_VALUE; }
        //
        // (Projects/NUCLEO-N657X0-Q/.../Target/spi_port.c, spi_port_set_cs.)
        // Back-to-back transfers with no gap are outside the module's spec, and
        // it is exactly what our exchanges were doing.
        Timer::after(CS_GAP).await;
        self.cs.set_high().map_err(|_| Error::Spi)?;
        Timer::after(CS_DELAY).await;

        let mut result = self.perform(first_len, out, tx_deferred).await;

        Timer::after(CS_DELAY).await;
        if self.cs.set_low().is_err() && result.is_ok() {
            result = Err(Error::Spi);
        }

        // A missing header ack is not fatal; the reference only counts it.
        let _ = with_timeout(HDR_ACK_TIMEOUT, self.hdr_ack.wait()).await;

        result
    }

    /// Steps 3–4 of the exchange, with CS already asserted.
    async fn perform(&mut self, first_len: usize, out: &mut [u8], tx_deferred: bool) -> Result<Received> {
        {
            let tx = &self.tx_scratch[..first_len];
            let rx = &mut self.rx_scratch[..first_len];
            with_timeout(MSG_XFER_TIMEOUT, self.spi.transfer(rx, tx))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| Error::Spi)?;
        }

        let mut header_bytes = [0u8; HEADER_LEN];
        header_bytes.copy_from_slice(&self.rx_scratch[..HEADER_LEN]);
        let slave = Header::from_bytes(&header_bytes);
        if slave.validate(&header_bytes).is_err() {
            // Bring-up tracing: the raw bytes say whether this is garbage, or a
            // real header read at the wrong offset (i.e. we are desynced by some
            // number of bytes from the previous exchange).
            #[cfg(feature = "defmt")]
            defmt::trace!(
                "hdr invalid: first_len={} raw={=[u8]}",
                first_len,
                &header_bytes[..]
            );
            return Err(Error::InvalidResponse);
        }

        let mut available = first_len - HEADER_LEN;
        let extra = second_xfer_len(slave.payload_len(), first_len);
        if extra > 0 {
            let rx = &mut self.rx_scratch[first_len..first_len + extra];
            with_timeout(MSG_XFER_TIMEOUT, self.spi.read(rx))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| Error::Spi)?;
            available += extra;
        }

        let slave_len = slave.payload_len();
        let to_copy = slave_len.min(out.len()).min(available);
        out[..to_copy].copy_from_slice(&self.rx_scratch[HEADER_LEN..HEADER_LEN + to_copy]);

        self.rx_stall = slave.rx_stall();

        Ok(Received {
            traffic_type: slave.traffic_type_raw(),
            len: to_copy,
            truncated: slave_len > to_copy,
            tx_deferred,
        })
    }

    /// Consume the engine, returning the SPI bus and chip-select pin.
    pub fn release(self) -> (SPI, CS) {
        (self.spi, self.cs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_transfer_fits_payload_plus_header() {
        assert_eq!(first_xfer_len(0, false), HEADER_LEN);
        assert_eq!(first_xfer_len(4, false), HEADER_LEN + 4);
        assert_eq!(first_xfer_len(5, false), HEADER_LEN + 8);
    }

    #[test]
    fn stalled_first_transfer_is_header_only() {
        assert_eq!(first_xfer_len(100, true), HEADER_LEN);
    }

    #[test]
    fn second_transfer_only_when_slave_frame_is_longer() {
        assert_eq!(second_xfer_len(100, HEADER_LEN), 100);
        assert_eq!(second_xfer_len(5, HEADER_LEN), 8);
        assert_eq!(second_xfer_len(4, HEADER_LEN + 64), 0);
        assert_eq!(second_xfer_len(1514, HEADER_LEN), 1516);
    }

    #[test]
    fn reference_constants() {
        assert_eq!(PAD_BYTE, 0x88);
        assert_eq!(MAX_PAYLOAD, 1520);
        assert_eq!(TXN_READY_TIMEOUT, Duration::from_millis(2000));
    }
}
