//! WiFi scan example
//!
//! This example demonstrates how to scan for available WiFi networks
//! using the ST67W611 driver.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use st67w611_driver::{
    at::processor::AtProcessor, bus::SpiTransport, Config, Driver, NetworkDevice, TlsManager,
    WiFiManager, WiFiMode,
};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize hardware (platform-specific)
    let p = embassy_stm32::init(Default::default());

    // Set up SPI
    let spi = /* Initialize your SPI peripheral here */;
    let cs = /* Initialize your CS pin here */;

    // Create static resources using make_static macro
    let spi_transport = st67w611_driver::make_static!(SpiTransport::new(spi, cs));
    let spi_mutex = st67w611_driver::make_static!(st67w611_driver::sync::TmMutex::new(
        spi_transport
    ));

    // Create AT processor
    let processor = st67w611_driver::make_static!(AtProcessor::new());

    // Create WiFi manager
    let config = Config::default();
    let wifi = st67w611_driver::make_static!(WiFiManager::new(
        processor,
        config.command_timeout
    ));

    // Create network device
    let network = st67w611_driver::make_static!(NetworkDevice::new(processor));

    // Create TLS manager
    let tls = st67w611_driver::make_static!(TlsManager::new(processor, config.command_timeout));

    // Create driver
    let driver = st67w611_driver::make_static!(Driver::new(
        spi_mutex, processor, wifi, network, tls, config
    ));

    // Spawn RX processor task
    spawner.spawn(rx_task(driver)).unwrap();

    // Wait for initialization
    Timer::after(Duration::from_secs(2)).await;

    // Initialize WiFi in station mode
    match driver.init_wifi(WiFiMode::Station).await {
        Ok(_) => defmt::info!("WiFi initialized"),
        Err(e) => {
            defmt::error!("Failed to initialize WiFi: {:?}", e);
            return;
        }
    }

    // Scan for networks
    defmt::info!("Scanning for WiFi networks...");
    match driver.wifi_scan().await {
        Ok(results) => {
            defmt::info!("Found {} networks:", results.len());
            for (i, result) in results.iter().enumerate() {
                defmt::info!(
                    "  {}. SSID: {}, RSSI: {} dBm, Channel: {}, Security: {:?}",
                    i + 1,
                    result.ssid.as_str(),
                    result.rssi,
                    result.channel,
                    result.security
                );
            }
        }
        Err(e) => {
            defmt::error!("Scan failed: {:?}", e);
        }
    }

    // Keep running
    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}
