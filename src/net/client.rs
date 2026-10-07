//! High-level network configuration and services.
//!
//! [`Net`] wraps the `AT+CIP*` / `AT+CIPSNTP*` configuration and service
//! commands in a typed, transport-generic API. It is the counterpart to
//! [`crate::wifi::WiFi`]: use it for interface options (IPv6, receive mode,
//! multiple connections), DNS/SNTP/ping, TLS options and TCP servers.
//!
//! ```no_run
//! use st67w611::net::client::Net;
//!
//! async fn setup<C: st67w611::at::AtTransport>(control: C) -> Result<(), st67w611::Error> {
//!     let net = Net::new(control);
//!     net.set_ipv6(true).await?;
//!     net.lookup("example.com").await?;
//!     Ok(())
//! }
//! ```

use crate::at::command::network as cmd;
use crate::at::parser::{parse_csv, parse_int, parse_ip, unquote};
use crate::at::reply::AtOutput;
use crate::at::transport::{AtTransport, AtTransportExt};
use crate::at::LineBuffer;
use crate::error::{Error, Result};
use crate::types::Ipv4Address;

/// A certificate or key stored in the module's filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CertificateType {
    /// CA certificate.
    Ca,
    /// Client certificate.
    Client,
    /// Client private key.
    ClientKey,
}

impl CertificateType {
    /// The filename the module expects for this certificate.
    fn filename(&self) -> &'static str {
        match self {
            CertificateType::Ca => "ca_cert.pem",
            CertificateType::Client => "client_cert.pem",
            CertificateType::ClientKey => "client_key.pem",
        }
    }
}

/// High-level network client.
///
/// `T` is any [`AtTransport`]; see the module docs.
pub struct Net<T> {
    transport: T,
}

impl<T> Net<T> {
    /// Create a network client over `transport`.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// The underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Consume the client, returning the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: AtTransport> Net<T> {
    // ---- Interface options ----------------------------------------------

