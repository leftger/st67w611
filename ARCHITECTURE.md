# ST67W611 Driver Architecture

## Overview

This document explains the architectural design of the ST67W611 async driver, key
decisions, and usage recommendations.

## Firmware architectures

The module runs one of two firmwares; the host build selects which one it is
talking to.

| | **T01** (`mission-t01`, default) | **T02** (`mission-t02`) |
|---|---|---|
| TCP/IP stack | on the module | on the host MCU |
| Host API | AT-command sockets | `embassy-net` (xarxa) over raw Ethernet frames |
| SPI traffic | application data only | every frame, header/ACK included |
| Best for | throughput, simple apps | IPv6, host TLS, existing embassy-net code |

The module advertises both as separate firmware images (`ST67W6X_CLI` and
`ST67W6X_CLI_LWIP` in the X-CUBE package).

## Layer architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Application                                                  │
├───────────────────────────┬──────────────────────────────────┤
│  T01 high-level APIs      │  T02: embassy-net ::Stack (xarxa) │
│  WiFi/HTTP/MQTT/Advanced  │  TCP · UDP · DHCP · DNS · SLAAC   │
│                           │  embedded-tls (net::tls)          │
├───────────────────────────┴──────────────────────────────────┤
│  net::xarxa ---- embassy-net-driver-channel (ch::Device)      │
│     │                        ▲ PacketBuf   │ PacketBuf        │
│     └────────────► Runner ───┘             ▼                  │
│   Control (AT) ──► │        bus::engine::Engine               │
├────────────────────┴──────────────────────────────────────────┤
│  AT layer (command/parser/processor)  ·  bus::frame codec     │
├───────────────────────────────────────────────────────────────┤
│  SPI (CS active-high, RDY EXTI)                               │
└───────────────────────────────────────────────────────────────┘
```

## Key design decisions

### 1. No-std and no allocator

`heapless` collections exclusively, static resource pools, compile-time buffer
capacities.

### 2. Async/await with Embassy

All I/O is async; background tasks drive RX and event distribution.

### 3. The SPI frame protocol (`bus::frame`, `bus::engine`)

Every transaction is a **full-duplex exchange**, mirroring `spi_xfer_one()` in
`Driver/W61_bus/spi_iface.c`:

* 8-byte header: `magic(0x55AA) · len(u16) · version:2|rx_stall:1|flags:5 · type · rsvd`.
* The host asserts CS (active **high**), clocks `HEADER_LEN + align4(payload)`
  bytes, and simultaneously reads the module's own header.
* If the module announced a longer frame than was clocked, a second read-only
  transfer fetches the remainder, still inside the same CS.
* Payloads are padded to 4 bytes with `0x88`.
* When the module reports `rx_stall`, the host must stop attaching payloads
  until the bit clears; the engine tracks this and reports `tx_deferred`.
* Traffic types (`AtCommand`, `NetworkSta`, `NetworkAp`, `Hci`, `OpenThread`)
  let AT control and raw L2 data share one physical link.

Timeouts follow the reference: 2000 ms RDY, 500 ms transfer, 100 ms header-ack
(lenient). `MAX_PAYLOAD` is 1520 (`W61_MAX_SPI_XFER`).

The older `bus::spi` transport (split read/write, CS active-low) is kept only
for the legacy T01 path; it does not model the exchange and should not be used
for new code.

### 4. Event-driven RX

The AT layer routes unsolicited events (Wi-Fi state, socket events, `+IPD`)
through channels. The T02 runner routes L2 frames by traffic type.

## T02: the embassy-net (xarxa) driver

`embassy-net` 0.9 is a façade over the [xarxa](https://github.com/embassy-rs/xarxa)
stack; the old packet-token `embassy-net-driver` trait is gone. A driver now
implements `xarxa_driver::Driver` — or reuses `embassy-net-driver-channel`,
which is what `cyw43` and `enc28j60` do, and what `net::xarxa` does here.

`net::xarxa::new()` returns three handles:

* **`WifiDevice`** (`ch::Device`) — hand it to the stack with
  `Stack::add_iface_borrowed(&mut device)`.
* **`Runner`** — owns the SPI `Engine`; spawn `Runner::run()`. It multiplexes,
  via `select3`, three wakeups: AT requests, outbound `PacketBuf`s from the
  stack, and the module's RDY line. Received `NetworkSta`/`NetworkAp` frames
  are copied into `PacketBuf`s and pushed to the channel; `AtCommand` frames
  feed the line parser.
* **`Control`** — `at()`, `connect()`, `disconnect()`, `scan()`, `set_ipv6()`.
  `connect()` raises the channel link state, which is what lets the stack start
  using the interface (DHCP, etc.).

The global xarxa packet pool holds the buffers, so the channel queues cost a
handful of bytes per slot, not a whole frame each.

### IPv6

`ipv6` is enabled on `embassy-net` in `Cargo.toml`, so the stack speaks IPv6
(NDISC, SLAAC, ICMPv6) out of the box. `Control::set_ipv6(true)` issues
`AT+CIPV6=1` to enable it on the module link.

### TLS (`tls` feature)

`embedded-tls` 0.19 implements `embedded-io-async` 0.7, which
`embassy_net::tcp::TcpSocket` also implements, so `net::tls::client()` wraps a
connected socket directly. Certificate verification comes from embedded-tls's
`rustpki` (no_std) or `webpki` (std) provider; without one, `UnsecureProvider`
performs no verification and is for bring-up only.

### Throughput caveat

The SPI link caps out at roughly 1–2 MB/s at 30 MHz. In T02 every header, ACK
and retransmission crosses that link, so T02 trades throughput for stack
features. T01 keeps protocol processing on the module and moves only
application data, which is why it remains the default.

## X-CUBE-ST67W61 coverage

Comparison against the `W6X` network driver v1.3.0 (April 2026). "AT" means the
command builders exist and are unit-tested; a high-level typed driver may still
be missing. "—" means not implemented in this crate yet.

| Area | X-CUBE | This crate |
|---|---|---|
| Bus / SPI protocol | full-duplex engine | ✅ `bus::engine` |
| Typed clients (both firmwares) | — | ✅ `ble::Ble`, `fwu::Fwu`, `wifi::WiFi`, `net::client::Net` |
| Wi-Fi station + Soft-AP | ✅ | ✅ (`wifi`, typed `wifi::WiFi`) |
| Wi-Fi credentials store (`AT+CWCRED*`) | ✅ | AT + typed (`wifi::WiFi`) |
| TWT / DTIM / antenna diversity | ✅ | AT + typed (`wifi::WiFi`) |
| Country code, auto-connect, WPS | ✅ | AT + typed (`wifi::WiFi`) |
| Sockets (T01) TCP/UDP/SSL | ✅ | ✅ (`net::NetworkDevice`) |
| IPv6 sockets (T01) | ✅ | AT `AT+CIPV6` + `net::client::Net` |
| Server sockets / `CIPRECVMODE` | ✅ | AT + typed (`net::client::Net`) |
| HTTP client | GET/HEAD/POST/PUT | GET/POST/PUT/HEAD/DELETE (`http`) |
| MQTT | cfg/conn/SNI/raw/LWT | ✅ typed (`mqtt::Mqtt`), incl. Last Will (`AT+MQTTCONNCFG`) |
| T01 AT socket stack | module-side TCP/IP | ✅ `net::NetworkDevice` + `http` + `tls` (the T01 data path) |
| DNS (v4 + v6) / SNTP / ping | ✅ | ✅ typed (`net::client::Net`: `resolve`, `sntp_time_string`, `ping_rtt`) |
| Host-side stack (T02 netif) | ✅ (LwIP) | ✅ (`net::xarxa`, xarxa-based) |
| TLS | on module | ✅ via `embedded-tls` (`net::tls`) |
| Firmware update / FOTA | ✅ (`AT+OTASTART/SEND/FIN`) | ✅ typed driver (`fwu::Fwu`) |
| System / filesystem (`AT+FS`) | ✅ | AT (`at::command::{system,filesystem}`) |
| BLE (adv/scan/GATT/security) | ✅ | ✅ commands (`at::ble`) + typed driver (`ble::Ble`) |
| Shell / iperf / wfa-tg | ✅ | — |

The X-CUBE surface is enumerated in
`Middlewares/ST/ST67W6X_Network_Driver/Api/w6x_api.h` and documented in its
`Doc/README.md`.

## Memory layout (T01 defaults)

| Buffer | Default | Configurable |
|---|---|---|
| SPI RX | 4096 B | via `Config` |
| Socket RX | 2048 B × 8 | via `Config` |
| Multi-response | 512 B × 32 | fixed |
| Wi-Fi events | 4 slots | fixed |
| Socket events | 16 slots | fixed |
| IPD data | 2048 B × 4 | fixed |

T02 adds the `Engine` staging buffers (2 × 1528 B), a 1528 B frame buffer in the
runner, and the global xarxa packet pool (default 16 × ~1516 B, tunable through
xarxa's `packet-buf-count-*` / `packet-buf-*` features).

## Concurrency model

* `CriticalSectionRawMutex` for sync primitives.
* Atomic link state.
* All waits are async; no busy-waiting.
* One task owns the SPI engine (`AtProcessor::rx_task` in T01, `xarxa::Runner`
  in T02), which serialises access to the bus.

## API usage patterns

### T01 Wi-Fi

```rust
driver.init_wifi(WiFiMode::Station).await?;
let results = driver.wifi_scan().await?;
driver.wifi_connect("SSID", "password").await?;
let ip_config = driver.get_ip_config().await?;
```

### T01 socket / HTTP / MQTT

```rust
let device = driver.network_device();
let socket = device.allocate_socket(SocketProtocol::Tcp).await?;
device.connect_socket(spi, socket, "example.com", 80, timeout).await?;
device.send_socket(spi, socket, data, timeout).await?;

