//! ST67W611 WiFi Module Driver
//!
//! An async, no_std driver for ST67W611 WiFi modules using the Embassy framework.
//!
//! # Features
//!
//! - Async/await API using Embassy
//! - No heap allocation (uses heapless collections)
//! - WiFi connectivity (station and AP modes)
//! - TCP/UDP sockets
//! - TLS/SSL support
//! - MQTT client
//! - HTTP client
//! - embassy-net integration
//!
//! # Example
//!
//! ```no_run
//! use st67w611_driver::{Driver, Config};
//! use embassy_executor::Spawner;
//!
//! #[embassy_executor::main]
//! async fn main(spawner: Spawner) {
//!     // Initialize SPI and GPIO
//!     let spi = /* your SPI setup */;
//!     let cs = /* your CS pin setup */;
//!
//!     // Create driver
//!     let config = Config::default();
//!     let driver = Driver::new(spi, cs, config);
//!
//!     // Initialize WiFi
//!     driver.init_wifi().await.unwrap();
//!
//!     // Connect to network
//!     driver.wifi_connect("MySSID", "password").await.unwrap();
//!
//!     // Use the driver...
//! }
//! ```

#![no_std]
#![allow(async_fn_in_trait)]
#![warn(missing_docs)]

// Re-export embassy types that users need
pub use embassy_time::Duration;

// Module declarations
pub mod advanced;
pub mod at;
pub mod bus;
pub mod config;
pub mod error;
pub mod http;
pub mod mqtt;
pub mod net;
pub mod sync;
pub mod tls;
pub mod types;
pub mod wifi;

// Public API exports
pub use config::Config;
pub use error::{Error, Result};
pub use types::*;

use at::processor::AtProcessor;
use bus::SpiTransportAuto;
use embassy_time::Duration as EmbassyDuration;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiDevice;
use net::{NetworkDevice, St67w611Driver};
use sync::TmMutex;
use wifi::WiFiManager;

/// Main driver instance
pub struct Driver<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    /// SPI transport
    spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    /// AT processor
    processor: &'static AtProcessor,
    /// WiFi manager
    wifi: &'static WiFiManager,
    /// Network device
    network: &'static NetworkDevice,
    /// TLS manager
    tls: &'static tls::TlsManager,
    /// Configuration
    config: Config,
}

impl<SPI, CS> Driver<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    /// Create a new driver instance
    ///
    /// Note: This function requires static references to be created by the user
    /// using `make_static!` or similar macros. See examples for details.
    pub fn new(
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        processor: &'static AtProcessor,
        wifi: &'static WiFiManager,
        network: &'static NetworkDevice,
        tls: &'static tls::TlsManager,
        config: Config,
    ) -> Self {
        Self {
            spi,
            processor,
            wifi,
            network,
            tls,
            config,
        }
    }

    /// Initialize WiFi subsystem
    pub async fn init_wifi(&self, mode: WiFiMode) -> Result<()> {
        self.wifi.init(self.spi, mode).await
    }

    /// Scan for WiFi networks
    pub async fn wifi_scan(&self) -> Result<ScanResults> {
        self.wifi.scan(self.spi).await
    }

    /// Connect to a WiFi access point
    pub async fn wifi_connect(&self, ssid: &str, password: &str) -> Result<()> {
        let result = self.wifi.connect(self.spi, ssid, password).await;

        // Update network device link state based on connection result
        if result.is_ok() {
            self.network.set_link_state(true);
        }

        result
    }

    /// Disconnect from WiFi
    pub async fn wifi_disconnect(&self) -> Result<()> {
        let result = self.wifi.disconnect(self.spi).await;

        // Update network device link state
        self.network.set_link_state(false);

        result
    }

    /// Get current IP configuration
    pub async fn get_ip_config(&self) -> Result<IpConfig> {
        self.wifi.get_ip_config(self.spi).await
    }

    /// Get MAC address
    pub async fn get_mac(&self) -> Result<MacAddress> {
        self.wifi.get_mac(self.spi).await
    }

    /// Get WiFi state
    pub async fn get_wifi_state(&self) -> WiFiState {
        self.wifi.get_state().await
    }

    /// Get the network device (for embassy-net integration)
    pub fn network_device(&self) -> &NetworkDevice {
        self.network
    }

    /// Get the TLS manager
    pub fn tls_manager(&self) -> &tls::TlsManager {
        self.tls
    }

    /// Create an MQTT client
    pub fn mqtt_client(&self, link_id: u8) -> mqtt::MqttClient {
        mqtt::MqttClient::new(link_id, self.processor, self.config.command_timeout)
    }

    /// Create an HTTP client
    pub fn http_client(&self) -> http::HttpClient {
        http::HttpClient::new(self.network, self.processor, self.config.command_timeout)
    }

    /// Create a DNS resolver
    pub fn dns_resolver(&self) -> advanced::DnsResolver {
        advanced::DnsResolver::new(self.processor, self.config.command_timeout)
    }

    /// Create an SNTP client
    pub fn sntp_client(&self) -> advanced::SntpClient {
        advanced::SntpClient::new(self.processor, self.config.command_timeout)
    }

    /// Create a ping utility
    pub fn ping(&self) -> advanced::Ping {
        advanced::Ping::new(self.processor, self.config.command_timeout)
    }

    /// Get the AT processor for direct access
    pub fn processor(&self) -> &AtProcessor {
        self.processor
    }

    /// Spawn the RX processor task
    ///
    /// This task must be running for the driver to function.
    /// Call this once during initialization.
    pub async fn run_rx_task(&'static self) {
        self.processor.rx_task(self.spi).await
    }

    /// Spawn the IPD processor task
    ///
    /// This task handles incoming socket data (+IPD notifications).
    /// Call this once during initialization.
    pub async fn run_ipd_task(&'static self) {
        self.network.ipd_processor_task().await
    }
}

/// Helper macro to create static resources
///
/// This is a common pattern in Embassy applications for creating
/// static references required by the driver.
#[macro_export]
macro_rules! make_static {
    ($val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<_> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
    ($ty:ty) => {{
        static STATIC_CELL: static_cell::StaticCell<$ty> = static_cell::StaticCell::new();
        STATIC_CELL.uninit()
    }};
}

// Re-export commonly used types
pub use at::{AtCommand, AtResponse};
pub use bus::SpiTransport;
pub use http::{HttpClient, HttpMethod, HttpRequest, HttpResponse};
pub use mqtt::{MqttClient, MqttConfig, MqttMessage};
pub use tls::{CertificateType, TlsManager};
