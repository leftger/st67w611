# ST67W611 Async Driver

An async, `no_std` Rust driver for ST67W611 WiFi modules using the Embassy framework.

**Note**: This driver uses the module's built-in TCP/IP stack via AT commands. It does NOT support embassy-net due to SPI bandwidth limitations (see Architecture section below).

## Features

- **Async/Await**: Built on Embassy framework for efficient async I/O
- **No-std Compatible**: Works without heap allocation using `heapless` collections
- **WiFi Station Mode**: Scan, connect, disconnect, IP configuration
- **WiFi AP Mode**: Configure and run as access point, DHCP, station management
- **TCP/UDP Sockets**: Complete socket lifecycle (allocate, connect, send, receive, close)
- **TLS/SSL Support**: Socket-level SSL, SNI, certificate upload/download via filesystem
- **HTTP/HTTPS Client**: Full request/response handling with URL parsing
- **MQTT Client**: Publish/subscribe with QoS 0/1/2 support
- **DNS Resolution**: Hostname lookups, custom DNS servers
- **SNTP Client**: Network time synchronization with timezone support
- **Network Diagnostics**: Ping utility with RTT measurement
- **Power Management**: Deep sleep mode with timed wake-up
- **Error Handling**: Automatic retry with exponential backoff
- **Modular Architecture**: Clean layered design from SPI transport to high-level protocols

### NOT Supported

- ❌ **embassy-net**: The module's 30MHz SPI limit makes transparent packet mode impractical. The built-in TCP/IP stack is the correct architecture for this hardware. See `ARCHITECTURE.md` for detailed technical explanation.

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

    // Spawn RX processor task (required)
    spawner.spawn(rx_task(driver)).unwrap();

    // Spawn IPD processor task (handles incoming socket data)
    spawner.spawn(ipd_task(driver)).unwrap();

    // Initialize WiFi
    driver.init_wifi(WiFiMode::Station).await.unwrap();

    // Connect to WiFi
    driver.wifi_connect("MySSID", "password").await.unwrap();

    // Now you can use sockets, HTTP, MQTT, etc.
    let http = driver.http_client();
    let response = http.get(spi_mutex, "https://api.example.com/data").await.unwrap();
}

#[embassy_executor::task]
async fn rx_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_rx_task().await;
}

