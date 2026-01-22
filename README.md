# ST67W611 Async Embassy Driver

An async, `no_std` Rust driver for ST67W611 WiFi modules using the Embassy framework.

## Features

- **Async/Await**: Built on Embassy framework for efficient async I/O
- **No-std Compatible**: Works without heap allocation using `heapless` collections
- **WiFi Management**: Station mode with scanning, connection management (AP mode not yet implemented)
- **MQTT Client**: Full publish/subscribe support with QoS levels
- **Socket Operations**: Basic TCP/UDP socket support via AT commands
- **TLS/SSL Configuration**: Socket-level SSL configuration and SNI support
- **Modular Architecture**: Clean layered design from SPI transport to high-level APIs

### Experimental/In Progress
- **Embassy-net Integration**: Skeleton implementation (needs packet translation layer)
- **HTTP Client**: Structure defined, request/response handling needs completion
- **Certificate Management**: API defined, upload/download needs implementation

## Requirements

- Rust nightly (for Embassy)
- STM32 microcontroller with SPI interface
- ST67W611 WiFi module connected via SPI

## Quick Start

```rust
use st67w611_driver::{
    at::processor::AtProcessor, bus::SpiTransport, Config, Driver,
    NetworkDevice, TlsManager, WiFiManager, WiFiMode,
};
use embassy_executor::Spawner;
use embassy_time::Duration;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize SPI and CS pin (platform-specific)
    let spi = /* your SPI setup */;
    let cs = /* your CS pin */;

    // Create static resources
    let spi_transport = st67w611_driver::make_static!(SpiTransport::new(spi, cs));
    let spi_mutex = st67w611_driver::make_static!(
        st67w611_driver::sync::TmMutex::new(spi_transport)
    );
    let processor = st67w611_driver::make_static!(AtProcessor::new());
    let config = Config::default();
    let wifi = st67w611_driver::make_static!(
        WiFiManager::new(processor, config.command_timeout)
    );
    let network = st67w611_driver::make_static!(NetworkDevice::new(processor));
    let tls = st67w611_driver::make_static!(
        TlsManager::new(processor, config.command_timeout)
    );

    let driver = st67w611_driver::make_static!(Driver::new(
        spi_mutex, processor, wifi, network, tls, config
    ));

    // Spawn RX processor task
    spawner.spawn(rx_task(driver)).unwrap();

    // Initialize WiFi
    driver.init_wifi(WiFiMode::Station).await.unwrap();

    // Connect to WiFi
    driver.wifi_connect("MySSID", "password").await.unwrap();

    // Now you can use sockets, MQTT, etc.
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}
```

See the `examples/` directory for complete working examples.

## Architecture

The driver is organized in layers:

1. **Bus Layer** (`bus/`): SPI transport using `embedded-hal-async`
2. **AT Command Layer** (`at/`): Command formatting, parsing, and RX processing
3. **Network Device Layer** (`net/`): `embassy-net` Driver implementation
4. **High-Level API Layer**: WiFi, TLS, MQTT, HTTP abstractions

### Design Note: embassy-net Integration

The ST67W611 module has a built-in TCP/IP stack accessible via AT commands (e.g., `AT+CIPSTART`, `AT+CIPSEND`). This creates a unique challenge for embassy-net integration, which expects a packet-based interface.

Current implementation wraps socket-level AT commands to provide a packet-like interface, but a future enhancement would be to investigate if the module supports a transparent/passthrough mode for raw packet access, which would provide better integration with smoltcp.

## Memory Usage

The driver is designed for `no_std` environments without heap allocation:

- Fixed-size collections using `heapless`
- Static resource pools for sockets and responses
- Configurable buffer sizes
- Typical RAM usage: ~16KB (depends on configuration)

## Status

This driver is in early development. The architecture and basic functionality are implemented, but significant work remains for production use.

### Implemented ✅
- [x] **Phase 1: Foundation & Bus Layer** - Complete SPI transport with embedded-hal-async
- [x] **Phase 2: AT Command System** - Command formatting, parsing, RX processor, event dispatcher
- [x] **Phase 3: WiFi Management** - Basic init, scan, connect, disconnect (needs enhancement for multi-result scan)
- [x] **Phase 7: MQTT Client** - Connection, publish, subscribe with QoS support

### Partially Implemented ⚠️
- [~] **Phase 4: embassy-net Driver** - Skeleton implementation, needs actual packet RX/TX translation
- [~] **Phase 5: TCP/UDP Sockets** - Basic socket operations (connect, send, close), needs receive implementation
- [~] **Phase 6: TLS/SSL Support** - Configuration and SNI support, certificate upload is placeholder
- [~] **Phase 8: HTTP Client** - Structure defined, needs actual HTTP request/response handling

### Known Limitations
- WiFi scan returns limited results (parser needs enhancement for multiple +CWLAP responses)
- Socket receive operations not fully implemented
- HTTP client is skeleton only (use raw sockets or MQTT for now)
- Certificate upload/management needs implementation
- embassy-net Driver needs packet-level translation from socket API
- Examples are illustrative but not tested on hardware
- Response matching in AT processor is simplified (needs better command/response correlation)

### Next Steps
1. Enhance WiFi scan to collect all results
2. Implement socket data reception
3. Complete HTTP client implementation
4. Test on actual hardware
5. Improve error handling and recovery
6. Add comprehensive documentation

## Examples

See the `examples/` directory for example code (note: examples are illustrative and not yet tested on hardware):

- `wifi_scan.rs` - Scan for available WiFi networks
- `tcp_client.rs` - TCP client connection and data transfer
- `mqtt_client.rs` - MQTT publish/subscribe with broker
- `https_request.rs` - HTTPS GET request with TLS

**Note**: These examples require platform-specific initialization code (SPI, GPIO setup) that you'll need to provide for your specific microcontroller.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
