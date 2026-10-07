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
//! ## Firmware selection
//!
//! Exactly one firmware feature is active, and it picks where TCP/IP lives:
//!
//! | | T01 (`mission-t01`, default) | T02 (`mission-t02`) |
//! |---|---|---|
//! | TCP/IP | on the module | on the host (`embassy-net`) |
//! | Sockets | AT sockets via `net::NetworkDevice` | `embassy-net` sockets via `net::xarxa` |
//! | Enter via | [`Driver`] | `driver_t02::Driver` (also exported as [`Driver`]) |
//!
//! Everything above the socket layer is **identical** on both firmwares: the
//! same clients, reached the same way.
//!
//! ```ignore
//! // Both firmwares: one driver, one set of accessors.
//! let wifi  = driver.wifi();   // mode, scan, credentials, TWT, Soft-AP
//! let net   = driver.net();    // IPv6, DNS (typed), SNTP, ping, TCP server
//! let ble   = driver.ble();
//! let fwu   = driver.fwu();
//! let mqtt  = driver.mqtt();
//! ```
//!
//! Sockets are unified too: [`stack::TcpSocket`] resolves to the module's
//! AT-socket adapter on T01 and to `embassy_net::tcp::TcpSocket` on T02, and in
//! both cases implements `embedded-io-async`'s `Read`/`Write` — so application
//! code and [`net::tls`] are firmware-independent.
//!
//! # Example (T01 Firmware)
//!
//! ```no_run,ignore
//! use st67w611::{
//!     at::processor::AtProcessor, bus::SpiTransport, Config, Driver,
//!     NetworkDevice, TlsManager, WiFiMode, SocketProtocol,
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
pub mod ble;
pub mod bus;
pub mod config;
pub mod error;
pub mod fwu;
pub mod mqtt;
pub mod net;
pub mod stack;
pub mod sync;
pub mod types;
pub mod util;
pub mod wifi;

// T01-specific modules (AT command-based networking)
#[cfg(feature = "mission-t01")]
pub mod http;
#[cfg(feature = "mission-t01")]
pub mod power;

// T02-specific module: the driver facade mirroring the T01 `Driver`
#[cfg(feature = "mission-t02")]
pub mod driver_t02;

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
/// On T02 the equivalent is `crate::Driver` from [`crate::driver_t02`], which
/// exposes the same `wifi()`/`net()`/`ble()`/`fwu()`/`mqtt()` accessors over
/// the raw-L2 [`net::xarxa`] link.
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
    /// Network device
    network: &'static net::NetworkDevice,
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
    ///
    /// TLS is configured through [`Driver::net`] — `configure_ssl`, `set_sni`,
    /// `set_ssl_psk`, `set_ssl_alpn`, `upload_certificate`, …
    pub fn new(
        spi: &'static sync::TmMutex<SpiTransport<SPI, CS>>,
        processor: &'static AtProcessor,
        network: &'static net::NetworkDevice,
        config: Config,
    ) -> Self {
        Self {
            spi,
            processor,
            network,
            config,
        }
    }

    /// The transport-generic Wi-Fi client over this driver's AT link.
    ///
    /// Replaces the old `wifi_scan` / `get_ip_config` / `get_mac` /
    /// `get_wifi_state` conveniences. On T02, pass
    /// [`crate::net::xarxa::Control`] to [`wifi::WiFi::new`] instead.
    pub fn wifi(&self) -> wifi::WiFi<crate::at::transport::ProcessorTransport<'_, SPI, CS>> {
        wifi::WiFi::new(crate::at::transport::ProcessorTransport::new(
            self.processor,
            self.spi,
            self.config.command_timeout,
        ))
    }

    /// Wait for the next unsolicited Wi-Fi event (`WIFI GOT IP`, …).
    ///
    /// These are pushed by the module, so they cannot arrive through the
    /// request/response AT transport.
    pub async fn next_wifi_event(&self) -> at::processor::WiFiEvent {
        self.processor.wifi_event_receiver().receive().await
    }

    /// Initialize the Wi-Fi subsystem.
    pub async fn init_wifi(&self, mode: WiFiMode) -> Result<()> {
        self.wifi().set_mode(mode).await
    }

    /// Connect to a WiFi access point
    pub async fn wifi_connect(&self, ssid: &str, password: &str) -> Result<()> {
        let result = self.wifi().connect(ssid, password).await;

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
        let result = self.wifi().disconnect().await;

        // Update network device link state
        self.network.set_link_state(false);

        result
    }

    /// Get the network device for direct socket operations
    pub fn network_device(&self) -> &NetworkDevice {
        self.network
    }

    /// Create an MQTT client over this driver's AT link.
    ///
    /// On T02, pass [`crate::net::xarxa::Control`] to
    /// [`crate::mqtt::Mqtt::new`] instead.
    pub fn mqtt(&self) -> mqtt::Mqtt<crate::at::transport::ProcessorTransport<'_, SPI, CS>> {
        mqtt::Mqtt::new(crate::at::transport::ProcessorTransport::new(
            self.processor,
            self.spi,
            self.config.command_timeout,
        ))
    }

    /// Create an HTTP client
    pub fn http_client(&self) -> http::HttpClient {
        http::HttpClient::new(self.network, self.processor, self.config.command_timeout)
    }

    /// Create a BLE client over this driver's AT link.
    ///
    /// The returned [`crate::ble::Ble`] is generic over
    /// [`crate::at::AtTransport`]; this uses the T01 [`AtProcessor`]-based
    /// adapter. (On T02, pass [`crate::net::xarxa::Control`] to
    /// [`crate::ble::Ble::new`] instead.)
    pub fn ble(
        &self,
    ) -> crate::ble::Ble<crate::at::transport::ProcessorTransport<'_, SPI, CS>> {
        crate::ble::Ble::new(crate::at::transport::ProcessorTransport::new(
            self.processor,
            self.spi,
            self.config.command_timeout,
        ))
    }

    /// Create a firmware-update client over this driver's AT link.
    ///
    /// On T02, pass [`crate::net::xarxa::Control`] to
    /// [`crate::fwu::Fwu::new`] instead.
    pub fn fwu(
        &self,
    ) -> crate::fwu::Fwu<crate::at::transport::ProcessorTransport<'_, SPI, CS>> {
        crate::fwu::Fwu::new(crate::at::transport::ProcessorTransport::new(
            self.processor,
            self.spi,
            self.config.command_timeout,
        ))
    }

    /// Create a network configuration/services client over this driver's AT link.
    ///
    /// Covers DNS (including typed resolution), SNTP, ping, interface options
    /// and TCP servers. On T02, pass [`crate::net::xarxa::Control`] to
    /// [`crate::net::client::Net::new`] instead.
    pub fn net(
        &self,
    ) -> crate::net::client::Net<crate::at::transport::ProcessorTransport<'_, SPI, CS>> {
        crate::net::client::Net::new(crate::at::transport::ProcessorTransport::new(
            self.processor,
            self.spi,
            self.config.command_timeout,
        ))
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
pub use mqtt::{Mqtt, MqttConfig, MqttMessage};
#[cfg(feature = "mission-t01")]
pub use net::NetworkDevice;
#[cfg(feature = "mission-t01")]
pub use sync::TmMutex;
pub use net::client::CertificateType;

// T02-specific re-exports
#[cfg(feature = "mission-t02")]
pub use driver_t02::Driver;
#[cfg(feature = "mission-t02")]
pub use net::{Control, XarxaRunner, XarxaState, WifiDevice};
