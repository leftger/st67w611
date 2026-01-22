//! Network device abstraction

use core::sync::atomic::{AtomicU8, Ordering};
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::Vec;

use crate::at::processor::{AtProcessor, SocketEvent};
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;
use crate::types::*;

/// Socket state information
#[derive(Debug)]
pub struct Socket {
    /// Socket state
    pub state: TmMutex<SocketState>,
    /// Protocol type
    pub protocol: TmMutex<Option<SocketProtocol>>,
    /// Receive buffer
    pub rx_buffer: TmMutex<Vec<u8, 2048>>,
}

impl Socket {
    /// Create a new socket
    pub const fn new() -> Self {
        Self {
            state: TmMutex::new(SocketState::Free),
            protocol: TmMutex::new(None),
            rx_buffer: TmMutex::new(Vec::new()),
        }
    }

    /// Allocate the socket
    pub async fn allocate(&self, protocol: SocketProtocol) -> Result<()> {
        let mut state = self.state.lock().await;
        if *state != SocketState::Free {
            return Err(Error::SocketInUse);
        }

        *state = SocketState::Allocated;
        let mut proto = self.protocol.lock().await;
        *proto = Some(protocol);

        Ok(())
    }

    /// Free the socket
    pub async fn free(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        *state = SocketState::Free;

        let mut proto = self.protocol.lock().await;
        *proto = None;

        let mut buf = self.rx_buffer.lock().await;
        buf.clear();

        Ok(())
    }

    /// Get socket state
    pub async fn get_state(&self) -> SocketState {
        let state = self.state.lock().await;
        *state
    }

    /// Set socket state
    pub async fn set_state(&self, new_state: SocketState) {
        let mut state = self.state.lock().await;
        *state = new_state;
    }

    /// Append data to the receive buffer
    pub async fn append_rx_data(&self, data: &[u8]) -> Result<usize> {
        let mut buffer = self.rx_buffer.lock().await;

        let mut bytes_written = 0;
        for &byte in data {
            if buffer.push(byte).is_err() {
                // Buffer full
                break;
            }
            bytes_written += 1;
        }

        Ok(bytes_written)
    }

    /// Read data from the receive buffer
    pub async fn read_rx_data(&self, dest: &mut [u8]) -> Result<usize> {
        let mut buffer = self.rx_buffer.lock().await;

        let to_read = core::cmp::min(dest.len(), buffer.len());
        dest[..to_read].copy_from_slice(&buffer[..to_read]);

        // Remove read data from buffer by shifting remaining data to the front
        let remaining = buffer.len() - to_read;
        if remaining > 0 {
            // Shift remaining bytes to the front
            for i in 0..remaining {
                buffer[i] = buffer[i + to_read];
            }
        }

        // Truncate to remove the consumed data
        buffer.truncate(remaining);

        Ok(to_read)
    }

    /// Get the number of bytes available in the receive buffer
    pub async fn available_rx_bytes(&self) -> usize {
        let buffer = self.rx_buffer.lock().await;
        buffer.len()
    }

    /// Clear the receive buffer
    pub async fn clear_rx_buffer(&self) {
        let mut buffer = self.rx_buffer.lock().await;
        buffer.clear();
    }
}

/// Network device
pub struct NetworkDevice {
    /// Socket pool
    sockets: [Socket; MAX_SOCKETS],
    /// AT processor
    processor: &'static AtProcessor,
    /// Link state (0 = down, 1 = up)
    link_state: AtomicU8,
}

impl NetworkDevice {
    /// Create a new network device
    pub const fn new(processor: &'static AtProcessor) -> Self {
        const SOCKET: Socket = Socket::new();
        Self {
            sockets: [SOCKET; MAX_SOCKETS],
            processor,
            link_state: AtomicU8::new(0),
        }
    }

    /// Allocate a socket
    pub async fn allocate_socket(&self, protocol: SocketProtocol) -> Result<SocketId> {
        for (id, socket) in self.sockets.iter().enumerate() {
            if socket.allocate(protocol).await.is_ok() {
                return Ok(SocketId::new(id as u8).unwrap());
            }
        }
        Err(Error::NoSocketAvailable)
    }

    /// Free a socket
    pub async fn free_socket(&self, id: SocketId) -> Result<()> {
        if id.raw() >= MAX_SOCKETS as u8 {
            return Err(Error::InvalidSocket);
        }

        self.sockets[id.raw() as usize].free().await
    }

    /// Get socket by ID
    pub fn get_socket(&self, id: SocketId) -> Result<&Socket> {
        if id.raw() >= MAX_SOCKETS as u8 {
            return Err(Error::InvalidSocket);
        }

        Ok(&self.sockets[id.raw() as usize])
    }

    /// Set link state
    pub fn set_link_state(&self, up: bool) {
        self.link_state.store(if up { 1 } else { 0 }, Ordering::Relaxed);
    }

    /// Get link state
    pub fn is_link_up(&self) -> bool {
        self.link_state.load(Ordering::Relaxed) != 0
    }

