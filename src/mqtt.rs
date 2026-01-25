//! MQTT client implementation

use embassy_time::Duration;
use heapless::String;

use crate::at::processor::AtProcessor;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;
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

/// MQTT client
pub struct MqttClient {
    /// Link ID for MQTT connection
    link_id: u8,
    /// AT processor
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
    /// Connection state
    connected: TmMutex<bool>,
}

impl MqttClient {
    /// Create a new MQTT client
    pub const fn new(link_id: u8, processor: &'static AtProcessor, timeout: Duration) -> Self {
        Self {
            link_id,
            processor,
            timeout,
            connected: TmMutex::new(false),
        }
    }

    /// Connect to MQTT broker
    pub async fn connect<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        host: &str,
        port: u16,
        config: &MqttConfig,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // Set user configuration
        let username = config.username.as_ref().map(|s| s.as_str()).unwrap_or("");
        let password = config.password.as_ref().map(|s| s.as_str()).unwrap_or("");

        let cmd = crate::at::command::mqtt::set_user_config(
            self.link_id,
            1, // scheme: 1 for TCP, 2 for TLS
            &config.client_id,
            username,
            password,
        )?;

        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), self.timeout)
            .await?;
        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        // Connect to broker
        let cmd = crate::at::command::mqtt::connect(self.link_id, host, port, false)?;
        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), Duration::from_secs(30))
            .await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        let mut connected = self.connected.lock().await;
        *connected = true;

        Ok(())
    }

    /// Publish a message
    pub async fn publish<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        topic: &str,
        payload: &str,
        qos: MqttQos,
        retain: bool,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let connected = self.connected.lock().await;
        if !*connected {
            return Err(Error::NotConnected);
        }
        drop(connected);

        let cmd = crate::at::command::mqtt::publish(self.link_id, topic, payload, qos, retain)?;
        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), self.timeout)
            .await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        Ok(())
    }

    /// Subscribe to a topic
    pub async fn subscribe<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        topic: &str,
        qos: MqttQos,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let connected = self.connected.lock().await;
        if !*connected {
            return Err(Error::NotConnected);
        }
        drop(connected);

        let cmd = crate::at::command::mqtt::subscribe(self.link_id, topic, qos)?;
        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), self.timeout)
            .await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        Ok(())
    }

    /// Unsubscribe from a topic
    pub async fn unsubscribe<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        topic: &str,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let connected = self.connected.lock().await;
        if !*connected {
            return Err(Error::NotConnected);
        }
        drop(connected);

        let cmd = crate::at::command::mqtt::unsubscribe(self.link_id, topic)?;
        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), self.timeout)
            .await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        Ok(())
    }

    /// Disconnect from broker
    pub async fn disconnect<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
    ) -> Result<()>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let cmd = crate::at::command::mqtt::disconnect(self.link_id)?;
        let response = self
            .processor
            .send_command(spi, cmd.as_bytes(), self.timeout)
            .await?;

        if response != crate::at::AtResponse::Ok {
            return Err(Error::MqttError);
        }

        let mut connected = self.connected.lock().await;
        *connected = false;

        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        let connected = self.connected.lock().await;
        *connected
    }
}
