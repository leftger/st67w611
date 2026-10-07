//! Network driver notes
//!
//! ## Choosing between T01 and T02
//!
//! The module runs one of two firmware architectures, selected at build time
//! with a Cargo feature.
//!
//! ### T01 — TCP/IP on the module (`mission-t01`, default)
//!
//! The module owns the TCP/IP stack and the host drives it with AT commands.
//! Use the socket-based APIs provided by this crate:
//!
//! * [`crate::net::NetworkDevice`] for raw TCP/UDP/SSL sockets,
//! * [`crate::http::HttpClient`] for HTTP/HTTPS,
//! * [`crate::mqtt::Mqtt`] for MQTT.
//!
//! This is the recommended path for most applications: only application data
//! crosses SPI, so the 30 MHz link is not saturated by per-packet headers,
//! ACKs or retransmissions.
//!
//! ```ignore
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
//! ### T02 — TCP/IP on the host (`mission-t02`)
//!
//! The host runs the TCP/IP stack and the module acts as a WiFi MAC/PHY,
//! forwarding raw Ethernet frames (the same "direct link" architecture ST ships
//! as the `ST67W6X_CLI_LWIP` reference application).
//!
//! Use [`crate::net::xarxa`], which plugs the module into `embassy-net`:
//! IPv4, IPv6, TCP, UDP, DHCP, DNS/mDNS, SLAAC, multicast and ICMP all come
//! from the stack, and `embedded-tls` can be layered on top of a TCP socket.
//!
//! The trade-off is real: at 30 MHz the SPI link caps out at roughly
//! 1–2 MB/s, well under what WiFi could deliver, and now the host has to move
//! every header and ACK across it too. Choose T02 when you need the stack's
//! features (IPv6, TLS on the host, custom routing) or must interoperate with
//! existing `embassy-net` code, and T01 when you want the most throughput.
