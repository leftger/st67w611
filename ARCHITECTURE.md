# ST67W611 Driver Architecture

## Overview

This document explains the architectural design of the ST67W611 async driver, key decisions, and usage recommendations.

## Layer Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  High-Level APIs                                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │
│  │  WiFi    │  │   HTTP   │  │   MQTT   │  │ Advanced │   │
│  │ Manager  │  │  Client  │  │  Client  │  │   (DNS)  │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│  Network Device Layer                                       │
│  ┌──────────────────┐         ┌──────────────────┐         │
│  │  NetworkDevice   │         │ St67w611Driver   │         │
│  │ (Socket Mgmt)    │         │ (embassy-net)    │         │
│  └──────────────────┘         └──────────────────┘         │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│  AT Command Layer                                           │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                 │
│  │ Command  │  │  Parser  │  │   RX     │                 │
│  │ Builder  │  │          │  │Processor │                 │
│  └──────────┘  └──────────┘  └──────────┘                 │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│  Bus Layer (SPI Transport)                                  │
│  ┌──────────────────────────────────────────┐              │
│  │  SpiTransport (embedded-hal-async)       │              │
│  └──────────────────────────────────────────┘              │
└─────────────────────────────────────────────────────────────┘
                              │
                         Hardware SPI
```

## Key Design Decisions

### 1. No-std and No Allocator

**Decision**: Use `heapless` collections exclusively, no heap allocation.

**Rationale**:
- Embedded systems often lack heap allocators
- Predictable memory usage
- No allocation failures at runtime
- Better for safety-critical applications

**Implementation**:
- Fixed-size `heapless::Vec` and `heapless::String` throughout
- Static resource pools (response slots, sockets)
- Compile-time capacity configuration

### 2. Async/Await with Embassy

**Decision**: Built on Embassy framework, async throughout.

**Rationale**:
- Efficient cooperative multitasking without OS overhead
- Natural API for I/O operations
- Better than blocking or callback-based approaches
- Growing ecosystem in embedded Rust

**Implementation**:
- All I/O operations are async
- Uses `embassy-sync` primitives (Mutex, Signal, Channel)
- Background tasks for RX processing and data routing

### 3. Multi-Response Command Support

**Decision**: Separate handling for commands that return multiple data lines.

**Rationale**:
- WiFi scan returns multiple `+CWLAP` lines
- IP config query returns multiple `+CIPSTA` lines
- Need to collect all responses before returning to caller

**Implementation**:
- `ResponseSlot` has both a signal (for single response) and channel (for multiple)
- `send_multi_response_command()` enables collection mode
- Responses collected until final `OK` or `ERROR`

### 4. Event-Driven Architecture

**Decision**: Use channels for event distribution (WiFi events, socket events, IPD data).

**Rationale**:
- Decouples RX processing from event consumers
- Non-blocking event delivery
- Multiple consumers can wait on events

**Implementation**:
- WiFi events: Connection, disconnection, got IP
- Socket events: Connected, closed, data available
- IPD data: Actual received socket data bytes

## embassy-net Integration Challenge

### The Problem

**ST67W611 Architecture:**
```
Application → AT Commands → Module's TCP/IP Stack → WiFi → Network
              (AT+CIPSTART)   (Built-in)
```

**embassy-net Architecture:**
```
Application → smoltcp Stack → Driver (packets) → WiFi → Network
              (TCP/IP in MCU)   (Raw packets)
```

**Mismatch**:
- ST67W611 expects socket-level commands (connect, send, receive)
- embassy-net expects packet-level interface (Ethernet/IP frames)
- The module's TCP/IP stack and smoltcp would conflict

### Current Implementation

The `St67w611Driver` implements the `embassy_net::driver::Driver` trait with:

- ✅ **Link state tracking**: Works correctly, reflects WiFi connection status
- ✅ **Packet buffer infrastructure**: RX/TX queues for packet storage
- ✅ **Token-based interface**: Proper RxToken and TxToken implementations
- ⚠️ **Packet bridging**: Infrastructure present but requires custom logic

### Why Transparent Mode Isn't Feasible

**Hardware Limitation**: The ST67W611 has a maximum SPI clock of 30MHz, which provides:
- Theoretical max throughput: ~3.75 MB/s
- Practical throughput: ~1-2 MB/s (accounting for protocol overhead)

**Why this matters**:
- Raw packet forwarding would consume most/all available SPI bandwidth
- WiFi can provide 10-100+ Mbps, but SPI becomes the bottleneck
- Module's built-in TCP/IP stack is actually the **correct architecture**
- The stack handles protocol processing locally, only sending/receiving application data over SPI
- This dramatically reduces SPI traffic and improves efficiency

**Conclusion**: Transparent/packet mode is not just unimplemented—it's architecturally inappropriate for this hardware. The socket-based AT command interface is the right design.

### Socket API Approach (Correct Design)

**Architecture:**
```
Application → Socket APIs → Module TCP/IP Stack → WiFi (100+ Mbps)
              (Efficient)   (Local processing)