    /// Get socket event channel
    pub fn socket_event_receiver(&self) -> &Channel<CriticalSectionRawMutex, SocketEvent, 16> {
        self.processor.socket_event_receiver()
    }

    /// Connect socket
    pub async fn connect_socket<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        id: SocketId,
        host: &str,
        port: u16,
        timeout: embassy_time::Duration,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let socket = self.get_socket(id)?;

        // Get protocol
        let protocol = {
            let proto = socket.protocol.lock().await;
            proto.ok_or(Error::InvalidSocket)?
        };

        // Set state to connecting
        socket.set_state(SocketState::Connecting).await;

        // Send connect command
        let cmd = crate::at::command::network::connect(id.raw(), protocol, host, port)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), timeout).await?;

        if response != crate::at::AtResponse::Ok {
            socket.set_state(SocketState::Allocated).await;
            return Err(Error::ConnectionFailed);
        }

        // Set state to connected
        socket.set_state(SocketState::Connected).await;

        Ok(())
    }

    /// Send data on socket
    pub async fn send_socket<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        id: SocketId,
        data: &[u8],
        timeout: embassy_time::Duration,
    ) -> Result<usize>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let socket = self.get_socket(id)?;

        // Check state
        if socket.get_state().await != SocketState::Connected {
            return Err(Error::NotConnected);
        }

        // Send length command
        let cmd = crate::at::command::network::send(id.raw(), data.len())?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), timeout).await?;

        // Wait for ready prompt ">"
        if response != crate::at::AtResponse::ReadyPrompt {
            return Err(Error::SocketError);
        }

        // Send actual data
        {
            let mut spi_guard = spi.lock().await;
            spi_guard.write(data).await?;
        }

        // Wait for SEND OK
        // In a real implementation, we'd wait for the SEND OK response
        // For now, assume success
        Ok(data.len())
    }

    /// Close socket
    pub async fn close_socket<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        id: SocketId,
        timeout: embassy_time::Duration,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let socket = self.get_socket(id)?;
        socket.set_state(SocketState::Closing).await;

        let cmd = crate::at::command::network::close(id.raw())?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), timeout).await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::SocketError);
        }

        socket.free().await?;

        Ok(())
    }

    /// Receive data from socket (reads from local buffer first, then queries module if empty)
    pub async fn receive_socket<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        id: SocketId,
        buffer: &mut [u8],
        timeout: embassy_time::Duration,
    ) -> Result<usize>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let socket = self.get_socket(id)?;

        // Check state
        let state = socket.get_state().await;
        if state != SocketState::Connected {
            return Err(Error::NotConnected);
        }

        // First, check if we have data in the local buffer
        let available = socket.available_rx_bytes().await;
        if available > 0 {
            return socket.read_rx_data(buffer).await;
        }

        // If no data in buffer, try to receive from module using AT+CIPRECV
        let length_to_request = core::cmp::min(buffer.len(), 2048);
        let cmd = crate::at::command::network::receive(id.raw(), length_to_request)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), timeout).await?;

        // Parse the +CIPRECV response
        if let crate::at::AtResponse::Data { prefix, content } = response {
            if prefix.as_str() == "+CIPRECV" {
                // Content format: "<length>:<data>" but data comes in next reads
                // For simplicity, we'll return 0 for now and mark this as needing enhancement
                // A proper implementation would need to read the raw data bytes following this response
                return Ok(0);
            }
        }

        Ok(0)
    }

    /// Receive data from socket (non-blocking, reads from buffer only)
    pub async fn receive_socket_buffered(&self, id: SocketId, buffer: &mut [u8]) -> Result<usize> {
        let socket = self.get_socket(id)?;

        // Check state
        let state = socket.get_state().await;
        if state != SocketState::Connected {
            return Err(Error::NotConnected);
        }

        // Read from the socket's RX buffer
        socket.read_rx_data(buffer).await
    }

    /// Check how many bytes are available to receive on a socket
    pub async fn available_bytes(&self, id: SocketId) -> Result<usize> {
        let socket = self.get_socket(id)?;
        Ok(socket.available_rx_bytes().await)
    }

    /// Handle received data notification (+IPD event)
    /// This should be called when data arrives on a socket
    pub async fn handle_received_data(&self, link_id: u8, data: &[u8]) -> Result<()> {
        if let Some(id) = SocketId::new(link_id) {
            let socket = self.get_socket(id)?;

            // Append data to socket's receive buffer
            let bytes_written = socket.append_rx_data(data).await?;

            if bytes_written < data.len() {
                // Buffer overflow - some data was lost
                // In a production implementation, we might want to log this or notify the user
            }

            Ok(())
        } else {
            Err(Error::InvalidSocket)
        }
    }

    /// Background task to process IPD data from the AT processor
    /// This should be spawned as a task to continuously route received socket data
    pub async fn ipd_processor_task(&'static self) {
        let ipd_channel = self.processor.ipd_data_receiver();

        loop {
            // Wait for IPD data
            let ipd_data = ipd_channel.receive().await;

            // Route to appropriate socket buffer
            let _ = self.handle_received_data(ipd_data.link_id, &ipd_data.data).await;
        }
    }
}
