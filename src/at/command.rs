//! AT command definitions and formatting

use heapless::String;
use core::fmt::Write as _;

use crate::error::{Error, Result};
use crate::types::*;

/// Maximum AT command length
pub const MAX_AT_COMMAND_LEN: usize = 256;

/// AT command string type
pub type AtCommandString = String<MAX_AT_COMMAND_LEN>;

/// AT command builder
#[derive(Debug, Clone)]
pub struct AtCommand {
    buffer: AtCommandString,
}

impl AtCommand {
    /// Create a new AT command
    pub fn new(cmd: &str) -> Result<Self> {
        let mut buffer = AtCommandString::new();
        buffer.push_str(cmd).map_err(|_| Error::BufferTooSmall)?;
        Ok(Self { buffer })
    }

    /// Add a parameter to the command
    pub fn with_param(mut self, param: &str) -> Result<Self> {
        self.buffer.push(',').map_err(|_| Error::BufferTooSmall)?;
        self.buffer.push_str(param).map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Add a quoted string parameter
    pub fn with_string_param(mut self, param: &str) -> Result<Self> {
        self.buffer.push(',').map_err(|_| Error::BufferTooSmall)?;
        self.buffer.push('"').map_err(|_| Error::BufferTooSmall)?;
        self.buffer.push_str(param).map_err(|_| Error::BufferTooSmall)?;
        self.buffer.push('"').map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Add an integer parameter
    pub fn with_int_param(mut self, value: i32) -> Result<Self> {
        self.buffer.push(',').map_err(|_| Error::BufferTooSmall)?;
        write!(self.buffer, "{}", value).map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Finalize the command with CRLF
    pub fn build(mut self) -> Result<AtCommandString> {
        self.buffer.push_str("\r\n").map_err(|_| Error::BufferTooSmall)?;
        Ok(self.buffer)
    }

    /// Get the command as bytes
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.as_bytes()
    }
}

/// System AT commands
pub mod system {
    use super::*;

    /// Test AT command (AT)
    pub fn test() -> Result<AtCommandString> {
        AtCommand::new("AT")?.build()
    }

    /// Reset module (AT+RST)
    pub fn reset() -> Result<AtCommandString> {
        AtCommand::new("AT+RST")?.build()
    }

    /// Get firmware version (AT+GMR)
    pub fn get_version() -> Result<AtCommandString> {
        AtCommand::new("AT+GMR")?.build()
    }

    /// Set echo mode (ATE0 or ATE1)
    pub fn set_echo(enabled: bool) -> Result<AtCommandString> {
        if enabled {
            AtCommand::new("ATE1")?.build()
        } else {
            AtCommand::new("ATE0")?.build()
        }
    }

    /// Deep sleep (AT+GSLP)
    pub fn deep_sleep(time_ms: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+GSLP")?
            .with_int_param(time_ms as i32)?
            .build()
    }

    /// Restore factory settings (AT+RESTORE)
    pub fn restore_factory() -> Result<AtCommandString> {
        AtCommand::new("AT+RESTORE")?.build()
    }

    /// Set UART configuration (AT+UART_CUR)
    pub fn set_uart(baudrate: u32, databits: u8, stopbits: u8, parity: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+UART_CUR")?
            .with_int_param(baudrate as i32)?
            .with_int_param(databits as i32)?
            .with_int_param(stopbits as i32)?
            .with_int_param(parity as i32)?
            .build()
    }

    /// Set system store mode (AT+SYSSTORE)
    /// 0 = don't save to flash, 1 = save to flash
    pub fn set_sysstore(mode: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+SYSSTORE")?
            .with_int_param(mode as i32)?
            .build()
    }

    /// Get system RAM usage (AT+SYSRAM?)
    pub fn get_ram_usage() -> Result<AtCommandString> {
        AtCommand::new("AT+SYSRAM?")?.build()
    }

    /// Get system flash info (AT+SYSFLASH?)
    pub fn get_flash_info() -> Result<AtCommandString> {
        AtCommand::new("AT+SYSFLASH?")?.build()
    }
}

/// WiFi AT commands
pub mod wifi {
    use super::*;

    /// Set WiFi mode (AT+CWMODE)
    pub fn set_mode(mode: WiFiMode) -> Result<AtCommandString> {
        AtCommand::new("AT+CWMODE")?
            .with_int_param(mode as i32)?
            .build()
    }

    /// Connect to AP (AT+CWJAP)
    pub fn connect(ssid: &str, password: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWJAP")?
            .with_string_param(ssid)?
            .with_string_param(password)?
            .build()
    }

    /// Disconnect from AP (AT+CWQAP)
    pub fn disconnect() -> Result<AtCommandString> {
        AtCommand::new("AT+CWQAP")?.build()
    }

    /// Scan for networks (AT+CWLAP)
    pub fn scan() -> Result<AtCommandString> {
        AtCommand::new("AT+CWLAP")?.build()
    }

    /// Get current AP (AT+CWJAP?)
    pub fn get_current_ap() -> Result<AtCommandString> {
        AtCommand::new("AT+CWJAP?")?.build()
    }

    /// Set station IP (AT+CIPSTA)
    pub fn set_station_ip(ip: &Ipv4Address, gateway: &Ipv4Address, netmask: &Ipv4Address) -> Result<AtCommandString> {
        use core::fmt::Write;

        let mut ip_str = String::<16>::new();
        write!(ip_str, "{}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3]).map_err(|_| Error::BufferTooSmall)?;

        let mut gw_str = String::<16>::new();
        write!(gw_str, "{}.{}.{}.{}", gateway.0[0], gateway.0[1], gateway.0[2], gateway.0[3]).map_err(|_| Error::BufferTooSmall)?;

        let mut nm_str = String::<16>::new();
        write!(nm_str, "{}.{}.{}.{}", netmask.0[0], netmask.0[1], netmask.0[2], netmask.0[3]).map_err(|_| Error::BufferTooSmall)?;

        AtCommand::new("AT+CIPSTA")?
            .with_string_param(&ip_str)?
            .with_string_param(&gw_str)?
            .with_string_param(&nm_str)?
            .build()
    }

    /// Get station IP (AT+CIPSTA?)
    pub fn get_station_ip() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTA?")?.build()
    }

    /// Get MAC address (AT+CIPSTAMAC?)
    pub fn get_mac() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTAMAC?")?.build()
    }

    /// Configure soft AP (AT+CWSAP)
    /// ssid: AP SSID
    /// password: AP password (8-63 chars for WPA/WPA2)
    /// channel: WiFi channel (1-13)
    /// encryption: 0=OPEN, 2=WPA_PSK, 3=WPA2_PSK, 4=WPA_WPA2_PSK
    pub fn configure_ap(ssid: &str, password: &str, channel: u8, encryption: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CWSAP")?
            .with_string_param(ssid)?
            .with_string_param(password)?
            .with_int_param(channel as i32)?
            .with_int_param(encryption as i32)?
            .build()
    }

    /// Get soft AP configuration (AT+CWSAP?)
    pub fn get_ap_config() -> Result<AtCommandString> {
        AtCommand::new("AT+CWSAP?")?.build()
    }

    /// List connected stations (AT+CWLIF)
    pub fn list_stations() -> Result<AtCommandString> {
        AtCommand::new("AT+CWLIF")?.build()
    }

    /// Set AP IP configuration (AT+CIPAP)
    pub fn set_ap_ip(ip: &Ipv4Address, gateway: &Ipv4Address, netmask: &Ipv4Address) -> Result<AtCommandString> {
        use core::fmt::Write;

        let mut ip_str = String::<16>::new();
        write!(ip_str, "{}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3]).map_err(|_| Error::BufferTooSmall)?;

        let mut gw_str = String::<16>::new();
        write!(gw_str, "{}.{}.{}.{}", gateway.0[0], gateway.0[1], gateway.0[2], gateway.0[3]).map_err(|_| Error::BufferTooSmall)?;

        let mut nm_str = String::<16>::new();
        write!(nm_str, "{}.{}.{}.{}", netmask.0[0], netmask.0[1], netmask.0[2], netmask.0[3]).map_err(|_| Error::BufferTooSmall)?;

        AtCommand::new("AT+CIPAP")?
            .with_string_param(&ip_str)?
            .with_string_param(&gw_str)?
            .with_string_param(&nm_str)?
            .build()
    }

    /// Get AP IP configuration (AT+CIPAP?)
    pub fn get_ap_ip() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPAP?")?.build()
    }

    /// Set DHCP configuration (AT+CWDHCP)
    /// mode: 0=soft-AP, 1=station, 2=both
    /// enable: true to enable DHCP, false to disable
    pub fn set_dhcp(mode: u8, enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CWDHCP")?
            .with_int_param(mode as i32)?
            .with_int_param(if enable { 1 } else { 0 })?
            .build()
    }
}

/// Network AT commands
pub mod network {
    use super::*;

