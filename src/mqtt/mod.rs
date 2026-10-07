//! MQTT support.
//!
//! [`MqttConfig`] and [`MqttMessage`] are plain data types; the
//! transport-generic client is [`client::Mqtt`], re-exported as [`Mqtt`].

use heapless::String;

use crate::types::MqttQos;

/// Maximum MQTT topic length
pub const MAX_MQTT_TOPIC_LEN: usize = 128;

/// Maximum MQTT payload length
pub const MAX_MQTT_PAYLOAD_LEN: usize = 512;

/// MQTT topic type
pub type MqttTopic = String<MAX_MQTT_TOPIC_LEN>;

/// MQTT payload type
pub type MqttPayload = String<MAX_MQTT_PAYLOAD_LEN>;

/// MQTT connection configuration
#[derive(Debug, Clone)]
pub struct MqttConfig {
    /// Client ID
    pub client_id: String<32>,
    /// Username (optional)
    pub username: Option<String<32>>,
    /// Password (optional)
    pub password: Option<String<32>>,
    /// Keep alive interval (seconds)
    pub keep_alive: u16,
    /// Clean session flag
    pub clean_session: bool,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            username: None,
            password: None,
            keep_alive: 60,
            clean_session: true,
        }
    }
}

/// MQTT message
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MqttMessage {
    /// Topic
    pub topic: MqttTopic,
    /// Payload
    pub payload: MqttPayload,
    /// QoS level
    pub qos: MqttQos,
    /// Retain flag
    pub retain: bool,
}

pub mod client;

pub use client::Mqtt;
