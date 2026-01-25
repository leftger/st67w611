//! Embassy-net driver for ST67W611 (T02 firmware)
//!
//! This module provides an embassy-net compatible driver for the ST67W611 module
//! when using T02 firmware (LwIP on host architecture).
//!
//! With T02 firmware, the module acts as a WiFi MAC/PHY only, and the TCP/IP stack
//! runs on the host MCU using embassy-net.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                      Host MCU                           │
//! │  ┌─────────────────────────────────────────────────┐   │
//! │  │              Application                         │   │
//! │  └─────────────────┬───────────────────────────────┘   │
//! │                    │                                    │
//! │  ┌─────────────────▼───────────────────────────────┐   │
//! │  │           embassy-net (TCP/IP stack)             │   │
//! │  └─────────────────┬───────────────────────────────┘   │
//! │                    │                                    │
//! │  ┌─────────────────▼───────────────────────────────┐   │
//! │  │     ST67W611Driver (this module)                 │   │
//! │  │     - Implements embassy-net-driver traits       │   │
//! │  │     - Handles frame encapsulation                │   │
//! │  └─────────────────┬───────────────────────────────┘   │
//! │                    │ SPI (frame protocol)              │
//! └────────────────────┼────────────────────────────────────┘
//!                      │
//! ┌────────────────────▼────────────────────────────────────┐
//! │              ST67W611 Module (T02 firmware)              │
//! │  - WiFi MAC/PHY only                                    │
//! │  - Passes raw Ethernet frames                           │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```no_run,ignore
//! use st67w611_driver::net::embassy_driver::{St67w611Driver, State};
//! use embassy_net::{Stack, StackResources};
//!
//! // Create driver and state
//! let state = make_static!(State::<MTU, 4, 4>::new());
//! let (device, runner) = St67w611Driver::new(spi_transport, state);
//!
//! // Spawn the runner task
//! spawner.spawn(wifi_runner(runner)).unwrap();
//!
//! // Use with embassy-net
//! let stack = Stack::new(device, config, resources, seed);
//! ```

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};

use crate::error::{Error, Result};

/// Maximum Transmission Unit for Ethernet frames
pub const MTU: usize = 1514;

/// SPI frame header magic code
const SPI_HEADER_MAGIC: u16 = 0x55AA;

/// Traffic types for SPI frame protocol
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficType {
    /// AT commands and responses
    AtCommand = 0,
    /// WiFi Station network data (Ethernet frames)
    NetworkSta = 1,
    /// WiFi AP network data (Ethernet frames)
    NetworkAp = 2,
    /// Bluetooth HCI data
    Hci = 3,
    /// OpenThread/802.15.4 data
    OpenThread = 4,
}

/// SPI frame header (8 bytes)
///
/// Wire format (little-endian):
/// - Bytes 0-1: Magic (0x55AA)
/// - Bytes 2-3: Payload length
/// - Byte 4: Version (2 bits) | RX stall (1 bit) | Flags (5 bits)
/// - Byte 5: Traffic type
/// - Bytes 6-7: Reserved
#[derive(Debug, Clone, Copy)]
pub struct SpiFrameHeader {
    /// Magic code (always 0x55AA)
    pub magic: u16,
    /// Payload length (not including header)
    pub length: u16,
    /// Protocol version (2 bits)
    pub version: u8,
    /// Peer RX is stalled
    pub rx_stall: bool,
    /// Flags (5 bits)
    pub flags: u8,
    /// Traffic type
    pub traffic_type: TrafficType,
}

impl SpiFrameHeader {
    /// Create a new header for sending data
    pub fn new(traffic_type: TrafficType, payload_len: u16) -> Self {
        Self {
            magic: SPI_HEADER_MAGIC,
            length: payload_len,
            version: 0,
            rx_stall: false,
            flags: 0,
            traffic_type,
        }
    }

    /// Serialize header to bytes (little-endian, 8 bytes)
    pub fn to_bytes(&self) -> [u8; 8] {
        let flags_byte =
            (self.version & 0x03) | ((self.rx_stall as u8) << 2) | ((self.flags & 0x1F) << 3);
        [
            (self.magic & 0xFF) as u8,
            (self.magic >> 8) as u8,
            (self.length & 0xFF) as u8,
            (self.length >> 8) as u8,
            flags_byte,
            self.traffic_type as u8,
            0, // reserved
            0, // reserved
        ]
    }