    /// Start connection (AT+CIPSTART)
    pub fn connect(link_id: u8, protocol: SocketProtocol, host: &str, port: u16) -> Result<AtCommandString> {
        let proto = match protocol {
            SocketProtocol::Tcp => "TCP",
            SocketProtocol::Udp => "UDP",
            SocketProtocol::Ssl => "SSL",
        };

        AtCommand::new("AT+CIPSTART")?
            .with_int_param(link_id as i32)?
            .with_string_param(proto)?
            .with_string_param(host)?
            .with_int_param(port as i32)?
            .build()
    }

    /// Send data (AT+CIPSEND)
    pub fn send(link_id: u8, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSEND")?
            .with_int_param(link_id as i32)?
            .with_int_param(length as i32)?
            .build()
    }

    /// Close connection (AT+CIPCLOSE)
    pub fn close(link_id: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPCLOSE")?
            .with_int_param(link_id as i32)?
            .build()
    }

    /// Receive data from connection (AT+CIPRECV)
    pub fn receive(link_id: u8, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPRECV")?
            .with_int_param(link_id as i32)?
            .with_int_param(length as i32)?
            .build()
    }

    /// Get connection status (AT+CIPSTATUS)
    pub fn get_status() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTATUS")?.build()
    }

    /// Set multiple connections mode (AT+CIPMUX)
    pub fn set_mux(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPMUX")?
            .with_int_param(if enable { 1 } else { 0 })?
            .build()
    }