#[embassy_executor::task]
async fn ipd_task(driver: &'static Driver<impl embedded_hal_async::spi::SpiDevice, impl embedded_hal::digital::OutputPin>) {
    driver.run_ipd_task().await;
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

The ST67W611 module has a built-in TCP/IP stack accessible via AT commands (e.g., `AT+CIPSTART`, `AT+CIPSEND`). While this driver implements the `embassy_net::driver::Driver` trait, there's an important architectural consideration:

**Why Socket APIs Are Recommended:**
- The module's SPI interface has a 30MHz maximum clock (~3.75 MB/s theoretical, 1-2 MB/s practical)
- WiFi provides 10-100+ Mbps throughput
- **SPI bandwidth is the bottleneck**, not WiFi
- The module's built-in TCP/IP stack processes protocols locally, minimizing SPI traffic
- Transparent packet mode would saturate SPI with protocol overhead

**This is the correct architecture for this hardware.** The driver provides comprehensive socket APIs that work efficiently with the module's design:
- `NetworkDevice` for TCP/UDP sockets
- `HttpClient` / `MqttClient` for protocols
- `DnsResolver`, `SntpClient`, `Ping` for network utilities

The embassy-net Driver implementation is provided for compatibility but **direct socket APIs are recommended** for production use. See `ARCHITECTURE.md` for detailed analysis.

## Memory Usage

The driver is designed for `no_std` environments without heap allocation:

- Fixed-size collections using `heapless`
- Static resource pools for sockets and responses
- Configurable buffer sizes
- Typical RAM usage: ~16KB (depends on configuration)

## Status

This driver has completed most core functionality but still needs hardware testing and refinement.

### Fully Implemented ✅
- [x] **Phase 1: Foundation & Bus Layer** - Complete SPI transport with embedded-hal-async
- [x] **Phase 2: AT Command System** - Command formatting, parsing, RX processor with multi-response support
- [x] **Phase 3: WiFi Management** - Station mode (init, scan, connect, disconnect, IP config) + AP mode (configure, start, list stations)
- [x] **Phase 5: TCP/UDP Sockets** - Complete socket lifecycle with send/receive and +IPD binary data handling
- [x] **Phase 6: TLS/SSL Support** - SSL configuration, SNI, certificate upload/download via filesystem
- [x] **Phase 7: MQTT Client** - Connection, publish, subscribe with QoS support
- [x] **Phase 8: HTTP Client** - Full HTTP/HTTPS client with URL parsing, request/response handling
- [x] **Phase 9: Advanced Features** - DNS resolution, SNTP time sync, Ping, Power management, WiFi AP mode

### NOT Implemented ❌
- [ ] **Phase 4: embassy-net Driver** - NOT SUPPORTED. The module's 30MHz SPI limit makes transparent packet mode impractical and inefficient. The built-in TCP/IP stack accessed via socket APIs is the correct architecture. See `ARCHITECTURE.md` and `net/driver.rs` for detailed technical explanation.

### Recent Improvements (Latest Sessions)

**Session 3 (Architecture clarity & final features):**
- ✅ Comprehensive ARCHITECTURE.md explaining why embassy-net is not supported
- ✅ Technical analysis: 30MHz SPI bandwidth limitation vs WiFi throughput
- ✅ Documentation clarifying socket APIs are the correct approach
- ✅ Certificate upload/download via filesystem (AT+FS commands)
- ✅ Filesystem operations module (write, read, delete, list files)
- ✅ Connection status monitoring (AT+CIPSTATUS parsing)
- ✅ Power management module with deep sleep support (AT+GSLP)
- ✅ Utility module with retry logic (exponential backoff, fixed delay)
- ✅ WiFi connection with automatic retry wrapper
- ✅ Removed misleading embassy-net scaffolding per hardware constraints

**Session 2:**
- ✅ +IPD unsolicited data reception with binary data handling
- ✅ Background IPD processor task for automatic socket buffer filling
- ✅ System configuration commands (AT+SYSSTORE, AT+RESTORE, AT+UART, etc.)
- ✅ DNS resolution API with custom DNS server configuration
- ✅ SNTP time synchronization client
- ✅ Ping utility for network diagnostics
- ✅ Complete WiFi AP mode support (configure, start, list connected stations)
- ✅ DHCP configuration for both station and AP modes
- ✅ New advanced networking module with DNS, SNTP, and Ping

**Session 1:**
- ✅ Multi-response command support for collecting multiple AT responses (scan results, IP config)
- ✅ WiFi scan now collects all available networks, not just one
- ✅ Socket receive operations implemented with buffered data management
- ✅ HTTP client fully functional with URL parsing, request formatting, and response parsing
- ✅ IP configuration query now returns complete ip/gateway/netmask information
- ✅ Improved AT processor response routing and handling

### Known Limitations & Notes
- **embassy-net**: NOT SUPPORTED due to 30MHz SPI bandwidth constraint. Module's built-in TCP/IP stack is the correct architecture. See `ARCHITECTURE.md` for technical analysis.
- **Socket receive via AT+CIPRECV**: Command implemented but binary data extraction needs enhancement (use +IPD auto-receive for now)
- **Examples**: Illustrative code, not tested on actual hardware yet
- **Certificate upload**: Implemented via AT+FS, response handling could be more robust
- **Hardware dependencies**: Examples need platform-specific SPI/GPIO initialization

### Next Steps
1. **Hardware Testing**: Test all features on STM32 with actual ST67W611 module
2. **AT+CIPRECV Enhancement**: Improve binary data extraction from receive responses
3. **More Examples**: Add examples for DNS, SNTP, AP mode, power management
4. **Performance Tuning**: Optimize buffer sizes and polling intervals based on real-world usage
5. **API Documentation**: Add comprehensive rustdoc for all public functions
6. **CI/CD**: Set up automated testing and release workflow

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