    /// Parse header from bytes
    pub fn from_bytes(bytes: &[u8; 8]) -> Option<Self> {
        let magic = u16::from_le_bytes([bytes[0], bytes[1]]);
        if magic != SPI_HEADER_MAGIC {
            return None;
        }

        let length = u16::from_le_bytes([bytes[2], bytes[3]]);
        let flags_byte = bytes[4];
        let traffic_type_raw = bytes[5];

        let traffic_type = match traffic_type_raw {
            0 => TrafficType::AtCommand,
            1 => TrafficType::NetworkSta,
            2 => TrafficType::NetworkAp,
            3 => TrafficType::Hci,
            4 => TrafficType::OpenThread,
            _ => return None, // Invalid traffic type
        };

        Some(Self {
            magic,
            length,
            version: flags_byte & 0x03,
            rx_stall: (flags_byte & 0x04) != 0,
            flags: (flags_byte >> 3) & 0x1F,
            traffic_type,
        })
    }

    /// Check if header is valid
    pub fn is_valid(&self) -> bool {
        self.magic == SPI_HEADER_MAGIC && (self.length as usize) <= MTU
    }
}

/// Packet buffer for TX/RX
#[derive(Debug)]
pub struct PacketBuf<const N: usize> {
    /// Packet data
    pub buf: [u8; N],
    /// Actual length of data in buffer
    pub len: usize,
}

impl<const N: usize> PacketBuf<N> {
    /// Create a new empty packet buffer
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }
}

impl<const N: usize> Default for PacketBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Driver state holding RX/TX queues
pub struct State<const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> {
    rx_channel: Channel<NoopRawMutex, PacketBuf<MTU>, RX_QUEUE>,
    tx_channel: Channel<NoopRawMutex, PacketBuf<MTU>, TX_QUEUE>,
    link_state: Signal<NoopRawMutex, bool>,
}

impl<const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    State<MTU, RX_QUEUE, TX_QUEUE>
{
    /// Create a new state instance
    pub const fn new() -> Self {
        Self {
            rx_channel: Channel::new(),
            tx_channel: Channel::new(),
            link_state: Signal::new(),
        }
    }
}

/// Embassy-net compatible driver for ST67W611
///
/// This driver implements the `embassy-net-driver` traits for integration
/// with embassy-net's TCP/IP stack.
pub struct St67w611Device<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> {
    state: &'d State<MTU, RX_QUEUE, TX_QUEUE>,
}

impl<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    St67w611Device<'d, MTU, RX_QUEUE, TX_QUEUE>
{
    /// Create a new driver instance
    fn new(state: &'d State<MTU, RX_QUEUE, TX_QUEUE>) -> Self {
        Self { state }
    }
}

/// Runner task that handles SPI communication
pub struct St67w611Runner<
    'd,
    SPI,
    CS,
    const MTU: usize,
    const RX_QUEUE: usize,
    const TX_QUEUE: usize,
> where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
{
    state: &'d State<MTU, RX_QUEUE, TX_QUEUE>,
    transport: St67w611Transport<SPI, CS>,
}

/// Low-level SPI transport with frame protocol
pub struct St67w611Transport<SPI, CS>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
{
    spi: SPI,
    cs: CS,
}

impl<SPI, CS> St67w611Transport<SPI, CS>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
{
    /// Create a new transport
    pub fn new(spi: SPI, cs: CS) -> Self {
        Self { spi, cs }
    }

    /// Send a frame with the given traffic type
    pub async fn send_frame(&mut self, traffic_type: TrafficType, data: &[u8]) -> Result<()> {
        // Calculate padded length (must be 4-byte aligned)
        let padded_len = (data.len() + 3) & !3;

        // Create header
        let header = SpiFrameHeader::new(traffic_type, padded_len as u16);
        let header_bytes = header.to_bytes();

        // Build frame
        let mut frame = [0u8; 8 + MTU + 4]; // header + max payload + padding
        frame[..8].copy_from_slice(&header_bytes);
        frame[8..8 + data.len()].copy_from_slice(data);

        // Padding with 0x88 as per spec
        for i in data.len()..padded_len {
            frame[8 + i] = 0x88;
        }

        let total_len = 8 + padded_len;

        // Assert CS and transfer
        self.cs.set_high().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        self.spi
            .write(&frame[..total_len])
            .await
            .map_err(|_| Error::Spi)?;

        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok(())
    }

