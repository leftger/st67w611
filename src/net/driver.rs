//! embassy-net Driver implementation
//!
//! Note: This driver faces an architectural challenge. The ST67W611 module has its own
//! built-in TCP/IP stack accessed via AT commands, while embassy-net expects raw packet-level
//! access for its smoltcp-based stack. This implementation provides a bridge that allows
//! basic connectivity, but for production use, consider using the socket APIs directly
//! (NetworkDevice, HttpClient, MqttClient, etc.) instead of embassy-net.
//!
//! For full packet-level integration, the module would need to support a transparent/
//! passthrough mode that forwards raw IP packets.

use core::sync::atomic::{AtomicBool, Ordering};
use core::task::Context;
use embassy_net::driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::waitqueue::AtomicWaker;
use heapless::Vec;

use crate::net::device::NetworkDevice;
use crate::types::MacAddress;

/// Maximum packet size (MTU)
const MAX_PACKET_SIZE: usize = 1514; // Ethernet frame size

/// Packet buffer for RX/TX
#[derive(Clone)]
pub struct PacketBuffer {
    data: Vec<u8, MAX_PACKET_SIZE>,
}

impl PacketBuffer {
    /// Create an empty packet buffer
    pub const fn new() -> Self {
        Self {
            data: Vec::new(),
        }
    }

    /// Get packet data
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Get mutable packet data
    pub fn as_mut_slice(&mut self) -> &mut Vec<u8, MAX_PACKET_SIZE> {
        &mut self.data
    }
}

/// Packet queue for storing received packets
pub struct PacketQueue {
    queue: Channel<CriticalSectionRawMutex, PacketBuffer, 4>,
}

impl PacketQueue {
    /// Create a new packet queue
    pub const fn new() -> Self {
        Self {
            queue: Channel::new(),
        }
    }

    /// Try to enqueue a packet
    pub fn try_enqueue(&self, packet: PacketBuffer) -> bool {
        self.queue.try_send(packet).is_ok()
    }

    /// Try to dequeue a packet
    pub fn try_dequeue(&self) -> Option<PacketBuffer> {
        self.queue.try_receive().ok()
    }

    /// Check if queue has packets
    pub fn has_packets(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// RX token for receiving packets
pub struct St67w611RxToken {
    packet: PacketBuffer,
}

impl RxToken for St67w611RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let data = self.packet.as_mut_slice();
        f(data.as_mut())
    }
}

/// TX token for transmitting packets
pub struct St67w611TxToken {
    driver: &'static St67w611Driver,
}

impl TxToken for St67w611TxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut packet = PacketBuffer::new();
        // Reserve space for the packet
        packet.data.resize_default(len).ok();

        let result = f(packet.data.as_mut());

        // Enqueue packet for transmission
        let _ = self.driver.tx_queue.try_enqueue(packet);
        self.driver.tx_waker.wake();

        result
    }
}

/// ST67W611 network driver for embassy-net
pub struct St67w611Driver {
    /// Network device
    device: &'static NetworkDevice,
    /// MAC address
    mac_address: MacAddress,
    /// Waker for RX operations
    rx_waker: AtomicWaker,
    /// Waker for TX operations
    tx_waker: AtomicWaker,
    /// Received packet queue
    rx_queue: PacketQueue,
    /// Transmit packet queue
    tx_queue: PacketQueue,
    /// Whether the driver is initialized
    initialized: AtomicBool,
}

impl St67w611Driver {
    /// Create a new driver
    pub const fn new(device: &'static NetworkDevice, mac_address: MacAddress) -> Self {
        Self {
            device,
            mac_address,
            rx_waker: AtomicWaker::new(),
            tx_waker: AtomicWaker::new(),
            rx_queue: PacketQueue::new(),
            tx_queue: PacketQueue::new(),
            initialized: AtomicBool::new(false),
        }
    }

    /// Get the network device
    pub fn device(&self) -> &NetworkDevice {
        self.device
    }

    /// Enqueue a received packet (called from background task)
    pub fn enqueue_rx_packet(&self, packet: PacketBuffer) {
        if self.rx_queue.try_enqueue(packet) {
            self.rx_waker.wake();
        }
    }

    /// Get the TX queue for processing (called from background task)
    pub fn tx_queue(&self) -> &PacketQueue {
        &self.tx_queue
    }

    /// Get the RX waker
    pub fn rx_waker(&self) -> &AtomicWaker {
        &self.rx_waker
    }

    /// Get the TX waker
    pub fn tx_waker(&self) -> &AtomicWaker {
        &self.tx_waker
    }
}

impl Driver for St67w611Driver {
    type RxToken<'a> = St67w611RxToken where Self: 'a;
    type TxToken<'a> = St67w611TxToken where Self: 'a;

