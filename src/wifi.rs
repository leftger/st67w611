//! WiFi management API

use embassy_time::Duration;
use heapless::Vec;

use crate::at::command::{self, AtCommandString};
use crate::at::parser::{self, AtResponse};
use crate::at::processor::{AtProcessor, WiFiEvent};
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;
use crate::types::*;

/// WiFi manager
pub struct WiFiManager {
    /// Current WiFi state
    state: TmMutex<WiFiState>,
    /// AT processor reference
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl WiFiManager {
    /// Create a new WiFi manager
    pub const fn new(processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self {
            state: TmMutex::new(WiFiState::Uninitialized),
            processor,
            timeout,
        }
    }

    /// Initialize WiFi subsystem
    pub async fn init<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        mode: WiFiMode,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // Set WiFi mode
        let cmd = command::wifi::set_mode(mode)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response != AtResponse::Ok {
            return Err(Error::AtCommandFailed);
        }

        // Update state
        {
            let mut state = self.state.lock().await;
            *state = WiFiState::Disconnected;
        }

        Ok(())
    }

    /// Scan for WiFi networks
    pub async fn scan<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<ScanResults>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::wifi::scan()?;

        // Send as multi-response command
        let (slot, slot_idx) = self.processor.send_multi_response_command(spi, cmd.as_bytes()).await?;

        let mut results = ScanResults::new();

        // Collect all +CWLAP responses until we get OK or timeout
        let scan_timeout = embassy_time::Instant::now() + embassy_time::Duration::from_secs(30);

        loop {
            // Check for timeout
            if embassy_time::Instant::now() > scan_timeout {
                self.processor.release_multi_response_slot(slot_idx).await;
                return Err(Error::Timeout);
            }

            // Try to receive a data response (non-blocking)
            if let Some(response) = slot.try_receive_data_response() {
                if let AtResponse::Data { prefix, content } = response {
                    if prefix.as_str() == "+CWLAP" {
                        match parser::parse_scan_result(&content) {
                            Ok(scan_result) => {
                                if results.push(scan_result).is_err() {
                                    // Buffer full, stop collecting
                                    break;
                                }
                            }
                            Err(_) => {
                                // Failed to parse, skip this result
                                continue;
                            }
                        }
                    }
                }
                continue;
            }

            // Check if we got OK or ERROR (scan complete)
            match embassy_time::with_timeout(
                embassy_time::Duration::from_millis(100),
                slot.wait(embassy_time::Duration::from_secs(30))
            ).await {
                Ok(Ok(AtResponse::Ok)) => {
                    // Scan complete successfully
                    break;
                }
                Ok(Ok(AtResponse::Error)) | Ok(Err(_)) => {
                    // Scan failed
                    self.processor.release_multi_response_slot(slot_idx).await;
                    return Err(Error::AtCommandFailed);
                }
                Err(_) => {
                    // Timeout on wait, continue collecting
                    continue;
                }
                _ => continue,
            }
        }

        // Release the slot
        self.processor.release_multi_response_slot(slot_idx).await;