let http = driver.http_client();
let response = http.get(spi, "https://api.example.com/data").await?;

let mqtt = driver.mqtt_client(0);
mqtt.subscribe(spi, "topic/test", MqttQos::AtLeastOnce).await?;
```

### T02 embassy-net

```rust
let (device, runner, control) = xarxa::new(spi, cs, &READY, &HDR_ACK, &RDY_LEVEL, state, mac);
spawner.spawn(wifi_task(runner)).unwrap();

let mut device = device;
let (stack, stack_runner) = embassy_net::Stack::new(resources, seed);
stack.add_iface_borrowed(&mut device)?;
spawner.spawn(net_task(stack_runner)).unwrap();

control.connect("SSID", "password").await?;   // raises link state
control.set_ipv6(true).await?;
```

## Porting guide

Embassy replacements for the FreeRTOS primitives used by the C driver:

| FreeRTOS | Embassy |
|---|---|
| `xQueueCreate` | `Channel::new()` |
| `xSemaphoreCreateBinary` | `Signal::new()` |
| `xTaskCreate` | `spawner.spawn()` |
| `vTaskDelay` | `Timer::after().await` |

The AT-command API maps closely (`w6x_wifi_connect` → `Control::connect`), but
the C driver's per-call callbacks and blocking waits become `async fn`s.

## Contributing

Maintain:

* No-std compatibility, no heap allocation.
* Async APIs.
* Reference-faithful bus behaviour (`bus::engine` is the source of truth).
* Tests for parsing and frame logic (`cargo test --lib` runs them on the host).
