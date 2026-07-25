# ST67W611 Async Driver

An async, `no_std` Rust driver for ST67W611 WiFi modules using the Embassy framework.

## Features

- **Async/Await**: Built on Embassy framework for efficient async I/O
- **No-std Compatible**: Works without heap allocation using `heapless` collections
- **Dual Firmware Support**: Works with both T01 and T02 firmware architectures
- **WiFi Station Mode**: Scan, connect, disconnect, IP configuration
- **WiFi AP Mode**: Configure and run as access point, DHCP, station management
- **TCP/UDP Sockets**: Complete socket lifecycle (T01 firmware)
- **TLS/SSL Support**: Socket-level SSL, SNI, certificate management (T01 firmware)
- **HTTP/HTTPS Client**: Full request/response handling (T01 firmware)
- **MQTT Client**: Publish/subscribe with QoS 0/1/2 support (T01 firmware)
- **embassy-net Integration**: Raw Ethernet frame transport (T02 firmware)
- **DNS Resolution**: Hostname lookups, custom DNS servers
- **SNTP Client**: Network time synchronization
- **Power Management**: Deep sleep mode with timed wake-up

## Firmware Architectures

The ST67W611 module supports two firmware architectures:

### T01 Firmware (default)

The TCP/IP stack runs **on the module**. The host communicates via AT commands for socket operations, HTTP, MQTT, etc.

```bash
cargo build --features "mission-t01,defmt" --release
```

### T02 Firmware

The TCP/IP stack runs **on the host MCU** using embassy-net. The module acts as a WiFi MAC/PHY, passing raw Ethernet frames.

```bash
cargo build --features "mission-t02,defmt" --release
```

## Quick Start (T01 Firmware)

```rust
use st67w611::bus::SpiTransportRdy;
use embassy_sync::signal::Signal;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

// Static signals for RDY flow control
static TXN_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize SPI and pins (platform-specific)
    let spi = /* your SPI setup */;
    let cs = /* your CS pin */;

    // Create SPI transport with RDY flow control
    let mut transport = SpiTransportRdy::new(spi, cs, &TXN_READY, &HDR_ACK);

    // Send AT command
    transport.write(b"AT+CWMODE=1\r\n").await.unwrap();

    // Read response
    let mut buf = [0u8; 256];
    let len = transport.read(&mut buf).await.unwrap();
}
```

## Quick Start (T02 Firmware with embassy-net)

```rust
use st67w611::net::{new_driver, State, MTU};
use embassy_net::StackResources;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize SPI and pins (platform-specific)
    let spi = /* your SPI setup */;
    let cs = /* your CS pin */;

    // Create driver state and driver
    let state = make_static!(State::<MTU, 4, 4>::new());
    let (device, runner) = new_driver(spi, cs, state);

    // Spawn the driver runner task
    spawner.spawn(wifi_runner(runner)).unwrap();

    // Create embassy-net stack
    let stack_resources = make_static!(StackResources::<3>::new());
    let (stack, stack_runner) = embassy_net::new(
        device,
        embassy_net::Config::dhcpv4(Default::default()),
        stack_resources,
        seed,
    );

    // WiFi association via AT commands, then use stack for TCP/IP
}
```

See the `examples/` directory for complete working examples.

## Examples

### Diagnostic Examples (work with both T01 and T02)

- `stm32wba55_spi_diagnostic` - SPI frame protocol diagnostics
- `stm32wba55_diagnostic_wifi_scan` - Simple WiFi scan without driver
- `stm32wba55_minimal_test` - Basic SPI communication test
- `stm32wba55_debug_rdy` - RDY pin debugging
- `stm32wba55_firmware_programmer` - Firmware programming utility

### T01 Examples

- `stm32wba55_t01_wifi_scan` - WiFi scan using SpiTransportRdy
- `stm32wba55_t01_wifi_test` - WiFi test with AT commands

### T02 Examples

- `stm32wba55_t02_embassy_net` - Embassy-net integration

### Building Examples

```bash
# Diagnostic examples (no special features required)
cargo build --example stm32wba55_spi_diagnostic --features defmt --release

# T01 examples
cargo build --example stm32wba55_t01_wifi_scan --features "mission-t01,defmt" --release

# T02 examples
cargo build --example stm32wba55_t02_embassy_net --features "mission-t02,defmt" --release
```

## Architecture

The driver is organized in layers:

1. **Bus Layer** (`bus/`): SPI transport with RDY flow control
2. **AT Command Layer** (`at/`): Command formatting and parsing
3. **Network Layer** (`net/`): Socket API (T01) or embassy-net driver (T02)
4. **Protocol Layer**: WiFi, TLS, MQTT, HTTP abstractions (T01)

### SPI Frame Protocol

Both firmware types use an 8-byte header for SPI communication:

| Offset | Field   | Description                          |
|--------|---------|--------------------------------------|
| 0-1    | Magic   | 0x55AA                               |
| 2-3    | Length  | Payload length (little-endian)       |
| 4      | Flags   | Version, RX stall indicator          |
| 5      | Type    | Traffic type (AT=0, STA=1, AP=2)     |
| 6-7    | Reserved| Must be 0                            |

## Memory Usage

The driver is designed for `no_std` environments without heap allocation:

- Fixed-size collections using `heapless`
- Static resource pools for sockets and responses
- Configurable buffer sizes
- Typical RAM usage: ~16KB (depends on configuration)

## Requirements

- Rust nightly (for Embassy)
- STM32 microcontroller with SPI interface
- ST67W611 WiFi module connected via SPI
- T01 or T02 firmware flashed on the module

## License

The contents of this repository are dual-licensed under the _MIT OR Apache 2.0_
License. That means you can choose either the MIT license or the Apache 2.0
license when you re-use this code. See [`LICENSE-MIT`](./LICENSE-MIT) or
[`LICENSE-APACHE`](./LICENSE-APACHE) for more information on each specific
license.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