        Ok(results)
    }

    /// Connect to a WiFi access point
    pub async fn connect<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        ssid: &str,
        password: &str,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // Update state to connecting
        {
            let mut state = self.state.lock().await;
            *state = WiFiState::Connecting;
        }

        // Send connect command
        let cmd = command::wifi::connect(ssid, password)?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), Duration::from_secs(20)).await?;

        if response != AtResponse::Ok {
            let mut state = self.state.lock().await;
            *state = WiFiState::Disconnected;
            return Err(Error::ConnectionFailed);
        }

        // Wait for GOT_IP event
        let event_channel = self.processor.wifi_event_receiver();
        match embassy_time::with_timeout(
            Duration::from_secs(30),
            self.wait_for_event(event_channel, |e| matches!(e, WiFiEvent::GotIp)),
        )
        .await
        {
            Ok(_) => {
                let mut state = self.state.lock().await;
                *state = WiFiState::GotIp;
                Ok(())
            }
            Err(_) => {
                let mut state = self.state.lock().await;
                *state = WiFiState::Connected; // Connected but no IP
                Err(Error::Timeout)
            }
        }
    }

    /// Disconnect from WiFi
    pub async fn disconnect<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        {
            let mut state = self.state.lock().await;
            *state = WiFiState::Disconnecting;
        }

        let cmd = command::wifi::disconnect()?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if response != AtResponse::Ok {
            return Err(Error::AtCommandFailed);
        }

        {
            let mut state = self.state.lock().await;
            *state = WiFiState::Disconnected;
        }

        Ok(())
    }

    /// Get current IP configuration
    pub async fn get_ip_config<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<IpConfig>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::wifi::get_station_ip()?;

        // Send as multi-response command
        let (slot, slot_idx) = self.processor.send_multi_response_command(spi, cmd.as_bytes()).await?;

        let mut ip = None;
        let mut gateway = None;
        let mut netmask = None;

        // Collect all IP config responses
        let config_timeout = embassy_time::Instant::now() + embassy_time::Duration::from_secs(5);

        loop {
            // Check for timeout
            if embassy_time::Instant::now() > config_timeout {
                self.processor.release_multi_response_slot(slot_idx).await;
                return Err(Error::Timeout);
            }

            // Try to receive a data response
            if let Some(response) = slot.try_receive_data_response() {
                if let AtResponse::Data { prefix, content } = response {
                    if prefix.as_str().starts_with("+CIPSTA") {
                        if let Some((field, addr)) = parser::parse_ip_config_line(&prefix, &content)? {
                            match field {
                                parser::IpConfigField::Ip => ip = Some(addr),
                                parser::IpConfigField::Gateway => gateway = Some(addr),
                                parser::IpConfigField::Netmask => netmask = Some(addr),
                            }
                        }
                    }
                }
                continue;
            }

            // Check if we got OK or ERROR
            match embassy_time::with_timeout(
                embassy_time::Duration::from_millis(100),
                slot.wait(embassy_time::Duration::from_secs(5))
            ).await {
                Ok(Ok(AtResponse::Ok)) => {
                    // Query complete
                    break;
                }
                Ok(Ok(AtResponse::Error)) | Ok(Err(_)) => {
                    // Query failed
                    self.processor.release_multi_response_slot(slot_idx).await;
                    return Err(Error::AtCommandFailed);
                }
                Err(_) => {
                    // Timeout on wait, continue collecting
                    continue;
                }
                _ => continue,
            }
        }

        // Release the slot
        self.processor.release_multi_response_slot(slot_idx).await;

        // Build IpConfig from collected data
        match (ip, gateway, netmask) {
            (Some(ip), Some(gateway), Some(netmask)) => Ok(IpConfig { ip, gateway, netmask }),
            _ => Err(Error::InvalidResponse),
        }
    }

    /// Get MAC address
    pub async fn get_mac<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<MacAddress>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = command::wifi::get_mac()?;
        let response = self.processor.send_command(spi, cmd.as_bytes(), self.timeout).await?;

        if let AtResponse::Data { prefix, content } = response {
            if prefix.as_str() == "+CIPSTAMAC" {
                let mac_str = parser::unquote(&content);
                return parser::parse_mac(mac_str);
            }
        }

        Err(Error::InvalidResponse)
    }

    /// Get current WiFi state
    pub async fn get_state(&self) -> WiFiState {
        let state = self.state.lock().await;
        *state
    }

    /// Wait for a specific WiFi event
    async fn wait_for_event<F>(&self, channel: &embassy_sync::channel::Channel<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, WiFiEvent, 4>, predicate: F) -> Result<()>
    where
        F: Fn(&WiFiEvent) -> bool,
    {
        loop {
            let event = channel.receive().await;
            if predicate(&event) {
                return Ok(());
            }
        }
    }

    /// Stream WiFi events
    pub async fn events(&self) -> impl core::future::Future<Output = WiFiEvent> + '_ {
        self.processor.wifi_event_receiver().receive()
    }
}