SPI transfers: Only application data + AT commands
Bandwidth usage: Minimal, scales with actual data
```

**Implementation Status:**
- ✅ NetworkDevice for raw socket operations
- ✅ HttpClient for HTTP/HTTPS requests
- ✅ MqttClient for pub/sub messaging
- ✅ TLS with certificate management
- ✅ DNS, SNTP, Ping utilities

### embassy-net Status

**Infrastructure provided:**
- ✅ Complete Driver trait implementation
- ✅ Packet buffers and queues
- ✅ RxToken and TxToken
- ✅ Link state tracking

**Packet bridging**: Not implemented and **not recommended** due to:
1. SPI bandwidth limitations vs WiFi throughput
2. Added complexity with minimal benefit
3. Socket APIs are more efficient and feature-complete

**Use embassy-net ONLY if:**
- You have existing code requiring embassy-net compatibility
- You understand the limitations and accept reduced performance
- You're willing to implement custom packet bridging logic

**For all other uses**: Use the socket APIs directly

The driver provides complete implementations of:
- TCP/UDP sockets via `NetworkDevice`
- HTTP/HTTPS via `HttpClient`
- MQTT via `MqttClient`
- DNS, SNTP, Ping via `advanced` module

## Memory Layout

### Static Resources

```rust
// Required static allocations:
- SpiTransport: ~16 bytes
- TmMutex<SpiTransport>: ~20 bytes
- AtProcessor: ~2KB (response slots + channels)
- WiFiManager: ~8 bytes
- NetworkDevice: ~20KB (socket pool with buffers)
- TlsManager: ~8 bytes
- Driver: ~32 bytes

Total: ~22KB
```

### Buffer Configuration

| Buffer | Default Size | Configurable | Purpose |
|--------|-------------|--------------|---------|
| SPI RX | 4096 bytes | ✅ via Config | SPI read operations |
| Socket RX | 2048 bytes × 8 | ✅ via Config | Per-socket received data |
| Multi-response | 512 bytes × 32 | Fixed | Collecting scan results |
| WiFi events | 4 slots | Fixed | WiFi state changes |
| Socket events | 16 slots | Fixed | Socket notifications |
| IPD data | 2048 bytes × 4 | Fixed | Received socket data |

**Total RAM**: ~45-50KB

## Concurrency Model

### Background Tasks

**RX Processor Task** (`processor.rx_task()`):
- Continuously reads from SPI
- Parses AT responses line-by-line
- Routes to handlers
- Switches to binary mode for +IPD

**IPD Processor Task** (`network.ipd_processor_task()`):
- Monitors IPD data channel
- Routes data to socket buffers

**User Tasks**:
- Application code using async APIs
- Cooperative multitasking via Embassy

### Synchronization

- **CriticalSectionRawMutex**: For all sync primitives
- Lock-free where possible (atomics for link state)
- No busy-waiting (all waits are async)

## API Usage Patterns

### WiFi Connection

```rust
// Initialize
driver.init_wifi(WiFiMode::Station).await?;

// Scan
let results = driver.wifi_scan().await?;

// Connect
driver.wifi_connect("SSID", "password").await?;

// Get IP
let ip_config = driver.get_ip_config().await?;
```

### TCP Socket

```rust
let device = driver.network_device();

// Allocate socket
let socket_id = device.allocate_socket(SocketProtocol::Tcp).await?;

// Connect
device.connect_socket(spi, socket_id, "example.com", 80, timeout).await?;

// Send
device.send_socket(spi, socket_id, data, timeout).await?;

// Receive
let n = device.receive_socket(spi, socket_id, buffer, timeout).await?;

// Close
device.close_socket(spi, socket_id, timeout).await?;
```

### HTTP Request

```rust
let http = driver.http_client();

// GET
let response = http.get(spi, "https://api.example.com/data").await?;

// POST
let response = http.post(spi, "https://api.example.com/endpoint", body).await?;
```

### MQTT

```rust
let mqtt = driver.mqtt_client(0);

// Connect
let mut config = MqttConfig::default();
config.client_id.push_str("my_device").unwrap();
mqtt.connect(spi, "broker.hivemq.com", 1883, &config).await?;

// Subscribe
mqtt.subscribe(spi, "topic/test", MqttQos::AtLeastOnce).await?;

// Publish
mqtt.publish(spi, "topic/test", "Hello", MqttQos::AtLeastOnce, false).await?;
```

## Porting Guide

### From FreeRTOS Driver

Embassy replacements:

| FreeRTOS | Embassy |
|----------|---------|
| `xQueueCreate` | `Channel::new()` |
| `xSemaphoreCreateBinary` | `Signal::new()` |
| `xTaskCreate` | `spawner.spawn()` |
| `vTaskDelay` | `Timer::after().await` |

### API Mapping

Similar high-level API:

```c
// C API
w6x_wifi_connect("SSID", "pwd");

// Rust API
driver.wifi_connect("SSID", "pwd").await?;
```

## Contributing

Maintain:
- No-std compatibility
- No heap allocation
- Async APIs
- Comprehensive error handling
- Tests for parsing logic
- Examples for features