    /// Receive a frame, returning traffic type and data length
    pub async fn receive_frame(&mut self, buffer: &mut [u8]) -> Result<(TrafficType, usize)> {
        // Assert CS
        self.cs.set_high().map_err(|_| Error::Spi)?;
        Timer::after(Duration::from_micros(10)).await;

        // Read header
        let mut header_bytes = [0u8; 8];
        self.spi
            .read(&mut header_bytes)
            .await
            .map_err(|_| Error::Spi)?;

        let header = SpiFrameHeader::from_bytes(&header_bytes).ok_or(Error::InvalidResponse)?;

        if !header.is_valid() {
            self.cs.set_low().map_err(|_| Error::Spi)?;
            return Err(Error::InvalidResponse);
        }

        let payload_len = header.length as usize;

        if payload_len == 0 {
            self.cs.set_low().map_err(|_| Error::Spi)?;
            return Ok((header.traffic_type, 0));
        }

        if payload_len > buffer.len() {
            self.cs.set_low().map_err(|_| Error::Spi)?;
            return Err(Error::BufferTooSmall);
        }

        // Read payload
        self.spi
            .read(&mut buffer[..payload_len])
            .await
            .map_err(|_| Error::Spi)?;

        Timer::after(Duration::from_micros(10)).await;
        self.cs.set_low().map_err(|_| Error::Spi)?;

        Ok((header.traffic_type, payload_len))
    }
}

impl<'d, SPI, CS, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    St67w611Runner<'d, SPI, CS, MTU, RX_QUEUE, TX_QUEUE>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
{
    /// Run the driver (call this in a spawned task)
    ///
    /// This task handles:
    /// - Receiving Ethernet frames from the module and queuing them for embassy-net
    /// - Sending Ethernet frames from embassy-net to the module
    pub async fn run(mut self) -> ! {
        loop {
            // Check for TX packets to send
            if let Ok(packet) = self.state.tx_channel.try_receive() {
                if let Err(_e) = self
                    .transport
                    .send_frame(TrafficType::NetworkSta, &packet.buf[..packet.len])
                    .await
                {
                    #[cfg(feature = "defmt")]
                    defmt::warn!("Failed to send frame: {:?}", _e);
                }
            }

            // Try to receive a frame
            let mut rx_buf = [0u8; MTU];
            match self.transport.receive_frame(&mut rx_buf).await {
                Ok((TrafficType::NetworkSta, len)) if len > 0 => {
                    let mut packet = PacketBuf::<MTU>::new();
                    packet.buf[..len].copy_from_slice(&rx_buf[..len]);
                    packet.len = len;

                    if self.state.rx_channel.try_send(packet).is_err() {
                        #[cfg(feature = "defmt")]
                        defmt::warn!("RX queue full, dropping frame");
                    }
                }
                Ok((TrafficType::NetworkAp, len)) if len > 0 => {
                    // Handle AP traffic if needed
                    #[cfg(feature = "defmt")]
                    defmt::trace!("Received AP frame: {} bytes", len);
                }
                Ok(_) => {
                    // No data or other traffic type
                }
                Err(_e) => {
                    #[cfg(feature = "defmt")]
                    defmt::trace!("RX error: {:?}", _e);
                }
            }

            // Small delay to prevent busy-looping
            Timer::after(Duration::from_micros(100)).await;
        }
    }
}

/// Create a new ST67W611 driver for embassy-net
///
/// Returns the device (for embassy-net Stack) and runner (spawn in a task).
///
/// # Type Parameters
/// - `MTU`: Maximum transmission unit (typically 1514 for Ethernet)
/// - `RX_QUEUE`: Number of RX packet buffers
/// - `TX_QUEUE`: Number of TX packet buffers
pub fn new_driver<'d, SPI, CS, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>(
    spi: SPI,
    cs: CS,
    state: &'d State<MTU, RX_QUEUE, TX_QUEUE>,
) -> (
    St67w611Device<'d, MTU, RX_QUEUE, TX_QUEUE>,
    St67w611Runner<'d, SPI, CS, MTU, RX_QUEUE, TX_QUEUE>,
)
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
{
    let device = St67w611Device::new(state);
    let runner = St67w611Runner {
        state,
        transport: St67w611Transport::new(spi, cs),
    };
    (device, runner)
}

// ============================================================================
// embassy-net-driver trait implementations
// ============================================================================

/// Capabilities of the network driver
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// Maximum transmission unit
    pub max_transmission_unit: usize,
    /// Medium type (always Ethernet)
    pub medium: Medium,
}

/// Medium type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// Ethernet medium
    Ethernet,
}

/// RX token for receiving a packet
pub struct RxToken<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> {
    state: &'d State<MTU, RX_QUEUE, TX_QUEUE>,
    packet: PacketBuf<MTU>,
}

/// TX token for transmitting a packet
pub struct TxToken<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> {
    state: &'d State<MTU, RX_QUEUE, TX_QUEUE>,
}

