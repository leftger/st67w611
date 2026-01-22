# ST67W611 Async Embassy Driver

An async, `no_std` Rust driver for ST67W611 WiFi modules using the Embassy framework.

## Features

- **Async/Await**: Built on Embassy framework for efficient async I/O
- **No-std Compatible**: Works without heap allocation using `heapless` collections
- **Embassy-net Integration**: Full TCP/IP stack integration via `embassy-net`
- **WiFi Management**: Station and AP modes, scanning, connection management
- **Secure Connections**: TLS/SSL support using module's hardware acceleration
- **Protocol Support**: TCP, UDP, MQTT, HTTP/HTTPS
- **DMA Support**: Efficient SPI transfers with DMA

## Requirements

- Rust nightly (for Embassy)
- STM32 microcontroller with SPI interface
- ST67W611 WiFi module connected via SPI

## Quick Start

```rust
use st67w611_driver::{Driver, Config};
use embassy_executor::Spawner;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize SPI peripheral
    let spi = /* your SPI setup */;

    // Create driver
    let config = Config::default();
    let mut driver = Driver::new(spi, config);

    // Initialize and connect to WiFi
    driver.init().await.unwrap();
    driver.wifi_connect("MySSID", "password").await.unwrap();

    // Create network stack
    let device = driver.into_device();
    let stack = embassy_net::Stack::new(
        device,
        embassy_net::Config::dhcpv4(Default::default()),
        make_static!(StackResources::<2>::new()),
        seed,
    );

    spawner.spawn(net_task(stack)).unwrap();

    // Use the network stack...
}
```

## Architecture

The driver is organized in layers:

1. **Bus Layer** (`bus/`): SPI transport using `embedded-hal-async`
2. **AT Command Layer** (`at/`): Command formatting, parsing, and RX processing
3. **Network Device Layer** (`net/`): `embassy-net` Driver implementation
4. **High-Level API Layer**: WiFi, TLS, MQTT, HTTP abstractions

## Memory Usage

The driver is designed for `no_std` environments without heap allocation:

- Fixed-size collections using `heapless`
- Static resource pools for sockets and responses
- Configurable buffer sizes
- Typical RAM usage: ~16KB (depends on configuration)

## Status

This driver is under active development. Current status:

- [x] Phase 1: Foundation & Bus Layer
- [x] Phase 2: AT Command System
- [ ] Phase 3: WiFi Management (in progress)
- [ ] Phase 4: embassy-net Driver
- [ ] Phase 5: TCP/UDP Sockets
- [ ] Phase 6: TLS/SSL Support
- [ ] Phase 7: MQTT Client
- [ ] Phase 8: HTTP Client

## Examples

See the `examples/` directory for complete examples:

- `wifi_scan.rs` - Scan for available WiFi networks
- `tcp_client.rs` - TCP client connection
- `mqtt_client.rs` - MQTT publish/subscribe
- `https_request.rs` - HTTPS GET request

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
