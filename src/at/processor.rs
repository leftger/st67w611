//! AT command processor and RX handler

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Timer};
use heapless::Vec;

use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::at::parser::{self, AtResponse, LineBuffer, MAX_LINE_LEN};
use crate::sync::{TmMutex, TmSignal};

/// Maximum number of response slots for concurrent commands
pub const MAX_RESPONSE_SLOTS: usize = 8;

/// WiFi event types
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum WiFiEvent {
    /// Connected to AP
    Connected,
    /// Disconnected from AP
    Disconnected,
    /// Got IP address
    GotIp,
}

/// Socket event types
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SocketEvent {
    /// Socket connected
    Connected(u8),
    /// Socket closed
    Closed(u8),
    /// Data received on socket
    DataReceived { link_id: u8, length: usize },
}

/// Response slot for waiting on command responses
pub struct ResponseSlot {
    /// Whether this slot is in use
    in_use: TmMutex<bool>,
    /// Signal for response delivery
    signal: TmSignal<Result<AtResponse>>,
}

impl ResponseSlot {
    /// Create a new response slot
    pub const fn new() -> Self {
        Self {
            in_use: TmMutex::new(false),
            signal: Signal::new(),
        }
    }

    /// Try to allocate this slot
    pub async fn try_allocate(&self) -> bool {
        let mut in_use = self.in_use.lock().await;
        if *in_use {
            false
        } else {
            *in_use = true;
            true
        }
    }

    /// Release this slot
    pub async fn release(&self) {
        let mut in_use = self.in_use.lock().await;
        *in_use = false;
    }

    /// Wait for response with timeout
    pub async fn wait(&self, timeout: Duration) -> Result<AtResponse> {
        match with_timeout(timeout, self.signal.wait()).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Signal a response
    pub fn signal(&self, response: Result<AtResponse>) {
        self.signal.signal(response);
    }
}

/// Response slot pool
pub struct ResponseSlotPool {
    slots: [ResponseSlot; MAX_RESPONSE_SLOTS],
}

impl ResponseSlotPool {
    /// Create a new slot pool
    pub const fn new() -> Self {
        const SLOT: ResponseSlot = ResponseSlot::new();
        Self {
            slots: [SLOT; MAX_RESPONSE_SLOTS],
        }
    }

    /// Allocate a slot from the pool
    pub async fn allocate(&self) -> Result<&ResponseSlot> {
        for slot in &self.slots {
            if slot.try_allocate().await {
                return Ok(slot);
            }
        }
        Err(Error::NoResponseSlot)
    }
}

/// AT command processor
pub struct AtProcessor {
    /// Response slot pool
    response_slots: ResponseSlotPool,
    /// Current active response slot
    active_slot: TmMutex<Option<usize>>,
    /// WiFi event channel
    wifi_events: Channel<CriticalSectionRawMutex, WiFiEvent, 4>,
    /// Socket event channel
    socket_events: Channel<CriticalSectionRawMutex, SocketEvent, 16>,
}

impl AtProcessor {
    /// Create a new AT processor
    pub const fn new() -> Self {
        Self {
            response_slots: ResponseSlotPool::new(),
            active_slot: TmMutex::new(None),
            wifi_events: Channel::new(),
            socket_events: Channel::new(),
        }
    }

    /// Get WiFi event channel receiver
    pub fn wifi_event_receiver(&self) -> &Channel<CriticalSectionRawMutex, WiFiEvent, 4> {
        &self.wifi_events
    }

    /// Get socket event channel receiver
    pub fn socket_event_receiver(&self) -> &Channel<CriticalSectionRawMutex, SocketEvent, 16> {
        &self.socket_events
    }

    /// Send a command and wait for response
    pub async fn send_command<SPI, CS>(
        &self,
        spi: &TmMutex<SpiTransport<SPI, CS>>,
        command: &[u8],
        timeout: Duration,
    ) -> Result<AtResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // Allocate a response slot
        let slot = self.response_slots.allocate().await?;
        let slot_idx = self.get_slot_index(slot);