    /// Enable or disable multiple connections (`AT+CIPMUX`).
    pub async fn set_mux(&self, enable: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_mux(enable)).await
    }

    /// Enable or disable the data-info prefix on received data (`AT+CIPDINFO`).
    pub async fn set_data_info(&self, enable: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_data_info(enable)).await
    }

    /// Enable or disable IPv6 (`AT+CIPV6`).
    pub async fn set_ipv6(&self, enable: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_ipv6(enable)).await
    }

    /// Set the receive mode (`AT+CIPRECVMODE`).
    ///
    /// With buffered mode on, incoming data is pulled with
    /// [`receive`](Self::receive).
    pub async fn set_receive_mode(&self, buffered: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_receive_mode(buffered)).await
    }

    /// Set a socket's receive buffer length (`AT+CIPRECVBUF=<link>,<len>`).
    pub async fn set_receive_buffer(&self, link_id: u8, len: u32) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_receive_buffer(link_id, len))
            .await
    }

    /// Query a socket's receive buffer length.
    pub async fn receive_buffer(&self, link_id: u8) -> Result<AtOutput> {
        self.transport.exec(cmd::get_receive_buffer(link_id)).await
    }

    /// Read buffered data (`AT+CIPRECVDATA=<link>,<len>`).
    pub async fn receive(&self, link_id: u8, len: usize) -> Result<AtOutput> {
        self.transport.exec(cmd::receive(link_id, len)).await
    }

    /// Set per-socket TCP options (`AT+CIPTCPOPT`).
    pub async fn set_tcp_options(&self, link_id: u8, options: u32) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_tcp_options(link_id, options))
            .await
    }

    // ---- TLS options -----------------------------------------------------

    /// Set the TLS SNI (`AT+CIPSSLCSNI`).
    pub async fn set_sni(&self, link_id: u8, hostname: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_sni(link_id, hostname))
            .await
    }

    /// Configure a socket's TLS (`AT+CIPSSLCCONF`).
    pub async fn configure_ssl(&self, link_id: u8, auth_mode: u8) -> Result<()> {
        self.transport
            .exec_ok(cmd::configure_ssl(link_id, auth_mode))
            .await
    }

    /// Set a TLS pre-shared key (`AT+CIPSSLCPSK`).
    pub async fn set_ssl_psk(&self, link_id: u8, psk: &str, hint: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_ssl_psk(link_id, psk, hint))
            .await
    }

    /// Set the TLS ALPN list (`AT+CIPSSLCALPN`).
    pub async fn set_ssl_alpn(&self, link_id: u8, alpn: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_ssl_alpn(link_id, alpn))
            .await
    }

    // ---- DNS -------------------------------------------------------------

    /// Resolve a hostname (`AT+CIPDOMAIN`).
    pub async fn lookup(&self, hostname: &str) -> Result<AtOutput> {
        self.transport.exec(cmd::dns_lookup(hostname)).await
    }

    /// Resolve a hostname for a specific address family (`0` = auto).
    pub async fn lookup_typed(&self, hostname: &str, address_type: i32) -> Result<AtOutput> {
        self.transport
            .exec(cmd::dns_lookup_typed(hostname, address_type))
            .await
    }

    /// Configure DNS servers (`AT+CIPDNS`).
    pub async fn set_dns(&self, enable: bool, dns1: &str, dns2: Option<&str>) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_dns(enable, dns1, dns2))
            .await
    }

    /// Query the DNS configuration.
    pub async fn dns(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_dns()).await
    }

    // ---- SNTP ------------------------------------------------------------

    /// Configure SNTP (`AT+CIPSNTPCFG`).
    pub async fn configure_sntp(&self, enable: bool, timezone: i8, server: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::configure_sntp(enable, timezone, server))
            .await
    }

    /// Set the SNTP sync interval (`AT+CIPSNTPINTV`).
    pub async fn set_sntp_interval(&self, interval: u16) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_sntp_interval(interval))
            .await
    }

    /// Query the SNTP time (`AT+CIPSNTPTIME?`).
    pub async fn sntp_time(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_sntp_time()).await
    }

    // ---- Ping ------------------------------------------------------------

    /// Ping a host with default parameters.
    pub async fn ping(&self, host: &str) -> Result<AtOutput> {
        self.transport.exec(cmd::ping(host)).await
    }

    /// Ping with explicit parameters.
    pub async fn ping_with(
        &self,
        host: &str,
        length: u16,
        count: u16,
        interval: u16,
        timeout: Option<u16>,
    ) -> Result<AtOutput> {
        self.transport
            .exec(cmd::ping_with(host, length, count, interval, timeout))
            .await
    }

    // ---- TCP server ------------------------------------------------------

    /// Start a TCP server (`AT+CIPSERVER=1,...`).
    pub async fn start_server(&self, port: u16, ip: &str, backlog: u16, timeout: u32) -> Result<()> {
        self.transport
            .exec_ok(cmd::start_server(port, ip, backlog, timeout))
            .await
    }

    /// Stop a TCP server (`AT+CIPSERVER=0,...`).
    pub async fn stop_server(&self, close_connections: u16) -> Result<()> {
        self.transport
            .exec_ok(cmd::stop_server(close_connections))
            .await
    }

    /// Set the server's maximum connections (`AT+CIPSERVERMAXCONN`).
    pub async fn set_server_max_connections(&self, max: u16) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_server_max_connections(max))
            .await
    }

    // ---- Certificates ----------------------------------------------------

    /// Upload a certificate or key to the module's filesystem.
    ///
    /// The data is written in 512-byte chunks via `AT+FS`. Any existing file of
    /// the same name is deleted first.
    pub async fn upload_certificate(
        &self,
        cert_type: CertificateType,
        cert_data: &[u8],
    ) -> Result<()> {
        const CHUNK: usize = 512;
        let filename = cert_type.filename();

        // Best-effort delete of any previous file.
        let _ = self.delete_certificate(cert_type).await;

        let mut offset = 0;
        while offset < cert_data.len() {
            let end = core::cmp::min(offset + CHUNK, cert_data.len());
            let chunk = &cert_data[offset..end];
            self.transport
                .exec_payload(
                    crate::at::command::filesystem::fs_write(filename, offset, chunk.len()),
                    chunk,
                )
                .await?;
            offset = end;
        }
        Ok(())
    }

    /// Delete a certificate from the module's filesystem (`AT+FS`).
    ///
    /// A missing file is not an error.
    pub async fn delete_certificate(&self, cert_type: CertificateType) -> Result<()> {
        // OK or ERROR are both acceptable: the file may not exist yet.
        let _ = self
            .transport
            .exec(crate::at::command::filesystem::fs_delete(cert_type.filename()))
            .await?;
        Ok(())
    }

    /// Read a certificate back from the module's filesystem (`AT+FS`).
    ///
    /// Returns the raw response lines; the payload layout is module-specific.
    pub async fn read_certificate(
        &self,
        cert_type: CertificateType,
        len: usize,
    ) -> Result<AtOutput> {
        self.transport
            .exec(crate::at::command::filesystem::fs_read(
                cert_type.filename(),
                0,
                len,
            ))
            .await
    }

    // ---- Typed helpers ---------------------------------------------------

    /// Resolve a hostname to an IPv4 address (`AT+CIPDOMAIN`).
    pub async fn resolve(&self, hostname: &str) -> Result<Ipv4Address> {
        self.resolve_typed(hostname, 0).await
    }

    /// Resolve a hostname for a specific address family (`0` = auto).
    pub async fn resolve_typed(&self, hostname: &str, address_type: i32) -> Result<Ipv4Address> {
        let out = self.lookup_typed(hostname, address_type).await?;
        let value = out
            .value_after("+CIPDOMAIN:")
            .ok_or(Error::InvalidResponse)?;
        parse_ip(unquote(value))
    }

    /// The currently configured DNS servers.
    pub async fn dns_servers(&self) -> Result<(Ipv4Address, Option<Ipv4Address>)> {
        let out = self.dns().await?;
        let value = out
            .value_after("+CIPDNS_CUR:")
            .or_else(|| out.value_after("+CIPDNS:"))
            .ok_or(Error::InvalidResponse)?;
        let parts = parse_csv(value);
        if parts.is_empty() {
            return Err(Error::InvalidResponse);
        }
        let primary = parse_ip(unquote(&parts[0]))?;
        let secondary = if parts.len() > 1 {
            Some(parse_ip(unquote(&parts[1]))?)
        } else {
            None
        };
        Ok((primary, secondary))
    }

    /// SNTP time as a string (`AT+CIPSNTPTIME?`).
    pub async fn sntp_time_string(&self) -> Result<LineBuffer> {
        let out = self.sntp_time().await?;
        let value = out
            .value_after("+CIPSNTPTIME:")
            .ok_or(Error::InvalidResponse)?;
        let mut time = LineBuffer::new();
        time.push_str(unquote(value))
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(time)
    }

    /// Ping a host and return the round-trip time.
    pub async fn ping_rtt(&self, host: &str) -> Result<PingResult> {
        let out = self.ping(host).await?;
        match out.value_after("+PING:").and_then(|v| parse_int(v).ok()) {
            Some(rtt) => Ok(PingResult {
                rtt_ms: rtt as u32,
                success: true,
            }),
            None => Ok(PingResult {
                rtt_ms: 0,
                success: false,
            }),
        }
    }
}

