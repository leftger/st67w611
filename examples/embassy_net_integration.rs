//! embassy-net integration example
//!
//! IMPORTANT: This example demonstrates embassy-net integration, but please read the
//! limitations below before using this approach in production.
//!
//! ## Architectural Limitation
//!
//! The ST67W611 module has a built-in TCP/IP stack accessed via AT commands, while
//! embassy-net expects raw packet-level access for its smoltcp-based stack. This creates
//! an architectural mismatch.
//!
//! ## Recommended Approach
//!
//! For most applications, use the driver's socket APIs directly instead of embassy-net:
//! - `NetworkDevice` for socket operations
//! - `HttpClient` for HTTP/HTTPS
//! - `MqttClient` for MQTT
//!
//! These work directly with the module's proven TCP/IP stack.
//!
//! ## When to Use embassy-net
//!
//! Only use embassy-net integration if you:
//! - Need compatibility with existing embassy-net libraries
//! - Are willing to implement/test packet bridging logic
//! - Understand the performance and compatibility trade-offs

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_net::{Config as NetConfig, Stack, StackResources};
use embassy_time::{Duration, Timer};
use st67w611_driver::{
    at::processor::AtProcessor, bus::SpiTransport, Config, Driver, MacAddress, NetworkDevice,
    St67w611Driver, TlsManager, WiFiManager, WiFiMode,
};
use {defmt_rtt as _, panic_probe as _};

const SSID: &str = "YourSSID";
const PASSWORD: &str = "YourPassword";

type MyDevice = St67w611Driver;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize hardware
    let p = embassy_stm32::init(Default::default());

    // Set up SPI
    let spi = /* Initialize your SPI peripheral */;
    let cs = /* Initialize your CS pin */;

    // Create static resources
    let spi_transport = st67w611_driver::make_static!(SpiTransport::new(spi, cs));
    let spi_mutex = st67w611_driver::make_static!(st67w611_driver::sync::TmMutex::new(
        spi_transport
    ));

    let processor = st67w611_driver::make_static!(AtProcessor::new());
    let config = Config::default();
    let wifi = st67w611_driver::make_static!(WiFiManager::new(
        processor,
        config.command_timeout
    ));
    let network = st67w611_driver::make_static!(NetworkDevice::new(processor));
    let tls = st67w611_driver::make_static!(TlsManager::new(processor, config.command_timeout));

    let driver = st67w611_driver::make_static!(Driver::new(
        spi_mutex, processor, wifi, network, tls, config
    ));

    // Spawn background tasks
    spawner.spawn(rx_task(driver)).unwrap();
    spawner.spawn(ipd_task(driver)).unwrap();

    // Wait for initialization
    Timer::after(Duration::from_secs(2)).await;

    // Initialize WiFi
    defmt::info!("Initializing WiFi...");
    driver.init_wifi(WiFiMode::Station).await.unwrap();

    // Connect to WiFi
    defmt::info!("Connecting to WiFi: {}", SSID);
    driver.wifi_connect(SSID, PASSWORD).await.unwrap();

    // Get MAC address
    let mac = driver.get_mac().await.unwrap();
    defmt::info!("MAC Address: {:?}", mac);

    // Create embassy-net driver
    let net_driver = st67w611_driver::make_static!(driver.create_embassy_net_driver(mac));

    // Create embassy-net stack
    let net_config = NetConfig::dhcpv4(Default::default());
    let seed = 0x1234; // Random seed for TCP port selection

    let stack = &*st67w611_driver::make_static!(Stack::new(
        net_driver,
        net_config,
        st67w611_driver::make_static!(StackResources::<2>::new()),
        seed
    ));

    // Spawn network stack task
    spawner.spawn(net_task(stack)).unwrap();

    // Wait for network stack
    defmt::info!("Waiting for network stack to initialize...");
    Timer::after(Duration::from_secs(5)).await;

    // Note: At this point, the embassy-net stack is running, but actual packet
    // bridging would need to be implemented for full functionality.
    //
    // For a working example, see tcp_client.rs or https_request.rs which use
    // the socket APIs directly.

    defmt::info!("Embassy-net stack initialized");
    defmt::info!("Note: For functional networking, use the socket APIs directly (see other examples)");

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}

#[embassy_executor::task]
async fn ipd_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_ipd_task().await;
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<MyDevice>) {
    stack.run().await
}
