//! ST67W611 WiFi Module Driver
//!
//! An async, no_std driver for ST67W611 WiFi modules using the Embassy framework.
//!
//! # Firmware Architectures
//!
//! This driver supports two firmware architectures:
//!
//! ## T01 Firmware (default, `mission-t01` feature)
//!
//! The TCP/IP stack runs on the ST67W611 module. The host communicates via
//! AT commands for socket operations, HTTP, MQTT, etc.
//!
//! Features:
//! - WiFi connectivity (station and AP modes)
//! - TCP/UDP sockets via AT commands
//! - TLS/SSL with certificate management
//! - HTTP/HTTPS client
//! - MQTT client with QoS
//! - DNS, SNTP, Ping utilities
//! - Power management
//!
//! ## T02 Firmware (`mission-t02` feature)
//!
//! The TCP/IP stack runs on the host MCU using embassy-net. The module acts
//! as a WiFi MAC/PHY only, passing raw Ethernet frames.
//!
//! Features:
//! - embassy-net integration
//! - Full control over TCP/IP stack
//! - WiFi configuration via AT commands
//! - Raw Ethernet frame transport
//!
//! # Example (T01 Firmware)
//!
//! ```no_run,ignore
//! use st67w611::{
//!     at::processor::AtProcessor, bus::SpiTransport, Config, Driver,
//!     NetworkDevice, TlsManager, WiFiManager, WiFiMode, SocketProtocol,
//! };
//! use embassy_executor::Spawner;
//!
//! #[embassy_executor::main]
//! async fn main(spawner: Spawner) {
//!     // Create static resources (see examples for complete setup)
//!     let driver = /* create driver with make_static! */;
//!
//!     // Spawn background tasks
//!     spawner.spawn(rx_task(driver)).unwrap();
//!     spawner.spawn(ipd_task(driver)).unwrap();
//!
//!     // Initialize WiFi
//!     driver.init_wifi(WiFiMode::Station).await.unwrap();
//!     driver.wifi_connect("SSID", "password").await.unwrap();
//!
//!     // Use HTTP client
//!     let http = driver.http_client();
//!     let response = http.get(spi, "https://api.example.com").await.unwrap();
//! }
//! ```
//!
//! # Example (T02 Firmware with embassy-net)
//!
//! ```no_run,ignore
//! use st67w611::net::{new_driver, State, MTU};
//! use embassy_net::{Stack, StackResources};
//!
//! #[embassy_executor::main]
//! async fn main(spawner: Spawner) {
//!     // Create driver state
//!     let state = make_static!(State::<MTU, 4, 4>::new());
//!     let (device, runner) = new_driver(spi, cs, state);
//!
//!     // Spawn the runner task
//!     spawner.spawn(wifi_runner(runner)).unwrap();
//!
//!     // Use with embassy-net
//!     let stack = Stack::new(device, config, resources, seed);
//! }
//! ```
//!
//! See the `examples/` directory for complete working code.

#![no_std]
#![allow(async_fn_in_trait)]
#![warn(missing_docs)]

// Re-export embassy types that users need
pub use embassy_time::Duration;

// Module declarations - always available
pub mod at;
pub mod bus;
pub mod config;
pub mod error;
pub mod net;
pub mod sync;
pub mod types;
pub mod util;
pub mod wifi;

// T01-specific modules (AT command-based networking)
#[cfg(feature = "mission-t01")]
pub mod advanced;
#[cfg(feature = "mission-t01")]
pub mod http;
#[cfg(feature = "mission-t01")]
pub mod mqtt;
#[cfg(feature = "mission-t01")]
pub mod power;
#[cfg(feature = "mission-t01")]
pub mod tls;

// Public API exports
pub use config::Config;
pub use error::{Error, Result};
pub use types::*;

// T01-specific imports (private, for use in Driver struct)
#[cfg(feature = "mission-t01")]
use at::processor::AtProcessor;
#[cfg(feature = "mission-t01")]
use embedded_hal::digital::OutputPin;
#[cfg(feature = "mission-t01")]
use embedded_hal_async::spi::SpiDevice;

/// Main driver instance (T01 firmware only)
///
/// For T02 firmware, use [`net::St67w611Device`] with embassy-net instead.
#[cfg(feature = "mission-t01")]
pub struct Driver<SPI, CS>
where
    SPI: SpiDevice + 'static,
    CS: OutputPin + 'static,
{
    /// SPI transport
    spi: &'static sync::TmMutex<SpiTransport<SPI, CS>>,
    /// AT processor
    processor: &'static AtProcessor,
    /// WiFi manager
    wifi: &'static wifi::WiFiManager,
    /// Network device
    network: &'static net::NetworkDevice,
    /// TLS manager
    tls: &'static tls::TlsManager,
    /// Configuration
    config: Config,
}

#[cfg(feature = "mission-t01")]
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
        spi: &'static sync::TmMutex<SpiTransport<SPI, CS>>,
        processor: &'static AtProcessor,
        wifi: &'static wifi::WiFiManager,
        network: &'static net::NetworkDevice,
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

    /// Connect to WiFi with automatic retry on failure
    pub async fn wifi_connect_with_retry(
        &self,
        ssid: &str,
        password: &str,
        max_attempts: u8,
    ) -> Result<()> {
        util::retry_with_backoff(
            max_attempts,
            Duration::from_secs(1),
            Duration::from_secs(10),
            || async { self.wifi_connect(ssid, password).await },
        )
        .await
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

    /// Get the network device for direct socket operations
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

    /// Create a power manager
    pub fn power_manager(&self) -> power::PowerManager {
        power::PowerManager::new(self.processor, self.config.command_timeout)
    }

    /// Get connection status for all sockets
    pub async fn get_connection_status(&self) -> Result<net::device::ConnectionStatus> {
        self.network
            .get_connection_status(self.spi, self.config.command_timeout)
            .await
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

// T01-specific re-exports
#[cfg(feature = "mission-t01")]
pub use http::{HttpClient, HttpMethod, HttpRequest, HttpResponse};
#[cfg(feature = "mission-t01")]
pub use mqtt::{MqttClient, MqttConfig, MqttMessage};
#[cfg(feature = "mission-t01")]
pub use net::NetworkDevice;
#[cfg(feature = "mission-t01")]
pub use sync::TmMutex;
#[cfg(feature = "mission-t01")]
pub use tls::{CertificateType, TlsManager};
#[cfg(feature = "mission-t01")]
pub use wifi::WiFiManager;

// T02-specific re-exports
#[cfg(feature = "mission-t02")]
pub use net::{
    new_driver, Capabilities, Medium, PacketBuf, RxToken, SpiFrameHeader, St67w611Device,
    St67w611Runner, St67w611Transport, State, TrafficType, TxToken, MTU,
};
