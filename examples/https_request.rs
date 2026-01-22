//! HTTPS request example
//!
//! This example demonstrates how to make HTTPS requests
//! using the ST67W611 driver with TLS support.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use st67w611_driver::{
    at::processor::AtProcessor, bus::SpiTransport, Config, Driver, NetworkDevice, SocketProtocol,
    TlsManager, WiFiManager, WiFiMode,
};
use {defmt_rtt as _, panic_probe as _};

// WiFi credentials
const SSID: &str = "YourSSID";
const PASSWORD: &str = "YourPassword";

// HTTPS server
const SERVER_HOST: &str = "api.github.com";
const SERVER_PORT: u16 = 443;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize hardware (platform-specific)
    let p = embassy_stm32::init(Default::default());

    // Set up SPI and CS pin
    let spi = /* Initialize your SPI peripheral here */;
    let cs = /* Initialize your CS pin here */;

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

    // Spawn RX processor task
    spawner.spawn(rx_task(driver)).unwrap();

    // Wait for initialization
    Timer::after(Duration::from_secs(2)).await;

    // Initialize WiFi
    defmt::info!("Initializing WiFi...");
    driver
        .init_wifi(WiFiMode::Station)
        .await
        .expect("Failed to initialize WiFi");

    // Connect to WiFi
    defmt::info!("Connecting to WiFi: {}", SSID);
    driver
        .wifi_connect(SSID, PASSWORD)
        .await
        .expect("Failed to connect to WiFi");

    defmt::info!("Connected to WiFi!");

    // Get TLS manager
    let tls_manager = driver.tls_manager();

    // Allocate a socket
    defmt::info!("Allocating SSL socket...");
    let device = driver.network_device();
    let socket_id = device
        .allocate_socket(SocketProtocol::Ssl)
        .await
        .expect("Failed to allocate socket");

    defmt::info!("Socket allocated: {:?}", socket_id);

    // Configure SSL for the socket
    defmt::info!("Configuring SSL...");
    tls_manager
        .configure_socket_ssl(spi_mutex, socket_id.raw(), 1) // auth_mode: 1 = verify server
        .await
        .expect("Failed to configure SSL");

    // Set SNI hostname
    tls_manager
        .set_sni(spi_mutex, socket_id.raw(), SERVER_HOST)
        .await
        .expect("Failed to set SNI");

    defmt::info!("SSL configured");

    // Connect to HTTPS server
    defmt::info!("Connecting to {}:{}...", SERVER_HOST, SERVER_PORT);
    device
        .connect_socket(spi_mutex, socket_id, SERVER_HOST, SERVER_PORT, Duration::from_secs(15))
        .await
        .expect("Failed to connect socket");

    defmt::info!("Connected via TLS!");

    // Send HTTPS GET request
    let request = b"GET /repos/rust-lang/rust HTTP/1.1\r\nHost: api.github.com\r\nUser-Agent: ST67W611\r\nConnection: close\r\n\r\n";
    defmt::info!("Sending HTTPS request...");
    device
        .send_socket(spi_mutex, socket_id, request, Duration::from_secs(5))
        .await
        .expect("Failed to send data");

    defmt::info!("Request sent!");

    // Wait for response (in a real implementation, we'd receive and parse data here)
    Timer::after(Duration::from_secs(5)).await;

    // Close socket
    defmt::info!("Closing socket...");
    device
        .close_socket(spi_mutex, socket_id, Duration::from_secs(5))
        .await
        .expect("Failed to close socket");

    defmt::info!("Socket closed. HTTPS example complete!");

    // Keep running
    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}
