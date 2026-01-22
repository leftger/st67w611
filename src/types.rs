//! Common types used throughout the driver

use heapless::{String, Vec};

/// Maximum SSID length (802.11 standard)
pub const MAX_SSID_LEN: usize = 32;

/// Maximum password length (WPA2)
pub const MAX_PASSWORD_LEN: usize = 63;

/// Maximum number of scan results to store
pub const MAX_SCAN_RESULTS: usize = 32;

/// Maximum hostname length
pub const MAX_HOSTNAME_LEN: usize = 64;

/// Maximum number of sockets
pub const MAX_SOCKETS: usize = 8;

/// Maximum SPI transfer size
pub const MAX_SPI_XFER: usize = 4096;

/// SSID type (max 32 bytes)
pub type Ssid = String<MAX_SSID_LEN>;

/// Password type (max 63 bytes)
pub type Password = String<MAX_PASSWORD_LEN>;

/// Hostname type (max 64 bytes)
pub type Hostname = String<MAX_HOSTNAME_LEN>;

/// WiFi mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum WiFiMode {
    /// Station mode (client)
    Station = 1,
    /// Access Point mode
    AccessPoint = 2,
    /// Station + AP mode
    StationAp = 3,
}

/// WiFi security type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum WiFiSecurityType {
    /// Open network (no security)
    Open = 0,
    /// WEP security
    Wep = 1,
    /// WPA/PSK security
    WpaPsk = 2,
    /// WPA2/PSK security
    Wpa2Psk = 3,
    /// WPA/WPA2 PSK mixed
    WpaWpa2Psk = 4,
    /// WPA2 Enterprise
    Wpa2Enterprise = 5,
    /// WPA3 security
    Wpa3 = 6,
}

/// WiFi connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum WiFiState {
    /// Not initialized
    Uninitialized,
    /// Initialized but not connected
    Disconnected,
    /// Connecting to AP
    Connecting,
    /// Connected but no IP
    Connected,
    /// Connected with IP address
    GotIp,
    /// Disconnecting
    Disconnecting,
}

/// Scan result for a single access point
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ScanResult {
    /// SSID
    pub ssid: Ssid,
    /// BSSID (MAC address)
    pub bssid: [u8; 6],
    /// Channel number
    pub channel: u8,
    /// Signal strength (RSSI) in dBm
    pub rssi: i8,
    /// Security type
    pub security: WiFiSecurityType,
}

/// Collection of scan results
pub type ScanResults = Vec<ScanResult, MAX_SCAN_RESULTS>;

/// IP address (IPv4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    /// Create from octets
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    /// Get as octets
    pub const fn octets(&self) -> [u8; 4] {
        self.0
    }
}

/// IP configuration
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct IpConfig {
    /// IP address
    pub ip: Ipv4Address,
    /// Gateway address
    pub gateway: Ipv4Address,
    /// Netmask
    pub netmask: Ipv4Address,
}

/// Socket protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SocketProtocol {
    /// TCP protocol
    Tcp,
    /// UDP protocol
    Udp,
    /// SSL/TLS over TCP
    Ssl,
}

/// Socket state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SocketState {
    /// Socket is free
    Free,
    /// Socket is allocated but not connected
    Allocated,
    /// Socket is connecting
    Connecting,
    /// Socket is connected
    Connected,
    /// Socket is closing
    Closing,
}

/// Socket ID (0-7)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SocketId(pub u8);

impl SocketId {
    /// Create a new socket ID
    pub const fn new(id: u8) -> Option<Self> {
        if id < MAX_SOCKETS as u8 {
            Some(Self(id))
        } else {
            None
        }
    }

    /// Get the raw ID value
    pub const fn raw(&self) -> u8 {
        self.0
    }
}

/// MQTT QoS level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MqttQos {
    /// At most once delivery
    AtMostOnce = 0,
    /// At least once delivery
    AtLeastOnce = 1,
    /// Exactly once delivery
    ExactlyOnce = 2,
}

/// MAC address
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// Create a new MAC address
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    /// Get the bytes
    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
}
