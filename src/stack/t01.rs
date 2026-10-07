//! T01 sockets: the module owns TCP/IP and the host drives it with AT
//! commands.
//!
//! [`TcpSocket`] is a thin `embedded-io-async` adapter over
//! [`NetworkDevice`](crate::net::NetworkDevice), so the same application code
//! (and [`crate::net::tls`]) works on T01 and T02 alike.

use embassy_time::Duration;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiDevice;

use crate::bus::SpiTransport;
use crate::error::Error;
use crate::net::device::NetworkDevice;
use crate::sync::TmMutex;
use crate::types::{SocketId, SocketProtocol};

/// Socket-layer error, in a form that satisfies [`embedded_io::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SocketError(pub Error);

impl core::fmt::Display for SocketError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl core::error::Error for SocketError {}

impl embedded_io::Error for SocketError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl From<Error> for SocketError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

/// A stream socket running on the module.
///
/// Despite the name (which matches the T02 `embassy-net` type), `protocol`
/// selects `Tcp` or `Ssl` — both are byte streams, and `Ssl` means the module
/// terminates TLS, so `AT+SSLOPEN`-style sockets behave the same way here.
pub struct TcpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    device: &'static NetworkDevice,
    spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    id: SocketId,
    connected: bool,
    timeout: Duration,
}

impl<SPI, CS> TcpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    /// Allocate a TCP socket and connect it to `host:port`.
    pub async fn connect(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
        host: &str,
        port: u16,
    ) -> Result<Self, SocketError> {
        Self::connect_with_protocol(device, spi, timeout, SocketProtocol::Tcp, host, port).await
    }

    /// Allocate a socket of any stream `protocol` and connect it.
    pub async fn connect_with_protocol(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
        protocol: SocketProtocol,
        host: &str,
        port: u16,
    ) -> Result<Self, SocketError> {
        let mut socket = Self::new_with_protocol(device, spi, timeout, protocol).await?;
        socket.connect_to(host, port).await?;
        Ok(socket)
    }

    /// Allocate an unconnected TCP socket.
    pub async fn new(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
    ) -> Result<Self, SocketError> {
        Self::new_with_protocol(device, spi, timeout, SocketProtocol::Tcp).await
    }

    /// Allocate an unconnected socket of any stream `protocol`.
    pub async fn new_with_protocol(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
        protocol: SocketProtocol,
    ) -> Result<Self, SocketError> {
        let id = device.allocate_socket(protocol).await?;
        Ok(Self {
            device,
            spi,
            id,
            connected: false,
            timeout,
        })
    }

    /// Connect an already-allocated socket.
    pub async fn connect_to(&mut self, host: &str, port: u16) -> Result<(), SocketError> {
        self.device
            .connect_socket(self.spi, self.id, host, port, self.timeout)
            .await?;
        self.connected = true;
        Ok(())
    }

    /// The underlying socket id.
    pub fn id(&self) -> SocketId {
        self.id
    }

    /// Whether the socket is currently connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Close the connection and release the socket.
    pub async fn close(&mut self) -> Result<(), SocketError> {
        // `close_socket` already returns the socket to the pool.
        let result = self
            .device
            .close_socket(self.spi, self.id, self.timeout)
            .await;
        self.connected = false;
        result.map_err(SocketError)
    }
}

impl<SPI, CS> embedded_io_async::ErrorType for TcpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    type Error = SocketError;
}

impl<SPI, CS> embedded_io_async::Read for TcpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.device
            .receive_socket(self.spi, self.id, buf, self.timeout)
            .await
            .map_err(SocketError)
    }
}

impl<SPI, CS> embedded_io_async::Write for TcpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, SocketError> {
        self.device
            .send_socket(self.spi, self.id, buf, self.timeout)
            .await
            .map_err(SocketError)
    }

    async fn flush(&mut self) -> Result<(), SocketError> {
        Ok(())
    }
}

/// A UDP socket running on the module.
///
/// UDP is message-oriented, so it deliberately does not implement
/// `embedded_io_async`'s byte-stream traits; use [`send`](Self::send) /
/// [`recv`](Self::recv) instead.
pub struct UdpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    device: &'static NetworkDevice,
    spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    id: SocketId,
    timeout: Duration,
}

impl<SPI, CS> UdpSocket<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    /// Allocate a socket and connect it to `host:port`.
    pub async fn connect(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
        host: &str,
        port: u16,
    ) -> Result<Self, SocketError> {
        let mut socket = Self::new(device, spi, timeout).await?;
        socket.connect_to(host, port).await?;
        Ok(socket)
    }

    /// Allocate an unconnected socket.
    pub async fn new(
        device: &'static NetworkDevice,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
    ) -> Result<Self, SocketError> {
        let id = device.allocate_socket(SocketProtocol::Udp).await?;
        Ok(Self {
            device,
            spi,
            id,
            timeout,
        })
    }

    /// Connect an already-allocated socket.
    pub async fn connect_to(&mut self, host: &str, port: u16) -> Result<(), SocketError> {
        self.device
            .connect_socket(self.spi, self.id, host, port, self.timeout)
            .await?;
        Ok(())
    }

    /// The underlying socket id.
    pub fn id(&self) -> SocketId {
        self.id
    }

    /// Send one datagram.
    pub async fn send(&mut self, data: &[u8]) -> Result<usize, SocketError> {
        self.device
            .send_socket(self.spi, self.id, data, self.timeout)
            .await
            .map_err(SocketError)
    }

    /// Receive one datagram.
    pub async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.device
            .receive_socket(self.spi, self.id, buf, self.timeout)
            .await
            .map_err(SocketError)
    }

    /// Close the socket.
    pub async fn close(&mut self) -> Result<(), SocketError> {
        self.device
            .close_socket(self.spi, self.id, self.timeout)
            .await
            .map_err(SocketError)
    }
}
