//! Transport-generic MQTT client.
//!
//! [`Mqtt`] is the transport-generic MQTT client. It covers the whole
//! `AT+MQTT*` surface, including the connection configuration and **Last
//! Will** (`AT+MQTTCONNCFG`), and works on T01 and T02 alike.
//!
//! ```no_run
//! use st67w611::mqtt::Mqtt;
//! use st67w611::types::MqttQos;
//!
//! async fn publish<C: st67w611::at::AtTransport>(control: C) -> Result<(), st67w611::Error> {
//!     let mqtt = Mqtt::new(control);
//!     // Last Will: keepalive 60 s, topic "status", message "offline".
//!     mqtt.set_last_will(0, 60, "status", "offline", 1, true).await?;
//!     mqtt.connect(0, "broker.local", 1883, true).await?;
//!     mqtt.publish(0, "status", "online", MqttQos::AtMostOnce, false).await?;
//!     Ok(())
//! }
//! ```

use crate::at::command::mqtt as cmd;
use crate::at::reply::AtOutput;
use crate::at::transport::{AtTransport, AtTransportExt};
use crate::error::Result;
use crate::types::MqttQos;

/// High-level MQTT client.
///
/// `T` is any [`AtTransport`]; see the module docs.
pub struct Mqtt<T> {
    transport: T,
    connected: core::sync::atomic::AtomicBool,
}

impl<T> Mqtt<T> {
    /// Create an MQTT client over `transport`.
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            connected: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Consume the client, returning the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: AtTransport> Mqtt<T> {
    /// Configure the MQTT user (`AT+MQTTUSERCFG`).
    pub async fn set_user_config(
        &self,
        link_id: u8,
        scheme: u8,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_user_config(
                link_id, scheme, client_id, username, password,
            ))
            .await
    }

