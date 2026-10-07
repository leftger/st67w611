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
- **embassy-net (xarxa) Integration**: T02 raw-L2 driver for the full embassy-net
  stack — IPv4/IPv6, TCP/UDP, DHCP, DNS, SLAAC — with `embedded-tls` on top
- **DNS Resolution**: Hostname lookups, custom DNS servers
- **SNTP Client**: Network time synchronization
- **Power Management**: Deep sleep mode with timed wake-up

## Firmware Architectures

The ST67W611 module supports two firmware architectures.

### T01 Firmware (default)

The TCP/IP stack runs **on the module**. The host communicates via AT commands
for socket operations, HTTP, MQTT, etc. Only application data crosses SPI, so
this is the higher-throughput option.

```bash
cargo build --features "mission-t01,defmt" --release
```

### T02 Firmware

The TCP/IP stack runs **on the host MCU** using `embassy-net` (xarxa). The
module acts as a WiFi MAC/PHY, passing raw Ethernet frames. You get the whole
embassy-net stack — IPv4, IPv6, TCP, UDP, DHCP, DNS/mDNS, SLAAC, ICMP — and can
add `embedded-tls` for HTTPS.

```bash
# T02: raw-L2 driver for the xarxa-based embassy-net stack
cargo build --features "mission-t02,defmt" --release

# ... plus TLS over TCP (`tls` works on either firmware)
cargo build --features "mission-t02,tls,defmt" --release
```

> Exactly one firmware feature is active: `mission-t01` (default) or
> `mission-t02`. On T02 the module provides raw L2 frames and the host runs the
> xarxa-based `embassy-net` stack; on T01 the module owns TCP/IP and the host
> drives AT sockets. Either way the socket type is `stack::TcpSocket` and
> implements `embedded-io-async`'s `Read`/`Write`, so application code — and
> TLS — is identical.

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
use st67w611::net::xarxa::{self, State};
use embassy_net::{Stack, StackResources};
use embassy_sync::signal::Signal;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use core::sync::atomic::AtomicBool;

static READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RDY_LEVEL: AtomicBool = AtomicBool::new(false);
static STATE: static_cell::StaticCell<State<4, 4>> = static_cell::StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let spi = /* your SPI setup */;
    let cs = /* your CS pin */;

    // Driver + Wi-Fi control handle (read the MAC with AT+CIPSTAMAC? first, or
    // use a locally administered address).
    let (device, runner, control) =
        xarxa::new(spi, cs, &READY, &HDR_ACK, &RDY_LEVEL, STATE.init(State::new()), mac);

    spawner.spawn(wifi_runner(runner)).unwrap();

    let mut device = device;
    let resources = make_static!(StackResources::<3>::new());
    let (stack, stack_runner) = Stack::new(resources, seed);
    stack.add_iface_borrowed(&mut device).unwrap();
    spawner.spawn(net_runner(stack_runner)).unwrap();

    // Connect (raises the channel link state) and use the stack.
    control.connect("SSID", "password").await.unwrap();
    control.set_ipv6(true).await.unwrap();
}

#[embassy_executor::task]
async fn wifi_runner(runner: xarxa::Runner<'static, /* SPI */, /* CS */, 4, 4>) {
    runner.run().await
}
```

See the `examples/` directory for complete working examples.

## Examples

### Diagnostic examples (work with both T01 and T02)

- `stm32wba55_spi_diagnostic` - SPI frame protocol diagnostics
- `stm32wba55_diagnostic_wifi_scan` - Simple WiFi scan without driver
- `stm32wba55_minimal_test` - Basic SPI communication test
- `stm32wba55_debug_rdy` - RDY pin debugging
- `stm32wba55_firmware_programmer` - Firmware programming utility

### T01 examples

- `stm32wba55_t01_wifi_scan` - WiFi scan using SpiTransportRdy
- `stm32wba55_t01_wifi_test` - WiFi test with AT commands

### T02 examples

- `stm32wba55_t02_embassy_net` - embassy-net integration (legacy `mission-t02`)

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

1. **Bus Layer** (`bus/`): the reference full-duplex SPI engine (`engine`, `frame`) plus the original AT transports
2. **AT Command Layer** (`at/`): Command formatting and parsing
3. **Network Layer** (`net/`): T01 socket API, or the T02 xarxa driver and TLS
4. **Protocol Layer**: WiFi, TLS, MQTT, HTTP abstractions (T01)

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design and a feature-coverage
comparison against the X-CUBE-ST67W61 network driver.

### SPI Frame Protocol

Both firmware types use an 8-byte header for SPI communication:

| Offset | Field   | Description                          |
|--------|---------|--------------------------------------|
| 0-1    | Magic   | 0x55AA                               |
| 2-3    | Length  | Payload length (little-endian)       |
| 4      | Flags   | Version (2 bits), RX stall, flags    |
| 5      | Type    | Traffic type (AT=0, STA=1, AP=2)     |
| 6-7    | Reserved| Must be 0                            |

Payloads are padded to 4 bytes with `0x88`.

## Memory Usage

The driver is designed for `no_std` environments without heap allocation:

- Fixed-size collections using `heapless`
- Static resource pools for sockets and responses
- Configurable buffer sizes
- Typical RAM usage: ~16KB (T01, depends on configuration); the T02 path adds
  the xarxa packet pool and SPI staging buffers

## Requirements

- Rust nightly (for Embassy)
- STM32 microcontroller with SPI interface
- ST67W611 WiFi module connected via SPI
- T01 or T02 firmware flashed on the module

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
