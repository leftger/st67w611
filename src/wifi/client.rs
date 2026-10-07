//! Transport-generic Wi-Fi client.
//!
//! [`WiFi`] is the transport-generic Wi-Fi client: it works on T01 and T02 and
//! covers the whole `AT+CW*` surface, including the credential store,
//! auto-connect, hostname/country, TWT and the Soft-AP settings.
//! It replaces the old T01 `WiFiManager`.
//!
//! ```no_run
//! use st67w611::types::WiFiMode;
//! use st67w611::wifi::WiFi;
//!
//! async fn join<C: st67w611::at::AtTransport>(control: C) -> Result<(), st67w611::Error> {
//!     let wifi = WiFi::new(control);
//!     wifi.set_mode(WiFiMode::Station).await?;
//!     wifi.connect("ssid", "password").await?;
//!     Ok(())
//! }
//! ```

use crate::at::command::wifi as cmd;
use crate::at::reply::AtOutput;
use crate::at::transport::{AtTransport, AtTransportExt};
use crate::at::LineBuffer;
use crate::error::Result;
use crate::types::{Ipv4Address, WiFiMode};

/// High-level Wi-Fi client.
///
/// `T` is any [`AtTransport`]; see the module docs.
pub struct WiFi<T> {
    transport: T,
}

impl<T> WiFi<T> {
    /// Create a Wi-Fi client over `transport`.
    pub const fn new(transport: T) -> Self {
        Self { transport }
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

impl<T: AtTransport> WiFi<T> {
    // ---- Station ---------------------------------------------------------

    /// Select the Wi-Fi mode (`AT+CWMODE`).
    pub async fn set_mode(&self, mode: WiFiMode) -> Result<()> {
        self.transport.exec_ok(cmd::set_mode(mode)).await
    }

    /// Select the mode with an explicit station auto-connect flag.
    pub async fn set_mode_with_autoconnect(&self, mode: WiFiMode, sta_state: u32) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_mode_with_autoconnect(mode, sta_state))
            .await
    }

    /// Turn Wi-Fi off (`AT+CWMODE=0`).
    pub async fn stop(&self) -> Result<()> {
        self.transport.exec_ok(cmd::stop()).await
    }

    /// Join an access point (`AT+CWJAP`).
    pub async fn connect(&self, ssid: &str, password: &str) -> Result<()> {
        self.transport.exec_ok(cmd::connect(ssid, password)).await
    }

    /// Join an AP using a stored credential (`AT+CWJAPS`).
    pub async fn connect_stored(&self, ssid: &str) -> Result<()> {
        self.transport.exec_ok(cmd::connect_stored(ssid)).await
    }

    /// Disconnect (`AT+CWQAP`).
    pub async fn disconnect(&self) -> Result<()> {
        self.transport.exec_ok(cmd::disconnect()).await
    }

    /// Disconnect, choosing whether to restore the previous configuration.
    pub async fn disconnect_with(&self, restore: u32) -> Result<()> {
        self.transport.exec_ok(cmd::disconnect_with(restore)).await
    }