    /// Query the MQTT user config.
    pub async fn user_config(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_user_config()).await
    }

    /// Configure the connection: keepalive, clean-session flag and Last Will
    /// (`AT+MQTTCONNCFG`).
    ///
    /// `disable_clean_session` is `1` to disable the clean session; `will_qos`
    /// is the Last Will QoS; `will_retain` selects the Will retain flag.
    pub async fn set_conn_config(
        &self,
        link_id: u8,
        keepalive: u32,
        disable_clean_session: u32,
        will_topic: &str,
        will_message: &str,
        will_qos: u32,
        will_retain: u32,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_conn_config(
                link_id,
                keepalive,
                disable_clean_session,
                will_topic,
                will_message,
                will_qos,
                will_retain,
            ))
            .await
    }

    /// Convenience form of [`set_conn_config`](Self::set_conn_config) for the
    /// Last Will only.
    pub async fn set_last_will(
        &self,
        link_id: u8,
        keepalive: u32,
        will_topic: &str,
        will_message: &str,
        will_qos: u32,
        will_retain: bool,
    ) -> Result<()> {
        self.set_conn_config(
            link_id,
            keepalive,
            0,
            will_topic,
            will_message,
            will_qos,
            if will_retain { 1 } else { 0 },
        )
        .await
    }

    /// Query the connection config.
    pub async fn conn_config(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_conn_config()).await
    }

    /// Connect to a broker (`AT+MQTTCONN`).
    pub async fn connect(
        &self,
        link_id: u8,
        host: &str,
        port: u16,
        reconnect: bool,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::connect(link_id, host, port, reconnect))
            .await?;
        self.connected
            .store(true, core::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Whether [`connect`](Self::connect) has succeeded and
    /// [`disconnect`](Self::disconnect) has not since been called.
    pub async fn is_connected(&self) -> bool {
        self.connected.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Query the broker connection.
    pub async fn connection(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_connection()).await
    }

    /// Set the TLS SNI (`AT+MQTTSSLCSNI`).
    pub async fn set_sni(&self, link_id: u8, sni: &str) -> Result<()> {
        self.transport.exec_ok(cmd::set_sni(link_id, sni)).await
    }

    /// Query the TLS SNI.
    pub async fn sni(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_sni()).await
    }

    /// Publish a text payload (`AT+MQTTPUB`).
    pub async fn publish(
        &self,
        link_id: u8,
        topic: &str,
        data: &str,
        qos: MqttQos,
        retain: bool,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::publish(link_id, topic, data, qos, retain))
            .await
    }

    /// Publish a binary payload (`AT+MQTTPUBRAW` then the bytes).
    pub async fn publish_raw(
        &self,
        link_id: u8,
        topic: &str,
        data: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<()> {
        self.transport
            .exec_payload(
                cmd::publish_raw(link_id, topic, data.len() as u32, qos, retain),
                data,
            )
            .await
    }

    /// Subscribe to a topic (`AT+MQTTSUB`).
    pub async fn subscribe(&self, link_id: u8, topic: &str, qos: MqttQos) -> Result<()> {
        self.transport
            .exec_ok(cmd::subscribe(link_id, topic, qos))
            .await
    }

    /// List the active subscriptions.
    pub async fn subscriptions(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_subscriptions()).await
    }

    /// Unsubscribe from a topic (`AT+MQTTUNSUB`).
    pub async fn unsubscribe(&self, link_id: u8, topic: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::unsubscribe(link_id, topic))
            .await
    }

    /// Disconnect and release the client (`AT+MQTTCLEAN`).
    pub async fn disconnect(&self, link_id: u8) -> Result<()> {
        self.transport.exec_ok(cmd::disconnect(link_id)).await?;
        self.connected
            .store(false, core::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at::transport::test_support::{block_on, output_with, Mock};

    #[test]
    fn last_will_is_sent_in_conn_config() {
        let mqtt = Mqtt::new(Mock::default());
        block_on(mqtt.set_last_will(0, 60, "status", "offline", 1, true)).unwrap();
        assert_eq!(
            mqtt.transport().last(),
            "AT+MQTTCONNCFG=0,60,0,\"status\",\"offline\",1,1"
        );
        block_on(mqtt.conn_config()).unwrap();
        assert_eq!(mqtt.transport().last(), "AT+MQTTCONNCFG?");
    }

    #[test]
    fn publish_text_and_raw() {
        let mqtt = Mqtt::new(Mock {
            payload_supported: true,
            ..Default::default()
        });
        block_on(mqtt.publish(0, "t", "hi", MqttQos::AtMostOnce, false)).unwrap();
        assert_eq!(mqtt.transport().last(), "AT+MQTTPUB=0,\"t\",\"hi\",0,0");

        block_on(mqtt.publish_raw(0, "t", &[1, 2, 3], MqttQos::AtLeastOnce, true)).unwrap();
        assert_eq!(mqtt.transport().last(), "AT+MQTTPUBRAW=0,\"t\",3,1,1");
        assert_eq!(mqtt.transport().last_payload.get(), 3);
    }

    #[test]
    fn subscribe_and_disconnect() {
        let mqtt = Mqtt::new(Mock::default());
        block_on(mqtt.subscribe(0, "t/#", MqttQos::ExactlyOnce)).unwrap();
        assert_eq!(mqtt.transport().last(), "AT+MQTTSUB=0,\"t/#\",2");
        block_on(mqtt.disconnect(0)).unwrap();
        assert_eq!(mqtt.transport().last(), "AT+MQTTCLEAN=0");
    }

    #[test]
    fn raw_publish_requires_payload_support() {
        let mqtt = Mqtt::new(Mock::default());
        assert_eq!(
            block_on(mqtt.publish_raw(0, "t", &[1], MqttQos::AtMostOnce, false)),
            Err(crate::error::Error::NotSupported)
        );
    }

    #[test]
    fn subscriptions_returns_lines() {
        let mqtt = Mqtt::new(Mock {
            reply: Some(output_with("+MQTTSUB:0,\"t/#\",1")),
            ..Default::default()
        });
        let out = block_on(mqtt.subscriptions()).unwrap();
        assert_eq!(out.value_after("+MQTTSUB:"), Some("0,\"t/#\",1"));
    }
}