/// Result of a ping.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PingResult {
    /// Round-trip time in milliseconds.
    pub rtt_ms: u32,
    /// Whether the ping was successful.
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at::transport::test_support::{block_on, output_with, Mock};

    #[test]
    fn interface_options() {
        let net = Net::new(Mock::default());
        block_on(net.set_mux(true)).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPMUX=1");
        block_on(net.set_receive_mode(true)).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPRECVMODE=1");
        block_on(net.set_ipv6(true)).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPV6=1");
    }

    #[test]
    fn lookup_and_dns() {
        let net = Net::new(Mock::default());
        block_on(net.lookup("example.com")).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPDOMAIN=\"example.com\",0");
        block_on(net.set_dns(true, "8.8.8.8", Some("8.8.4.4"))).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPDNS=1,\"8.8.8.8\",\"8.8.4.4\"");
        block_on(net.dns()).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPDNS?");
    }

    #[test]
    fn server_and_ping() {
        let net = Net::new(Mock::default());
        block_on(net.stop_server(1)).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPSERVER=0,1");
        block_on(net.ping("1.1.1.1")).unwrap();
        assert_eq!(net.transport().last(), "AT+PING=\"1.1.1.1\",64,4,1000");
    }

    #[test]
    fn receive_buffer_returns_lines() {
        let net = Net::new(Mock {
            reply: Some(output_with("+CIPRECVBUF:1,256")),
            ..Default::default()
        });
        let out = block_on(net.receive_buffer(1)).unwrap();
        assert_eq!(out.value_after("+CIPRECVBUF:"), Some("1,256"));
    }

    #[test]
    fn resolve_parses_ip() {
        let net = Net::new(Mock {
            reply: Some(output_with("+CIPDOMAIN:93.184.216.34")),
            ..Default::default()
        });
        let ip = block_on(net.resolve("example.com")).unwrap();
        assert_eq!(net.transport().last(), "AT+CIPDOMAIN=\"example.com\",0");
        assert_eq!(ip.octets(), [93, 184, 216, 34]);
    }

    #[test]
    fn dns_servers_parse_primary_and_secondary() {
        let net = Net::new(Mock {
            reply: Some(output_with("+CIPDNS_CUR:\"8.8.8.8\",\"8.8.4.4\"")),
            ..Default::default()
        });
        let (primary, secondary) = block_on(net.dns_servers()).unwrap();
        assert_eq!(primary.octets(), [8, 8, 8, 8]);
        assert_eq!(secondary.unwrap().octets(), [8, 8, 4, 4]);
    }

    #[test]
    fn ping_rtt_parses_and_handles_failure() {
        let net = Net::new(Mock {
            reply: Some(output_with("+PING:23")),
            ..Default::default()
        });
        let result = block_on(net.ping_rtt("1.1.1.1")).unwrap();
        assert_eq!(result.rtt_ms, 23);
        assert!(result.success);

        let net = Net::new(Mock {
            reply: Some(output_with("ERROR")),
            ..Default::default()
        });
        let result = block_on(net.ping_rtt("1.1.1.1")).unwrap();
        assert!(!result.success);
    }
}