        // Set as active slot
        {
            let mut active = self.active_slot.lock().await;
            *active = Some(slot_idx);
        }

        // Send command over SPI
        {
            let mut spi_guard = spi.lock().await;
            spi_guard.write(command).await?;
        }

        // Wait for response
        let result = slot.wait(timeout).await;

        // Clear active slot and release
        {
            let mut active = self.active_slot.lock().await;
            *active = None;
        }
        slot.release().await;

        result
    }

    /// Process a received line
    fn process_line(&self, line: &str) -> Result<()> {
        // Parse the line
        let response = match parser::parse_line(line)? {
            Some(r) => r,
            None => return Ok(()), // Empty line
        };

        // Check if this is an unsolicited event
        if let AtResponse::Data { ref prefix, ref content } = response {
            if self.handle_unsolicited_event(prefix, content) {
                return Ok(());
            }
        }

        // Otherwise, signal to the active response slot
        // Note: In a real implementation, we'd need a way to match responses to specific commands
        // For now, we signal the first waiting slot
        for slot in &self.response_slots.slots {
            // Try to signal (non-blocking check if slot is in use)
            // This is a simplified version - real implementation needs better matching
            slot.signal(Ok(response.clone()));
            break;
        }

        Ok(())
    }

    /// Handle unsolicited events
    fn handle_unsolicited_event(&self, prefix: &str, content: &str) -> bool {
        match prefix {
            "+CW:CONNECTED" => {
                let _ = self.wifi_events.try_send(WiFiEvent::Connected);
                true
            }
            "+CW:DISCONNECTED" => {
                let _ = self.wifi_events.try_send(WiFiEvent::Disconnected);
                true
            }
            "+CW:GOT_IP" => {
                let _ = self.wifi_events.try_send(WiFiEvent::GotIp);
                true
            }
            _ if prefix.starts_with("+IPD") => {
                // Handle received data notification
                // Format: +IPD,<link_id>,<length>
                if let Ok(parts) = self.parse_ipd(content) {
                    let _ = self.socket_events.try_send(SocketEvent::DataReceived {
                        link_id: parts.0,
                        length: parts.1,
                    });
                }
                true
            }
            _ => false,
        }
    }

    /// Parse IPD notification
    fn parse_ipd(&self, content: &str) -> Result<(u8, usize)> {
        let parts = parser::parse_csv(content);
        if parts.len() >= 2 {
            let link_id = parser::parse_int(&parts[0])? as u8;
            let length = parser::parse_int(&parts[1])? as usize;
            Ok((link_id, length))
        } else {
            Err(Error::ParseError)
        }
    }

    /// Get slot index
    fn get_slot_index(&self, slot: &ResponseSlot) -> usize {
        let slot_ptr = slot as *const ResponseSlot;
        let base_ptr = &self.response_slots.slots[0] as *const ResponseSlot;
        ((slot_ptr as usize) - (base_ptr as usize)) / core::mem::size_of::<ResponseSlot>()
    }

    /// RX processor task - continuously reads from SPI and processes lines
    pub async fn rx_task<SPI, CS>(
        &'static self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let mut rx_buffer = [0u8; 4096];
        let mut line_buffer = LineBuffer::new();

        loop {
            // Read from SPI
            let len = {
                let mut spi_guard = spi.lock().await;
                match spi_guard.read(&mut rx_buffer).await {
                    Ok(len) => len,
                    Err(_) => {
                        // Error reading, wait and retry
                        Timer::after(Duration::from_millis(100)).await;
                        continue;
                    }
                }
            };

            // Process received data line by line
            for &byte in &rx_buffer[..len] {
                if byte == b'\n' {
                    // Process complete line
                    let _ = self.process_line(line_buffer.as_str());
                    line_buffer.clear();
                } else if byte != b'\r' {
                    // Add to line buffer (skip CR)
                    if line_buffer.push(byte as char).is_err() {
                        // Buffer full, clear and continue
                        line_buffer.clear();
                    }
                }
            }

            // Small delay between reads
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}
