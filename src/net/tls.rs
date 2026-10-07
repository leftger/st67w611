//! TLS over any `embedded-io-async` socket, backed by `embedded-tls`.
//!
//! `embedded-tls` 0.19 speaks the `embedded-io-async` 0.7 traits, which both
//! firmware stacks' sockets implement ([`crate::stack::TcpSocket`]), so a
//! connected socket can be handed straight to a `TlsConnection`.
//!
//! Certificate verification comes from `embedded-tls`:
//!
//! * enable its `rustpki` feature for the `no_std` X.509 verifier
//!   (`embedded_tls::pki::CertVerifier`),
//! * or `webpki` on `std`,
//! * without either, [`embedded_tls::UnsecureProvider`] performs **no**
//!   verification, which is only appropriate for PSK or bring-up.
//!
//! ```ignore
//! use embassy_net::tcp::TcpSocket;
//! use st67w611::net::tls;
//!
//! let mut socket = TcpSocket::new(stack, &mut tcp_rx, &mut tcp_tx);
//! socket.connect((IpAddress::v4(1, 1, 1, 1), 443)).await?;
//!
//! // A TLS record can be up to 16 KiB; 16640 bytes is the usual safe size.
//! let mut record_rx = [0u8; 16640];
//! let mut record_tx = [0u8; 16640];
//! let mut tls = tls::client(socket, &mut record_rx, &mut record_tx);
//!
//! let config = embedded_tls::TlsConfig::new().with_server_name("example.com");
//! tls.open(embedded_tls::TlsContext::new(
//!     &config,
//!     embedded_tls::UnsecureProvider::new::<tls::Aes128GcmSha256>(rng),
//! ))
//! .await?;
//!
//! tls.write_all(b"GET / HTTP/1.0\r\nHost: example.com\r\n\r\n").await?;
//! ```

pub use embedded_tls;
pub use embedded_tls::Aes128GcmSha256;

/// A TLS client connection over an `embedded-io-async` stream.
pub type TlsClient<'a, Socket, CipherSuite = Aes128GcmSha256> =
    embedded_tls::TlsConnection<'a, Socket, CipherSuite>;

/// Wrap an already-connected socket in a TLS connection.
///
/// The handshake happens on the first call to `open`:
///
/// ```ignore
/// tls.open(embedded_tls::TlsContext::new(&config, provider)).await?;
/// ```
pub fn client<'a, Socket, CipherSuite>(
    socket: Socket,
    record_read_buffer: &'a mut [u8],
    record_write_buffer: &'a mut [u8],
) -> TlsClient<'a, Socket, CipherSuite>
where
    Socket: embedded_io_async::Read + embedded_io_async::Write,
    CipherSuite: embedded_tls::TlsCipherSuite + 'static,
{
    embedded_tls::TlsConnection::new(socket, record_read_buffer, record_write_buffer)
}