    /// List nearby access points (`AT+CWLAP`).
    pub async fn scan(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::scan()).await
    }

    /// Scan with filters.
    pub async fn scan_opts(
        &self,
        max: u32,
        ssid: &str,
        mac: &str,
        scan_type: u16,
    ) -> Result<AtOutput> {
        self.transport
            .exec(cmd::scan_opts(max, ssid, mac, scan_type))
            .await
    }

    /// Query the AP currently joined (`AT+CWJAP?`).
    pub async fn current_ap(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_current_ap()).await
    }

    /// Station MAC address (`AT+CIPSTAMAC?`).
    pub async fn mac(&self) -> Result<LineBuffer> {
        self.transport.query(cmd::get_mac(), "+CIPSTAMAC:").await
    }

    /// Soft-AP MAC address (`AT+CIPAPMAC?`).
    pub async fn ap_mac(&self) -> Result<LineBuffer> {
        self.transport.query(cmd::get_ap_mac(), "+CIPAPMAC:").await
    }

    /// Station IP configuration (`AT+CIPSTA?`).
    pub async fn station_ip(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_station_ip()).await
    }

    /// Set the station IP configuration (`AT+CIPSTA`).
    pub async fn set_station_ip(
        &self,
        ip: &Ipv4Address,
        gateway: &Ipv4Address,
        netmask: &Ipv4Address,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_station_ip(ip, gateway, netmask))
            .await
    }

    // ---- Soft-AP ---------------------------------------------------------

    /// Configure the Soft-AP (`AT+CWSAP`).
    pub async fn configure_ap(
        &self,
        ssid: &str,
        password: &str,
        channel: u8,
        encryption: u8,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::configure_ap(ssid, password, channel, encryption))
            .await
    }

    /// Query the Soft-AP configuration (`AT+CWSAP?`).
    pub async fn ap_config(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_ap_config()).await
    }

    /// List stations associated with the Soft-AP (`AT+CWLIF`).
    pub async fn list_stations(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::list_stations()).await
    }

    /// Force a station off the Soft-AP (`AT+CWQIF`).
    pub async fn disconnect_station(&self, mac: &str) -> Result<()> {
        self.transport.exec_ok(cmd::disconnect_station(mac)).await
    }

    /// Soft-AP IP configuration (`AT+CIPAP?`).
    pub async fn ap_ip(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_ap_ip()).await
    }

    /// Set the Soft-AP IP configuration (`AT+CIPAP`).
    pub async fn set_ap_ip(
        &self,
        ip: &Ipv4Address,
        gateway: &Ipv4Address,
        netmask: &Ipv4Address,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_ap_ip(ip, gateway, netmask))
            .await
    }

    /// Set the Soft-AP DHCP server (`AT+CWDHCPS`).
    pub async fn set_ap_dhcp_server(
        &self,
        lease_min: u32,
        ip: &Ipv4Address,
        gateway: &Ipv4Address,
    ) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_ap_dhcp_server(lease_min, ip, gateway))
            .await
    }

    /// Query the Soft-AP DHCP server (`AT+CWDHCPS?`).
    pub async fn ap_dhcp_server(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_ap_dhcp_server()).await
    }

    /// Enable or disable the station DHCP client (`AT+CWDHCP`).
    pub async fn set_dhcp(&self, mode: u8, enable: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_dhcp(mode, enable)).await
    }

    // ---- Credential store and auto-connect -------------------------------

    /// Store a credential (`AT+CWJAP` credential entry).
    pub async fn add_credentials(&self, ssid: &str, password: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::add_credentials(ssid, password))
            .await
    }

    /// Delete a stored credential.
    pub async fn delete_credentials(&self, ssid: &str) -> Result<()> {
        self.transport.exec_ok(cmd::delete_credentials(ssid)).await
    }

    /// List stored credentials.
    pub async fn credentials(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_credentials()).await
    }

    /// Enable or disable auto-connect (`AT+CWAUTOCONN`).
    pub async fn set_auto_connect(&self, enable: bool) -> Result<()> {
        self.transport.exec_ok(cmd::set_auto_connect(enable)).await
    }

    /// Query the auto-connect setting.
    pub async fn auto_connect(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_auto_connect()).await
    }

    // ---- Identity and regulatory ----------------------------------------

    /// Set the DHCP hostname (`AT+CWHOSTNAME`).
    pub async fn set_hostname(&self, hostname: &str) -> Result<()> {
        self.transport.exec_ok(cmd::set_hostname(hostname)).await
    }

    /// Query the DHCP hostname.
    pub async fn hostname(&self) -> Result<LineBuffer> {
        self.transport
            .query(cmd::get_hostname(), "+CWHOSTNAME:")
            .await
    }

    /// Set the country code (`AT+CWCOUNTRY`).
    pub async fn set_country(&self, policy: u32, country: &str) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_country(policy, country))
            .await
    }

    /// Query the country code.
    pub async fn country(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_country()).await
    }

    /// Query the connection state (`AT+CWSTATE?`).
    pub async fn state(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_state()).await
    }

    /// Query the network mode (`AT+CIPMODE?`).
    pub async fn net_mode(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_net_mode()).await
    }

    /// Set the reconnect policy (`AT+CWRECONNCFG`).
    pub async fn set_reconnect_cfg(&self, interval: u16, count: u16) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_reconnect_cfg(interval, count))
            .await
    }

    /// Select the transport protocol (`AT+CIPPROTO`).
    pub async fn set_protocol(&self, protocol: u32) -> Result<()> {
        self.transport.exec_ok(cmd::set_protocol(protocol)).await
    }

    /// Query the transport protocol.
    pub async fn protocol(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_protocol()).await
    }

    /// Configure antenna diversity (`AT+CWANT`).
    pub async fn set_antenna(&self, enable: u32, a: u32, b: u32) -> Result<()> {
        self.transport
            .exec_ok(cmd::set_antenna(enable, a, b))
            .await
    }

    /// Query the antenna configuration.
    pub async fn antenna(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_antenna()).await
    }

    /// Trigger Wi-Fi Protected Setup (`AT+CWWPS`).
    pub async fn wps(&self) -> Result<()> {
        self.transport.exec_ok(cmd::wps()).await
    }

    // ---- Power saving and TWT -------------------------------------------

    /// Set the station wake DTIM interval (`AT+CWSTADTIM`).
    pub async fn set_wake_dtim(&self, dtim: u32) -> Result<()> {
        self.transport.exec_ok(cmd::set_wake_dtim(dtim)).await
    }

    /// Query the Soft-AP DTIM interval.
    pub async fn ap_dtim(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_ap_dtim()).await
    }

    /// Query Target Wake Time support.
    pub async fn twt_supported(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::get_twt_supported()).await
    }

    /// Query the Target Wake Time status.
    pub async fn twt_status(&self) -> Result<AtOutput> {
        self.transport.exec(cmd::twt_status()).await
    }

    /// Tear down one or all TWT flows.
    pub async fn twt_teardown(&self, flow: u16, all: u16) -> Result<()> {
        self.transport.exec_ok(cmd::twt_teardown(flow, all)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at::transport::test_support::{block_on, output_with, Mock};

    #[test]
    fn mode_and_connect() {
        let wifi = WiFi::new(Mock::default());
        block_on(wifi.set_mode(WiFiMode::Station)).unwrap();
        assert_eq!(wifi.transport().last(), "AT+CWMODE=1,0");
        block_on(wifi.connect("ssid", "password")).unwrap();
        // Trailing comma: the W61 dialect keeps the parameter list open for the
        // optional BSSID / WEP arguments that may follow. See w61_at_wifi.c.
        assert_eq!(
            wifi.transport().last(),
            "AT+CWJAP=\"ssid\",\"password\","
        );
        block_on(wifi.connect_stored("ssid")).unwrap();
        assert_eq!(wifi.transport().last(), "AT+CWJAPS=\"ssid\"");
    }

    #[test]
    fn hostname_query_parses_line() {
        let wifi = WiFi::new(Mock {
            reply: Some(output_with("+CWHOSTNAME:my-device")),
            ..Default::default()
        });
        let name = block_on(wifi.hostname()).unwrap();
        assert_eq!(name.as_str(), "my-device");
        assert_eq!(wifi.transport().last(), "AT+CWHOSTNAME?");
    }

    #[test]
    fn credentials_and_autoconnect() {
        let wifi = WiFi::new(Mock::default());
        block_on(wifi.set_auto_connect(true)).unwrap();
        assert_eq!(wifi.transport().last(), "AT+CWAUTOCONN=1");
        block_on(wifi.credentials()).unwrap();
        assert_eq!(wifi.transport().last(), "AT+CWCRED?");
    }

    #[test]
    fn scan_returns_lines() {
        let wifi = WiFi::new(Mock {
            reply: Some(output_with("+CWLAP:(3,\"ap\",-50,00:11:22:33:44:55,6)")),
            ..Default::default()
        });
        let out = block_on(wifi.scan()).unwrap();
        assert!(out.is_ok());
        assert_eq!(out.lines.len(), 1);
    }
}
