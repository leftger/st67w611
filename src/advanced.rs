//! Advanced networking features
//!
//! This module provides advanced networking capabilities including:
//! - DNS resolution
//! - SNTP time synchronization
//! - Ping utilities

use embassy_time::Duration;
use heapless::String;

use crate::at::command;
use crate::at::processor::AtProcessor;
use crate::at::AtResponse;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;
use crate::types::Ipv4Address;

/// DNS resolver
pub struct DnsResolver {
    /// AT processor
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl DnsResolver {
    /// Create a new DNS resolver
    pub const fn new(processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self { processor, timeout }
    }

    /// Resolve a hostname to an IP address
    pub async fn resolve<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        hostname: &str,
    ) -> Result<Ipv4Address>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::dns_lookup(hostname)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        // Parse response: +CIPDOMAIN:<ip>
        if let AtResponse::Data { prefix, content } = response {
            if prefix.as_str() == "+CIPDOMAIN" {
                let ip_str = crate::at::parser::unquote(&content);
                return crate::at::parser::parse_ip(ip_str);
            }
        }

        Err(Error::InvalidResponse)
    }

    /// Set custom DNS servers
    pub async fn set_dns_servers<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        primary: &str,
        secondary: Option<&str>,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::set_dns(true, primary, secondary)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response == AtResponse::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Get current DNS server configuration
    pub async fn get_dns_servers<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<(Ipv4Address, Option<Ipv4Address>)>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::get_dns()?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        // Parse response: +CIPDNS_CUR:<primary>[,<secondary>]
        if let AtResponse::Data { prefix, content } = response {
            if prefix.as_str().starts_with("+CIPDNS") {
                let parts = crate::at::parser::parse_csv(&content);
                if !parts.is_empty() {
                    let primary_str = crate::at::parser::unquote(&parts[0]);
                    let primary = crate::at::parser::parse_ip(primary_str)?;

                    let secondary = if parts.len() > 1 {
                        let secondary_str = crate::at::parser::unquote(&parts[1]);
                        Some(crate::at::parser::parse_ip(secondary_str)?)
                    } else {
                        None
                    };

                    return Ok((primary, secondary));
                }
            }
        }

        Err(Error::InvalidResponse)
    }
}

/// SNTP time synchronization
pub struct SntpClient {
    /// AT processor
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl SntpClient {
    /// Create a new SNTP client
    pub const fn new(processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self { processor, timeout }
    }

    /// Configure SNTP with timezone and server
    pub async fn configure<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        enable: bool,
        timezone: i8,
        server: &str,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::configure_sntp(enable, timezone, server)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response == AtResponse::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Get current time from SNTP server
    pub async fn get_time<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<String<64>>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::get_sntp_time()?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        // Parse response: +CIPSNTPTIME:<time_string>
        if let AtResponse::Data { prefix, content } = response {
            if prefix.as_str() == "+CIPSNTPTIME" {
                let time_str = crate::at::parser::unquote(&content);
                let mut result = String::new();
                result.push_str(time_str).map_err(|_| Error::BufferTooSmall)?;
                return Ok(result);
            }
        }

        Err(Error::InvalidResponse)
    }
}

/// Ping result
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PingResult {
    /// Round-trip time in milliseconds
    pub rtt_ms: u32,
    /// Whether the ping was successful
    pub success: bool,
}

/// Ping utility
pub struct Ping {
    /// AT processor
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl Ping {
    /// Create a new ping utility
    pub const fn new(processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self { processor, timeout }
    }

    /// Ping a host
    pub async fn ping<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        host: &str,
    ) -> Result<PingResult>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::network::ping(host)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), Duration::from_secs(10)).await?;

        // Parse response: +PING:<time> or timeout
        if let AtResponse::Data { prefix, content } = response {
            if prefix.as_str() == "+PING" {
                if let Ok(rtt) = crate::at::parser::parse_int(&content) {
                    return Ok(PingResult {
                        rtt_ms: rtt as u32,
                        success: true,
                    });
                }
            }
        }

        // If we get here, ping failed or timed out
        Ok(PingResult {
            rtt_ms: 0,
            success: false,
        })
    }
}