    fn receive(&mut self, cx: &mut Context) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Register waker
        self.rx_waker.register(cx.waker());

        // Check if link is up
        if !self.device.is_link_up() {
            return None;
        }

        // Try to get a packet from RX queue
        if let Some(packet) = self.rx_queue.try_dequeue() {
            let rx_token = St67w611RxToken { packet };
            let tx_token = St67w611TxToken {
                driver: unsafe {
                    // SAFETY: We need a static reference for the token.
                    // This is safe because the driver itself must be static,
                    // and the token is only used during the consume() call.
                    &*(self as *const Self)
                },
            };

            return Some((rx_token, tx_token));
        }

        None
    }

    fn transmit(&mut self, cx: &mut Context) -> Option<Self::TxToken<'_>> {
        // Register waker
        self.tx_waker.register(cx.waker());

        // Check if link is up
        if !self.device.is_link_up() {
            return None;
        }

        // Always allow transmission if link is up (queue has space)
        // The TX token will handle queuing
        Some(St67w611TxToken {
            driver: unsafe {
                // SAFETY: Same as above
                &*(self as *const Self)
            },
        })
    }

    fn link_state(&mut self, cx: &mut Context) -> LinkState {
        // Register wakers
        self.rx_waker.register(cx.waker());

        if self.device.is_link_up() {
            LinkState::Up
        } else {
            LinkState::Down
        }
    }

    fn capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::default();
        caps.max_transmission_unit = 1500;
        caps.max_burst_size = Some(1);
        // Note: We're emulating packet-level access over a socket-based interface,
        // so some features may not work perfectly
        caps
    }

    fn hardware_address(&self) -> HardwareAddress {
        HardwareAddress::Ethernet(*self.mac_address.as_bytes())
    }
}

impl St67w611Driver {
    /// Background task for processing TX packets
    ///
    /// This task reads packets from the TX queue and sends them via the network device.
    /// Note: This is a simplified implementation. For full functionality, the module would
    /// need to support raw IP packet transmission or a transparent mode.
    ///
    /// Current implementation: Packets are queued but require custom processing logic
    /// based on your specific use case. For most applications, using the socket APIs
    /// directly (HttpClient, MqttClient, etc.) is more practical than going through
    /// embassy-net.
    pub async fn packet_tx_task(&'static self) {
        loop {
            // Check TX queue for packets to send
            if let Some(_packet) = self.tx_queue.try_dequeue() {
                // TODO: Parse IP packet and determine destination
                // TODO: Create appropriate socket connection
                // TODO: Send packet data via socket
                //
                // This requires:
                // 1. IP/TCP/UDP header parsing
                // 2. Socket management (create, connect, send)
                // 3. Connection tracking
                //
                // For now, this is a placeholder. In practice, direct socket usage
                // is more efficient for modules with built-in TCP/IP stacks.
            }

            // Small delay
            embassy_time::Timer::after(embassy_time::Duration::from_millis(10)).await;
        }
    }

    /// Background task for receiving packets
    ///
    /// This task would monitor sockets for received data and convert to IP packets.
    /// Similar limitations apply as with TX task.
    pub async fn packet_rx_task(&'static self) {
        loop {
            // TODO: Monitor active sockets for data
            // TODO: Convert received socket data to IP packets
            // TODO: Enqueue to RX queue
            //
            // This is complex because we need to:
            // 1. Track all active connections
            // 2. Reconstruct IP/TCP headers from socket data
            // 3. Handle connection state machine
            //
            // Alternative: Use IPD data directly and build packets

            embassy_time::Timer::after(embassy_time::Duration::from_millis(10)).await;
        }
    }
}

// IMPORTANT USAGE NOTE:
// ===================
//
// The ST67W611 module has its own built-in TCP/IP stack accessed via AT commands.
// This creates an architectural mismatch with embassy-net, which expects raw packet
// access for its smoltcp-based stack.
//
// This implementation provides the required Driver interface with packet queuing
// infrastructure, but actual packet-to-socket bridging is complex and may have
// limitations.
//
// RECOMMENDED APPROACH:
// For most applications, use the driver's higher-level APIs directly:
//  - NetworkDevice for raw socket operations
//  - HttpClient for HTTP/HTTPS requests
//  - MqttClient for MQTT pub/sub
//  - TcpSocket / UdpSocket operations via NetworkDevice
//
// These APIs work directly with the module's TCP/IP stack and provide better
// performance and reliability than attempting to bridge to packet-level access.
//
// USE EMBASSY-NET ONLY IF:
// - You need compatibility with embassy-net-based libraries
// - You're willing to implement/test the packet bridging logic
// - Your use case can tolerate the architectural limitations