    /// Configure SSL (AT+CIPSSLCCONF)
    pub fn configure_ssl(link_id: u8, auth_mode: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCCONF")?
            .with_int_param(link_id as i32)?
            .with_int_param(auth_mode as i32)?
            .build()
    }

    /// Set SNI (AT+CIPSSLCSNI)
    pub fn set_sni(link_id: u8, hostname: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCSNI")?
            .with_int_param(link_id as i32)?
            .with_string_param(hostname)?
            .build()
    }

    /// DNS resolution (AT+CIPDOMAIN)
    pub fn dns_lookup(hostname: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPDOMAIN")?
            .with_string_param(hostname)?
            .build()
    }

    /// Set DNS server (AT+CIPDNS_CUR)
    pub fn set_dns(enable: bool, dns1: &str, dns2: Option<&str>) -> Result<AtCommandString> {
        let mut cmd = AtCommand::new("AT+CIPDNS_CUR")?
            .with_int_param(if enable { 1 } else { 0 })?
            .with_string_param(dns1)?;

        if let Some(dns2_addr) = dns2 {
            cmd = cmd.with_string_param(dns2_addr)?;
        }

        cmd.build()
    }

    /// Get DNS server configuration (AT+CIPDNS_CUR?)
    pub fn get_dns() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPDNS_CUR?")?.build()
    }

    /// Enable/disable SNTP (AT+CIPSNTPCFG)
    pub fn configure_sntp(enable: bool, timezone: i8, server1: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSNTPCFG")?
            .with_int_param(if enable { 1 } else { 0 })?
            .with_int_param(timezone as i32)?
            .with_string_param(server1)?
            .build()
    }

    /// Get SNTP time (AT+CIPSNTPTIME?)
    pub fn get_sntp_time() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSNTPTIME?")?.build()
    }

    /// Set ping timeout (AT+PING)
    pub fn ping(host: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+PING")?
            .with_string_param(host)?
            .build()
    }
}

