//! MQTT client example
//!
//! This example demonstrates how to connect to an MQTT broker
//! and publish/subscribe to topics.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use heapless::String;
use st67w611_driver::{
    at::processor::AtProcessor, bus::SpiTransport, mqtt::MqttConfig, Config, Driver, MqttQos,
    NetworkDevice, TlsManager, WiFiManager, WiFiMode,
};
use {defmt_rtt as _, panic_probe as _};

// WiFi credentials
const SSID: &str = "YourSSID";
const PASSWORD: &str = "YourPassword";

// MQTT broker settings
const MQTT_BROKER: &str = "broker.hivemq.com";
const MQTT_PORT: u16 = 1883;
const MQTT_CLIENT_ID: &str = "st67w611_test_client";
const MQTT_TOPIC: &str = "st67w611/test";

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

    // Create MQTT client
    let mqtt_client = driver.mqtt_client(0); // Use link ID 0

    // Configure MQTT
    let mut mqtt_config = MqttConfig::default();
    let mut client_id = String::new();
    client_id.push_str(MQTT_CLIENT_ID).unwrap();
    mqtt_config.client_id = client_id;

    // Connect to MQTT broker
    defmt::info!("Connecting to MQTT broker: {}:{}", MQTT_BROKER, MQTT_PORT);
    mqtt_client
        .connect(spi_mutex, MQTT_BROKER, MQTT_PORT, &mqtt_config)
        .await
        .expect("Failed to connect to MQTT broker");

    defmt::info!("Connected to MQTT broker!");

    // Subscribe to a topic
    defmt::info!("Subscribing to topic: {}", MQTT_TOPIC);
    mqtt_client
        .subscribe(spi_mutex, MQTT_TOPIC, MqttQos::AtLeastOnce)
        .await
        .expect("Failed to subscribe");

    defmt::info!("Subscribed!");

    // Publish messages periodically
    let mut counter = 0u32;
    loop {
        // Create message
        let mut message = String::<64>::new();
        use core::fmt::Write;
        write!(&mut message, "Hello from ST67W611! Count: {}", counter).unwrap();

        // Publish
        defmt::info!("Publishing: {}", message.as_str());
        match mqtt_client
            .publish(spi_mutex, MQTT_TOPIC, message.as_str(), MqttQos::AtLeastOnce, false)
            .await
        {
            Ok(_) => defmt::info!("Message published successfully"),
            Err(e) => defmt::error!("Failed to publish: {:?}", e),
        }

        counter += 1;

        // Wait before next publish
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}
