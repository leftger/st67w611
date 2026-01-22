//! Network driver notes
//!
//! ## Why This Driver Does NOT Support embassy-net
//!
//! ### Hardware Constraint
//!
//! The ST67W611 module has a maximum SPI clock of **30MHz**, providing:
//! - Theoretical throughput: ~3.75 MB/s
//! - Practical throughput: ~1-2 MB/s (with AT protocol overhead)
//!
//! Meanwhile, WiFi 802.11n can provide **10-100+ Mbps** (1.25-12.5 MB/s).
//!
//! **SPI is the bottleneck, not WiFi.**
//!
//! ### Why Transparent Mode Would Be Inefficient
//!
//! If the module forwarded raw IP packets over SPI:
//! - Every packet would include IP/TCP/UDP headers (~40-60 bytes)
//! - ACKs, retransmissions, and control packets would consume SPI bandwidth
//! - Protocol processing overhead would saturate the 30MHz SPI link
//! - Actual application throughput would be severely limited
//!
//! ### Why Built-in TCP/IP Stack Is The Right Design
//!
//! The module's built-in TCP/IP stack:
//! - Processes all protocols **locally** on the module
//! - Only application data crosses SPI
//! - Eliminates protocol overhead from SPI transfers
//! - Maximizes effective throughput for your application
//! - This is **optimal given the hardware constraints**
//!
//! ### Correct Usage Pattern
//!
//! Use the socket-based APIs provided by this driver:
//!
//! ```rust
//! // TCP/UDP sockets
//! let socket = device.allocate_socket(SocketProtocol::Tcp).await?;
//! device.connect_socket(spi, socket, "host", 80, timeout).await?;
//! device.send_socket(spi, socket, data, timeout).await?;
//!
//! // HTTP/HTTPS
//! let http = driver.http_client();
//! let response = http.get(spi, "https://api.example.com").await?;
//!
//! // MQTT
//! let mqtt = driver.mqtt_client(0);
//! mqtt.connect(spi, "broker", 1883, &config).await?;
//! mqtt.publish(spi, "topic", "data", qos, false).await?;
//! ```
//!
//! ### Summary
//!
//! This driver does NOT implement `embassy_net::driver::Driver` because:
//! 1. SPI bandwidth limitations make transparent mode impractical
//! 2. The module's built-in TCP/IP stack is the correct architecture
//! 3. Socket-based APIs are more efficient and fully featured
//!
//! If you need embassy-net compatibility, this is not the right WiFi module for your project.
//! Consider modules with higher-speed interfaces (SDIO, USB, etc.) that can support
//! transparent packet mode.