impl<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    St67w611Device<'d, MTU, RX_QUEUE, TX_QUEUE>
{
    /// Get driver capabilities
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            max_transmission_unit: MTU,
            medium: Medium::Ethernet,
        }
    }

    /// Get the link state
    pub fn link_state(&self) -> bool {
        // Try to get current state, default to false
        self.state.link_state.signaled()
    }

    /// Set the link state (call when WiFi connects/disconnects)
    pub fn set_link_state(&self, up: bool) {
        self.state.link_state.signal(up);
    }

    /// Try to receive a packet
    pub fn receive(
        &self,
    ) -> Option<(
        RxToken<'d, MTU, RX_QUEUE, TX_QUEUE>,
        TxToken<'d, MTU, RX_QUEUE, TX_QUEUE>,
    )> {
        match self.state.rx_channel.try_receive() {
            Ok(packet) => Some((
                RxToken {
                    state: self.state,
                    packet,
                },
                TxToken { state: self.state },
            )),
            Err(_) => None,
        }
    }

    /// Try to get a transmit token
    pub fn transmit(&self) -> Option<TxToken<'d, MTU, RX_QUEUE, TX_QUEUE>> {
        // Check if there's room in the TX queue
        if !self.state.tx_channel.is_full() {
            Some(TxToken { state: self.state })
        } else {
            None
        }
    }

    /// Get hardware address (MAC address)
    ///
    /// Note: This returns a placeholder. Call get_mac() on the WiFi manager
    /// after WiFi is initialized to get the real MAC address.
    pub fn hardware_address(&self) -> [u8; 6] {
        // TODO: Store actual MAC after WiFi init
        [0x00, 0x80, 0xE1, 0x00, 0x00, 0x00]
    }
}

impl<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    RxToken<'d, MTU, RX_QUEUE, TX_QUEUE>
{
    /// Consume the received packet
    pub fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, f: F) -> R {
        let mut packet = self.packet;
        f(&mut packet.buf[..packet.len])
    }
}

impl<'d, const MTU: usize, const RX_QUEUE: usize, const TX_QUEUE: usize>
    TxToken<'d, MTU, RX_QUEUE, TX_QUEUE>
{
    /// Consume the TX token and send a packet
    pub fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = PacketBuf::<MTU>::new();
        packet.len = len.min(MTU);
        let result = f(&mut packet.buf[..packet.len]);

        // Queue the packet for transmission
        let _ = self.state.tx_channel.try_send(packet);

        result
    }
}

// ============================================================================
// embassy-net-driver trait implementations
// ============================================================================

use embassy_net_driver::{
    Capabilities as DriverCapabilities, Driver, HardwareAddress, LinkState,
    RxToken as DriverRxToken, TxToken as DriverTxToken,
};

impl<'d, const MTU_SIZE: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> Driver
    for St67w611Device<'d, MTU_SIZE, RX_QUEUE, TX_QUEUE>
{
    type RxToken<'a>
        = RxToken<'a, MTU_SIZE, RX_QUEUE, TX_QUEUE>
    where
        Self: 'a;
    type TxToken<'a>
        = TxToken<'a, MTU_SIZE, RX_QUEUE, TX_QUEUE>
    where
        Self: 'a;

    fn receive(
        &mut self,
        _cx: &mut core::task::Context,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        St67w611Device::receive(self)
    }

    fn transmit(&mut self, _cx: &mut core::task::Context) -> Option<Self::TxToken<'_>> {
        St67w611Device::transmit(self)
    }

    fn link_state(&mut self, _cx: &mut core::task::Context) -> LinkState {
        if St67w611Device::link_state(self) {
            LinkState::Up
        } else {
            LinkState::Down
        }
    }

    fn capabilities(&self) -> DriverCapabilities {
        let mut caps = DriverCapabilities::default();
        caps.max_transmission_unit = MTU_SIZE;
        caps
    }

    fn hardware_address(&self) -> HardwareAddress {
        HardwareAddress::Ethernet(St67w611Device::hardware_address(self))
    }
}

impl<'d, const MTU_SIZE: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> DriverRxToken
    for RxToken<'d, MTU_SIZE, RX_QUEUE, TX_QUEUE>
{
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, f: F) -> R {
        RxToken::consume(self, f)
    }
}

impl<'d, const MTU_SIZE: usize, const RX_QUEUE: usize, const TX_QUEUE: usize> DriverTxToken
    for TxToken<'d, MTU_SIZE, RX_QUEUE, TX_QUEUE>
{
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        TxToken::consume(self, len, f)
    }
}
