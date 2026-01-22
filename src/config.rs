//! Driver configuration

use embassy_time::Duration;

/// Driver configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// Command timeout duration
    pub command_timeout: Duration,

    /// Maximum retries for failed commands
    pub max_retries: u8,

    /// RX buffer size for SPI reads
    pub rx_buffer_size: usize,

    /// Number of response slots for concurrent commands
    pub response_slots: usize,

    /// WiFi event channel capacity
    pub wifi_event_capacity: usize,

    /// Socket event channel capacity
    pub socket_event_capacity: usize,

    /// Per-socket receive buffer size
    pub socket_rx_buffer_size: usize,

    /// Enable echo mode (for debugging)
    pub echo_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(5),
            max_retries: 3,
            rx_buffer_size: 4096,
            response_slots: 8,
            wifi_event_capacity: 4,
            socket_event_capacity: 16,
            socket_rx_buffer_size: 2048,
            echo_enabled: false,
        }
    }
}

impl Config {
    /// Create a new configuration with default values
    pub const fn new() -> Self {
        Self {
            command_timeout: Duration::from_secs(5),
            max_retries: 3,
            rx_buffer_size: 4096,
            response_slots: 8,
            wifi_event_capacity: 4,
            socket_event_capacity: 16,
            socket_rx_buffer_size: 2048,
            echo_enabled: false,
        }
    }

    /// Set the command timeout
    pub const fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    /// Set the maximum retries
    pub const fn with_max_retries(mut self, retries: u8) -> Self {
        self.max_retries = retries;
        self
    }

    /// Set the RX buffer size
    pub const fn with_rx_buffer_size(mut self, size: usize) -> Self {
        self.rx_buffer_size = size;
        self
    }

    /// Set the number of response slots
    pub const fn with_response_slots(mut self, slots: usize) -> Self {
        self.response_slots = slots;
        self
    }

    /// Enable or disable echo mode
    pub const fn with_echo(mut self, enabled: bool) -> Self {
        self.echo_enabled = enabled;
        self
    }

    /// Set the socket RX buffer size
    pub const fn with_socket_rx_buffer_size(mut self, size: usize) -> Self {
        self.socket_rx_buffer_size = size;
        self
    }
}