/// Filesystem AT commands
pub mod filesystem {
    use super::*;

    /// Write data to filesystem (AT+FS=<op>,<filename>,<offset>,<length>)
    /// Operation: 0=delete, 1=write, 2=read
    pub fn fs_write(filename: &str, offset: usize, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+FS")?
            .with_int_param(1)? // 1 = write operation
            .with_string_param(filename)?
            .with_int_param(offset as i32)?
            .with_int_param(length as i32)?
            .build()
    }

    /// Read from filesystem (AT+FS)
    pub fn fs_read(filename: &str, offset: usize, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+FS")?
            .with_int_param(2)? // 2 = read operation
            .with_string_param(filename)?
            .with_int_param(offset as i32)?
            .with_int_param(length as i32)?
            .build()
    }

    /// Delete file from filesystem (AT+FS)
    pub fn fs_delete(filename: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+FS")?
            .with_int_param(0)? // 0 = delete operation
            .with_string_param(filename)?
            .with_int_param(0)?
            .with_int_param(0)?
            .build()
    }

    /// List files (AT+FS=3)
    pub fn fs_list() -> Result<AtCommandString> {
        AtCommand::new("AT+FS")?
            .with_int_param(3)? // 3 = list operation
            .with_string_param("")?
            .with_int_param(0)?
            .with_int_param(0)?
            .build()
    }
}

/// MQTT AT commands
pub mod mqtt {
    use super::*;

    /// Set MQTT username (AT+MQTTUSERCFG)
    pub fn set_user_config(
        link_id: u8,
        scheme: u8,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTUSERCFG")?
            .with_int_param(link_id as i32)?
            .with_int_param(scheme as i32)?
            .with_string_param(client_id)?
            .with_string_param(username)?
            .with_string_param(password)?
            .build()
    }

    /// Connect to MQTT broker (AT+MQTTCONN)
    pub fn connect(link_id: u8, host: &str, port: u16, reconnect: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCONN")?
            .with_int_param(link_id as i32)?
            .with_string_param(host)?
            .with_int_param(port as i32)?
            .with_int_param(if reconnect { 1 } else { 0 })?
            .build()
    }

    /// Publish message (AT+MQTTPUB)
    pub fn publish(link_id: u8, topic: &str, data: &str, qos: MqttQos, retain: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTPUB")?
            .with_int_param(link_id as i32)?
            .with_string_param(topic)?
            .with_string_param(data)?
            .with_int_param(qos as i32)?
            .with_int_param(if retain { 1 } else { 0 })?
            .build()
    }

    /// Subscribe to topic (AT+MQTTSUB)
    pub fn subscribe(link_id: u8, topic: &str, qos: MqttQos) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTSUB")?
            .with_int_param(link_id as i32)?
            .with_string_param(topic)?
            .with_int_param(qos as i32)?
            .build()
    }

    /// Unsubscribe from topic (AT+MQTTUNSUB)
    pub fn unsubscribe(link_id: u8, topic: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTUNSUB")?
            .with_int_param(link_id as i32)?
            .with_string_param(topic)?
            .build()
    }

    /// Disconnect from broker (AT+MQTTCLEAN)
    pub fn disconnect(link_id: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCLEAN")?
            .with_int_param(link_id as i32)?
            .build()
    }
}
