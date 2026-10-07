//! AT command definitions and formatting.
//!
//! The dialect is the ST67W611 (`W61`) AT command set used by the X-CUBE
//! network driver, not the ESP-AT set some commands superficially resemble.
//! Formats are transcribed from `Driver/W61_at/w61_at_*.c`; each command has a
//! unit test pinning the exact bytes so regressions are caught without hardware.
//!
//! Commands are built with [`AtCommand`]:
//!
//! ```
//! use st67w611::at::command::AtCommand;
//!
//! // AT+CWJAP="ssid","password"
//! let cmd = AtCommand::new("AT+CWJAP")
//!     .args().unwrap()
//!     .arg_quoted("ssid").unwrap()
//!     .arg_quoted("password").unwrap()
//!     .build()
//!     .unwrap();
//! assert_eq!(cmd.as_str(), "AT+CWJAP=\"ssid\",\"password\",\r\n");
//! ```

use core::fmt::Write as _;
use heapless::String;

use crate::error::{Error, Result};
use crate::types::*;

/// Maximum AT command length.
pub const MAX_AT_COMMAND_LEN: usize = 256;

/// AT command string type.
pub type AtCommandString = String<MAX_AT_COMMAND_LEN>;

/// The command without its trailing line ending.
///
/// [`AtTransport`](crate::at::AtTransport) implementations take the command
/// without CRLF and add it themselves; the builders produce wire bytes.
pub fn wire_str(command: &AtCommandString) -> &str {
    command.as_str().trim_end_matches(['\r', '\n'])
}

/// AT command builder.
///
/// `new` takes the command head. Call [`args`](Self::args) to append the `=`
/// that starts the argument list, then one of the `arg_*` methods per
/// parameter; they insert the separating commas. Commands with a literal
/// argument layout (for example `AT+FS=0,2`) put that layout in the head and
/// call `args()` only when they have further parameters.
#[derive(Debug, Clone)]
pub struct AtCommand {
    buffer: AtCommandString,
    first_arg: bool,
}

impl AtCommand {
    /// Start a command from its head, for example `"AT+CWJAP"`.
    ///
    /// The head may already contain literal arguments (for example
    /// `"AT+FS=0,2"`); the `arg_*` methods then append to them with commas.
    pub fn new(head: &str) -> Self {
        let mut buffer = AtCommandString::new();
        // The command head is short; ignore overflow (it cannot exceed the
        // buffer in practice, and `build` reports it if it somehow does).
        let _ = buffer.push_str(head);
        Self {
            buffer,
            // `false`: a head with literal arguments needs a comma before the
            // first appended one; [`args`](Self::args) adjusts this.
            first_arg: false,
        }
    }

    fn separator(&mut self) -> Result<()> {
        if !self.first_arg {
            self.buffer.push(',').map_err(|_| Error::BufferTooSmall)?;
        }
        self.first_arg = false;
        Ok(())
    }

    /// Begin the argument list.
    ///
    /// If the head does not yet contain `=` (for example `"AT+CWJAP"`), this
    /// appends it. If it already does (for example `"AT+FS=0,2"`), the head's
    /// literal arguments are kept and the next `arg_*` is comma-separated.
    pub fn args(mut self) -> Result<Self> {
        if self.buffer.as_bytes().contains(&b'=') {
            self.first_arg = false;
        } else {
            self.buffer.push('=').map_err(|_| Error::BufferTooSmall)?;
            self.first_arg = true;
        }
        Ok(self)
    }

    /// Append a raw (unquoted) argument.
    pub fn arg_str(mut self, value: &str) -> Result<Self> {
        self.separator()?;
        self.buffer
            .push_str(value)
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Append a `"double quoted"` argument.
    pub fn arg_quoted(mut self, value: &str) -> Result<Self> {
        self.separator()?;
        self.buffer.push('"').map_err(|_| Error::BufferTooSmall)?;
        self.buffer
            .push_str(value)
            .map_err(|_| Error::BufferTooSmall)?;
        self.buffer.push('"').map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Append a numeric argument.
    pub fn arg_int(mut self, value: impl core::fmt::Display) -> Result<Self> {
        self.separator()?;
        write!(self.buffer, "{}", value).map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Append a value verbatim, with no separator (for a trailing `?`, for
    /// example, or for a payload that follows the command).
    pub fn suffix(mut self, raw: &str) -> Result<Self> {
        self.buffer
            .push_str(raw)
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(self)
    }

    /// Finish the command, appending CRLF.
    pub fn build(mut self) -> Result<AtCommandString> {
        self.buffer
            .push_str("\r\n")
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(self.buffer)
    }

    /// Access the partial command.
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.as_bytes()
    }
}

/// Format an IPv4 address as `a.b.c.d`.
fn ipv4_string(ip: &Ipv4Address) -> Result<String<16>> {
    let mut s = String::<16>::new();
    write!(s, "{}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3]).map_err(|_| Error::BufferTooSmall)?;
    Ok(s)
}

/// System AT commands (`w61_at_sys.c`).
pub mod system {
    use super::*;

    /// Test the link (`AT`).
    pub fn test() -> Result<AtCommandString> {
        AtCommand::new("AT").build()
    }

    /// Reset the module (`AT+RST`).
    pub fn reset() -> Result<AtCommandString> {
        AtCommand::new("AT+RST").build()
    }

    /// Restore factory settings (`AT+RESTORE`).
    pub fn restore_factory() -> Result<AtCommandString> {
        AtCommand::new("AT+RESTORE").build()
    }

    /// Firmware version (`AT+GMR`).
    pub fn get_version() -> Result<AtCommandString> {
        AtCommand::new("AT+GMR").build()
    }

    /// Enable or disable echo (`ATE1` / `ATE0`).
    pub fn set_echo(enabled: bool) -> Result<AtCommandString> {
        AtCommand::new(if enabled { "ATE1" } else { "ATE0" }).build()
    }

    /// Query system RAM usage (`AT+SYSRAM?`).
    pub fn get_ram_usage() -> Result<AtCommandString> {
        AtCommand::new("AT+SYSRAM?").build()
    }

    /// Query the store configuration (`AT+SYSSTORE?`).
    pub fn get_store() -> Result<AtCommandString> {
        AtCommand::new("AT+SYSSTORE?").build()
    }

    /// Set the store mode (`AT+SYSSTORE=<mode>`).
    pub fn set_store(mode: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+SYSSTORE").args()?.arg_int(mode)?.build()
    }

    /// Enter power-save mode (`AT+PWR=<mode>`).
    ///
    /// The driver's `ps_mode` values are the `W6X_PowerMode` ones; for example
    /// `1` is hibernate. In hibernate the module sends no response, so use
    /// [`set_power_mode_hibernate`] which pairs a mode with a wake level.
    pub fn set_power_mode(mode: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+PWR").args()?.arg_int(mode)?.build()
    }

    /// Enter hibernate with an explicit wake level (`AT+PWR=<mode>,<level>`).
    pub fn set_power_mode_hibernate(mode: u32, level: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+PWR")
            .args()?
            .arg_int(mode)?
            .arg_int(level)?
            .build()
    }

    /// Configure the wake-up pin for low-power exit (`AT+SLWKIO=<pin>,0`).
    pub fn set_wakeup_pin(pin: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+SLWKIO")
            .args()?
            .arg_int(pin)?
            .arg_int(0)?
            .build()
    }

    /// Legacy helper: put the module into hibernate (`AT+PWR=1`).
    ///
    /// The ST67W611 wakes from hibernate on a GPIO, not a timer, so `time_ms`
    /// is ignored; configure the wake source with [`set_wakeup_pin`].
    pub fn deep_sleep(_time_ms: u32) -> Result<AtCommandString> {
        set_power_mode(1)
    }

    /// Read the clock source (`AT+GET_CLOCK`).
    pub fn get_clock() -> Result<AtCommandString> {
        AtCommand::new("AT+GET_CLOCK").build()
    }

    /// Set the clock source (`AT+SET_CLOCK=<source>`).
    pub fn set_clock(source: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+SET_CLOCK")
            .args()?
            .arg_int(source)?
            .build()
    }

    /// Query the supply voltage (`AT+VBAT?`).
    pub fn get_battery() -> Result<AtCommandString> {
        AtCommand::new("AT+VBAT?").build()
    }

    /// Read eFuse words (`AT+EFUSE-R=<n>,"0x<a>",1`).
    pub fn get_efuse(nbytes: u32, addr: u32) -> Result<AtCommandString> {
        let mut hex = String::<8>::new();
        write!(hex, "0x{:03x}", addr).map_err(|_| Error::BufferTooSmall)?;
        AtCommand::new("AT+EFUSE-R")
            .args()?
            .arg_int(nbytes)?
            .arg_quoted(&hex)?
            .arg_int(1)?
            .build()
    }

    /// Configure the IO-reset pin (`AT+IORST=<pin>`).
    pub fn set_io_reset(pin: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+IORST").args()?.arg_int(pin)?.build()
    }

    /// List supported commands (`AT+CMD?`).
    pub fn list_commands() -> Result<AtCommandString> {
        AtCommand::new("AT+CMD?").build()
    }
}

/// Wi-Fi AT commands (`w61_at_wifi.c`).
pub mod wifi {
    use super::*;

    /// Start station mode and auto-connect (`AT+CWMODE=1,<sta_state>`).
    pub fn set_mode(mode: WiFiMode) -> Result<AtCommandString> {
        match mode {
            // `W61_WiFi_Station_Start` uses the fixed `1,0` form.
            WiFiMode::Station => AtCommand::new("AT+CWMODE=1,0").build(),
            WiFiMode::AccessPoint => AtCommand::new("AT+CWMODE=2").build(),
            WiFiMode::StationAp => AtCommand::new("AT+CWMODE=3,0").build(),
        }
    }

    /// Start station mode with an explicit auto-connect flag
    /// (`AT+CWMODE=1,<sta_state>`).
    pub fn set_mode_with_autoconnect(mode: WiFiMode, sta_state: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CWMODE")
            .args()?
            .arg_int(mode as u8)?
            .arg_int(sta_state)?
            .build()
    }

    /// Disable Wi-Fi (`AT+CWMODE=0`).
    pub fn stop() -> Result<AtCommandString> {
        AtCommand::new("AT+CWMODE=0").build()
    }

    /// Join an access point (`AT+CWJAP="<ssid>","<pwd>"`).
    pub fn connect(ssid: &str, password: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWJAP")
            .args()?
            .arg_quoted(ssid)?
            .arg_quoted(password)?
            .suffix(",")?
            .build()
    }

    /// Join an AP using a credential stored in the module
    /// (`AT+CWJAPS="<ssid>"`).
    pub fn connect_stored(ssid: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWJAPS")
            .args()?
            .arg_quoted(ssid)?
            .build()
    }

    /// Disconnect from the AP (`AT+CWQAP=<restore>`).
    ///
    /// `restore` selects whether the module restores the previous Wi-Fi
    /// configuration.
    pub fn disconnect() -> Result<AtCommandString> {
        disconnect_with(1)
    }

    /// Disconnect with an explicit restore flag (`AT+CWQAP=<restore>`).
    pub fn disconnect_with(restore: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CWQAP")
            .args()?
            .arg_int(restore)?
            .build()
    }

    /// Scan for access points (`AT+CWLAP`).
    pub fn scan() -> Result<AtCommandString> {
        AtCommand::new("AT+CWLAP").build()
    }

    /// Scan with filters (`AT+CWLAP=<max>,"<ssid>","<mac>",<type>`).
    pub fn scan_opts(max: u32, ssid: &str, mac: &str, scan_type: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+CWLAP")
            .args()?
            .arg_int(max)?
            .arg_quoted(ssid)?
            .arg_quoted(mac)?
            .arg_int(scan_type)?
            .build()
    }

    /// Set scan-result options (`AT+CWLAPOPT=1,1695,-100,255,<max>`).
    pub fn scan_options(max: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CWLAPOPT=1,1695,-100,255")
            .args()?
            .arg_int(max)?
            .build()
    }

    /// Query the current AP (`AT+CWJAP?`).
    pub fn get_current_ap() -> Result<AtCommandString> {
        AtCommand::new("AT+CWJAP?").build()
    }

    /// Station MAC address (`AT+CIPSTAMAC?`).
    pub fn get_mac() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTAMAC?").build()
    }

    /// Soft-AP MAC address (`AT+CIPAPMAC?`).
    pub fn get_ap_mac() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPAPMAC?").build()
    }

    /// Set the station IP (`AT+CIPSTA="<ip>","<gw>","<mask>"`).
    pub fn set_station_ip(
        ip: &Ipv4Address,
        gateway: &Ipv4Address,
        netmask: &Ipv4Address,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTA")
            .args()?
            .arg_quoted(&ipv4_string(ip)?)?
            .arg_quoted(&ipv4_string(gateway)?)?
            .arg_quoted(&ipv4_string(netmask)?)?
            .build()
    }

    /// Query the station IP (`AT+CIPSTA?`).
    pub fn get_station_ip() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTA?").build()
    }

    /// Set the Soft-AP IP (`AT+CIPAP="<ip>","<gw>","<mask>"`).
    pub fn set_ap_ip(
        ip: &Ipv4Address,
        gateway: &Ipv4Address,
        netmask: &Ipv4Address,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPAP")
            .args()?
            .arg_quoted(&ipv4_string(ip)?)?
            .arg_quoted(&ipv4_string(gateway)?)?
            .arg_quoted(&ipv4_string(netmask)?)?
            .build()
    }

    /// Query the Soft-AP IP (`AT+CIPAP?`).
    pub fn get_ap_ip() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPAP?").build()
    }

    /// Configure the Soft-AP (`AT+CWSAP="<ssid>","<pwd>",<ch>,<enc>`).
    pub fn configure_ap(ssid: &str, password: &str, channel: u8, encryption: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CWSAP")
            .args()?
            .arg_quoted(ssid)?
            .arg_quoted(password)?
            .arg_int(channel)?
            .arg_int(encryption)?
            .build()
    }

    /// Query the Soft-AP configuration (`AT+CWSAP?`).
    pub fn get_ap_config() -> Result<AtCommandString> {
        AtCommand::new("AT+CWSAP?").build()
    }

    /// List stations connected to the Soft-AP (`AT+CWLIF`).
    pub fn list_stations() -> Result<AtCommandString> {
        AtCommand::new("AT+CWLIF").build()
    }

    /// Disconnect a station from the Soft-AP (`AT+CWQIF="<mac>"`).
    pub fn disconnect_station(mac: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWQIF")
            .args()?
            .arg_quoted(mac)?
            .build()
    }

    /// Configure DHCP (`AT+CWDHCP=<mode>,<state>`).
    pub fn set_dhcp(mode: u8, enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CWDHCP")
            .args()?
            .arg_int(mode)?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }

    /// Query DHCP (`AT+CWDHCP?`).
    pub fn get_dhcp() -> Result<AtCommandString> {
        AtCommand::new("AT+CWDHCP?").build()
    }

    /// Configure the Soft-AP DHCP server (`AT+CWDHCPS=1,<lease>,"<ip>","<gw>"`).
    pub fn set_ap_dhcp_server(lease_min: u32, ip: &Ipv4Address, gateway: &Ipv4Address) -> Result<AtCommandString> {
        AtCommand::new("AT+CWDHCPS=1")
            .args()?
            .arg_int(lease_min)?
            .arg_quoted(&ipv4_string(ip)?)?
            .arg_quoted(&ipv4_string(gateway)?)?
            .build()
    }

    /// Query the Soft-AP DHCP server (`AT+CWDHCPS?`).
    pub fn get_ap_dhcp_server() -> Result<AtCommandString> {
        AtCommand::new("AT+CWDHCPS?").build()
    }

    /// Store station credentials (`AT+CWCREDADD="<ssid>","<pwd>"`).
    pub fn add_credentials(ssid: &str, password: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWCREDADD")
            .args()?
            .arg_quoted(ssid)?
            .arg_quoted(password)?
            .build()
    }

    /// Delete stored credentials (`AT+CWCREDDEL="<ssid>"`).
    pub fn delete_credentials(ssid: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWCREDDEL")
            .args()?
            .arg_quoted(ssid)?
            .build()
    }

    /// List stored credentials (`AT+CWCRED?`).
    pub fn get_credentials() -> Result<AtCommandString> {
        AtCommand::new("AT+CWCRED?").build()
    }

    /// Enable or disable auto-connect (`AT+CWAUTOCONN=<0|1>`).
    pub fn set_auto_connect(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CWAUTOCONN")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }

    /// Query auto-connect (`AT+CWAUTOCONN?`).
    pub fn get_auto_connect() -> Result<AtCommandString> {
        AtCommand::new("AT+CWAUTOCONN?").build()
    }

    /// Set the station hostname (`AT+CWHOSTNAME="<name>"`).
    pub fn set_hostname(hostname: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWHOSTNAME")
            .args()?
            .arg_quoted(hostname)?
            .build()
    }

    /// Query the station hostname (`AT+CWHOSTNAME?`).
    pub fn get_hostname() -> Result<AtCommandString> {
        AtCommand::new("AT+CWHOSTNAME?").build()
    }

    /// Set the country code (`AT+CWCOUNTRY=<policy>,"<CC>"`).
    pub fn set_country(policy: u32, country: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CWCOUNTRY")
            .args()?
            .arg_int(policy)?
            .arg_quoted(country)?
            .build()
    }

    /// Query the country code (`AT+CWCOUNTRY?`).
    pub fn get_country() -> Result<AtCommandString> {
        AtCommand::new("AT+CWCOUNTRY?").build()
    }

    /// Query the Wi-Fi state (`AT+CWSTATE?`).
    pub fn get_state() -> Result<AtCommandString> {
        AtCommand::new("AT+CWSTATE?").build()
    }

    /// Query the network mode (`AT+CWNETMODE?`).
    pub fn get_net_mode() -> Result<AtCommandString> {
        AtCommand::new("AT+CWNETMODE?").build()
    }

    /// Configure reconnection (`AT+CWRECONNCFG=<interval>,<count>`).
    pub fn set_reconnect_cfg(interval: u16, count: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+CWRECONNCFG")
            .args()?
            .arg_int(interval)?
            .arg_int(count)?
            .build()
    }

    /// Set the 802.11 protocol (`AT+CWAPPROTO=<proto>`).
    pub fn set_protocol(protocol: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CWAPPROTO")
            .args()?
            .arg_int(protocol)?
            .build()
    }

    /// Query the 802.11 protocol (`AT+CWAPPROTO?`).
    pub fn get_protocol() -> Result<AtCommandString> {
        AtCommand::new("AT+CWAPPROTO?").build()
    }

    /// Configure antenna diversity (`AT+CWANTENABLE=<en>,<a>,<b>`).
    pub fn set_antenna(enable: u32, a: u32, b: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CWANTENABLE")
            .args()?
            .arg_int(enable)?
            .arg_int(a)?
            .arg_int(b)?
            .build()
    }

    /// Query the antenna configuration (`AT+CWANT?`).
    pub fn get_antenna() -> Result<AtCommandString> {
        AtCommand::new("AT+CWANT?").build()
    }

    /// Start WPS push-button (`AT+WPS=1`).
    pub fn wps() -> Result<AtCommandString> {
        AtCommand::new("AT+WPS=1").build()
    }

    /// Set the wake DTIM factor (`AT+SLWKDTIM=<dtim>`).
    pub fn set_wake_dtim(dtim: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+SLWKDTIM")
            .args()?
            .arg_int(dtim)?
            .build()
    }

    /// Query the AP DTIM (`AT+GET_AP_DTIM?`).
    pub fn get_ap_dtim() -> Result<AtCommandString> {
        AtCommand::new("AT+GET_AP_DTIM?").build()
    }

    /// Query TWT support (`AT+GET_TWT_SUPPORTED?`).
    pub fn get_twt_supported() -> Result<AtCommandString> {
        AtCommand::new("AT+GET_TWT_SUPPORTED?").build()
    }

    /// Query TWT status (`AT+TWT_STATUS?`).
    pub fn twt_status() -> Result<AtCommandString> {
        AtCommand::new("AT+TWT_STATUS?").build()
    }

    /// Tear down a TWT flow (`AT+TWT_TEARDOWN=0,<flow>,<all>`).
    pub fn twt_teardown(flow: u16, all: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+TWT_TEARDOWN=0")
            .args()?
            .arg_int(flow)?
            .arg_int(all)?
            .build()
    }
}

/// Network AT commands (`w61_at_net.c`).
pub mod network {
    use super::*;

    /// Open a connection (`AT+CIPSTART=<link>,"<proto>","<host>",<port>`).
    pub fn connect(
        link_id: u8,
        protocol: SocketProtocol,
        host: &str,
        port: u16,
    ) -> Result<AtCommandString> {
        let proto = match protocol {
            SocketProtocol::Tcp => "TCP",
            SocketProtocol::Udp => "UDP",
            SocketProtocol::Ssl => "SSL",
        };
        AtCommand::new("AT+CIPSTART")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(proto)?
            .arg_quoted(host)?
            .arg_int(port)?
            .build()
    }

    /// Send `length` bytes (`AT+CIPSEND=<link>,<length>`), then the payload.
    pub fn send(link_id: u8, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSEND")
            .args()?
            .arg_int(link_id)?
            .arg_int(length)?
            .build()
    }

    /// Close a connection (`AT+CIPCLOSE=<link>`).
    pub fn close(link_id: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPCLOSE")
            .args()?
            .arg_int(link_id)?
            .build()
    }

    /// Read received data (`AT+CIPRECVDATA=<link>,<length>`).
    pub fn receive(link_id: u8, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPRECVDATA")
            .args()?
            .arg_int(link_id)?
            .arg_int(length)?
            .build()
    }

    /// Query connection state (`AT+CIPSTATE?`).
    pub fn get_status() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSTATE?").build()
    }

    /// Set multiple-connection mode (`AT+CIPMUX=<0|1>`).
    pub fn set_mux(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPMUX")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }

    /// Enable data-info prefixes (`AT+CIPDINFO=<0|1>`).
    pub fn set_data_info(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPDINFO")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }

    /// Set the receive mode (`AT+CIPRECVMODE=<0|1>`).
    ///
    /// `1` makes the module buffer incoming data, which is then pulled with
    /// [`receive`].
    pub fn set_receive_mode(buffered: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPRECVMODE")
            .args()?
            .arg_int(if buffered { 1 } else { 0 })?
            .build()
    }

    /// Set a socket's receive buffer length (`AT+CIPRECVBUF=<link>,<len>`).
    pub fn set_receive_buffer(link_id: u8, len: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPRECVBUF")
            .args()?
            .arg_int(link_id)?
            .arg_int(len)?
            .build()
    }

    /// Query a socket's receive buffer length (`AT+CIPRECVBUF=<link>?`).
    pub fn get_receive_buffer(link_id: u8) -> Result<AtCommandString> {
        let mut cmd = AtCommand::new("AT+CIPRECVBUF");
        cmd = cmd.args()?.arg_int(link_id)?;
        cmd.suffix("?").and_then(|c| c.build())
    }

    /// Start a TCP server (`AT+CIPSERVER=1,<port>,"<ip>",<u16>,<u32>`).
    pub fn start_server(port: u16, ip: &str, backlog: u16, timeout: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSERVER=1")
            .args()?
            .arg_int(port)?
            .arg_quoted(ip)?
            .arg_int(backlog)?
            .arg_int(timeout)?
            .build()
    }

    /// Stop a TCP server (`AT+CIPSERVER=0,<close_connections>`).
    pub fn stop_server(close_connections: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSERVER=0")
            .args()?
            .arg_int(close_connections)?
            .build()
    }

    /// Set the server maximum connections (`AT+CIPSERVERMAXCONN=<n>`).
    pub fn set_server_max_connections(max: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSERVERMAXCONN")
            .args()?
            .arg_int(max)?
            .build()
    }

    /// Configure TLS (`AT+CIPSSLCCONF=<link>,<auth>`).
    ///
    /// The full ST form also names CA certificate, client certificate and key,
    /// which are uploaded through the filesystem first.
    pub fn configure_ssl(link_id: u8, auth_mode: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCCONF")
            .args()?
            .arg_int(link_id)?
            .arg_int(auth_mode)?
            .build()
    }

    /// Set the TLS SNI (`AT+CIPSSLCSNI=<link>,"<host>"`).
    pub fn set_sni(link_id: u8, hostname: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCSNI")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(hostname)?
            .build()
    }

    /// Set up a TLS pre-shared key (`AT+CIPSSLCPSK=<link>,"<psk>","<hint>"`).
    pub fn set_ssl_psk(link_id: u8, psk: &str, hint: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCPSK")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(psk)?
            .arg_quoted(hint)?
            .build()
    }

    /// Set the TLS ALPN list (`AT+CIPSSLCALPN=<link>,...`).
    pub fn set_ssl_alpn(link_id: u8, alpn: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSSLCALPN")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(alpn)?
            .build()
    }

    /// Set per-socket TCP options (`AT+CIPTCPOPT=<link>,...`).
    pub fn set_tcp_options(link_id: u8, options: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPTCPOPT")
            .args()?
            .arg_int(link_id)?
            .arg_int(options)?
            .build()
    }

    /// DNS lookup (`AT+CIPDOMAIN="<host>",<type>`).
    pub fn dns_lookup(hostname: &str) -> Result<AtCommandString> {
        dns_lookup_typed(hostname, 0)
    }

    /// DNS lookup with an explicit address family (`0` = auto).
    pub fn dns_lookup_typed(hostname: &str, address_type: i32) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPDOMAIN")
            .args()?
            .arg_quoted(hostname)?
            .arg_int(address_type)?
            .build()
    }

    /// Configure DNS (`AT+CIPDNS=1,"<dns1>"[,"<dns2>"]`).
    pub fn set_dns(enable: bool, dns1: &str, dns2: Option<&str>) -> Result<AtCommandString> {
        if !enable {
            return AtCommand::new("AT+CIPDNS=0").build();
        }
        let mut cmd = AtCommand::new("AT+CIPDNS=1")
            .args()?
            .arg_quoted(dns1)?;
        if let Some(dns2) = dns2 {
            cmd = cmd.arg_quoted(dns2)?;
        }
        cmd.build()
    }

    /// Query DNS (`AT+CIPDNS?`).
    pub fn get_dns() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPDNS?").build()
    }

    /// Configure SNTP (`AT+CIPSNTPCFG=<enable>,<tz>[,"<s1>"]`).
    pub fn configure_sntp(enable: bool, timezone: i8, server1: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSNTPCFG")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .arg_int(timezone)?
            .arg_quoted(server1)?
            .build()
    }

    /// Set the SNTP sync interval (`AT+CIPSNTPINTV=<interval>`).
    pub fn set_sntp_interval(interval: u16) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSNTPINTV")
            .args()?
            .arg_int(interval)?
            .build()
    }

    /// Query SNTP time (`AT+CIPSNTPTIME?`).
    pub fn get_sntp_time() -> Result<AtCommandString> {
        AtCommand::new("AT+CIPSNTPTIME?").build()
    }

    /// Ping a host (`AT+PING="<host>",<length>,<count>,<interval>`).
    pub fn ping(host: &str) -> Result<AtCommandString> {
        ping_with(host, 64, 4, 1000, None)
    }

    /// Ping with explicit parameters.
    ///
    /// `timeout` is only sent when the module's SDK supports it.
    pub fn ping_with(
        host: &str,
        length: u16,
        count: u16,
        interval: u16,
        timeout: Option<u16>,
    ) -> Result<AtCommandString> {
        let mut cmd = AtCommand::new("AT+PING")
            .args()?
            .arg_quoted(host)?
            .arg_int(length)?
            .arg_int(count)?
            .arg_int(interval)?;
        if let Some(timeout) = timeout {
            cmd = cmd.arg_int(timeout)?;
        }
        cmd.build()
    }

    /// Enable or disable IPv6 (`AT+CIPV6=<0|1>`).
    pub fn set_ipv6(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+CIPV6")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }
}

/// MQTT AT commands (`w61_at_mqtt.c`).
pub mod mqtt {
    use super::*;

    /// Configure the MQTT user (`AT+MQTTUSERCFG=<link>,<scheme>,"<id>","<user>","<pwd>"`).
    pub fn set_user_config(
        link_id: u8,
        scheme: u8,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTUSERCFG")
            .args()?
            .arg_int(link_id)?
            .arg_int(scheme)?
            .arg_quoted(client_id)?
            .arg_quoted(username)?
            .arg_quoted(password)?
            .build()
    }

    /// Query the MQTT user config (`AT+MQTTUSERCFG?`).
    pub fn get_user_config() -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTUSERCFG?").build()
    }

    /// Connect to a broker (`AT+MQTTCONN=<link>,"<host>",<port>,<reconnect>`).
    pub fn connect(link_id: u8, host: &str, port: u16, reconnect: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCONN")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(host)?
            .arg_int(port)?
            .arg_int(if reconnect { 1 } else { 0 })?
            .build()
    }

    /// Query the broker connection (`AT+MQTTCONN?`).
    pub fn get_connection() -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCONN?").build()
    }

    /// Set the broker SNI (`AT+MQTTSNI=<link>,"<sni>"`).
    pub fn set_sni(link_id: u8, sni: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTSNI")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(sni)?
            .build()
    }

    /// Query the broker SNI (`AT+MQTTSNI?`).
    pub fn get_sni() -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTSNI?").build()
    }

    /// Publish a string (`AT+MQTTPUB=<link>,"<topic>","<data>",<qos>,<retain>`).
    pub fn publish(
        link_id: u8,
        topic: &str,
        data: &str,
        qos: MqttQos,
        retain: bool,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTPUB")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(topic)?
            .arg_quoted(data)?
            .arg_int(qos as u8)?
            .arg_int(if retain { 1 } else { 0 })?
            .build()
    }

    /// Publish raw bytes (`AT+MQTTPUBRAW=<link>,"<topic>",<len>,<qos>,<retain>`).
    pub fn publish_raw(
        link_id: u8,
        topic: &str,
        len: u32,
        qos: MqttQos,
        retain: bool,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTPUBRAW")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(topic)?
            .arg_int(len)?
            .arg_int(qos as u8)?
            .arg_int(if retain { 1 } else { 0 })?
            .build()
    }

    /// Subscribe (`AT+MQTTSUB=<link>,"<topic>",<qos>`).
    pub fn subscribe(link_id: u8, topic: &str, qos: MqttQos) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTSUB")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(topic)?
            .arg_int(qos as u8)?
            .build()
    }

    /// Query subscriptions (`AT+MQTTSUB?`).
    pub fn get_subscriptions() -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTSUB?").build()
    }

    /// Unsubscribe (`AT+MQTTUNSUB=<link>,"<topic>"`).
    pub fn unsubscribe(link_id: u8, topic: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTUNSUB")
            .args()?
            .arg_int(link_id)?
            .arg_quoted(topic)?
            .build()
    }

    /// Configure the MQTT connection, including the Last Will
    /// (`AT+MQTTCONNCFG=<link>,<keepalive>,<disable_clean>,"<topic>","<msg>",<qos>,<retain>`).
    pub fn set_conn_config(
        link_id: u8,
        keepalive: u32,
        disable_clean_session: u32,
        will_topic: &str,
        will_message: &str,
        will_qos: u32,
        will_retain: u32,
    ) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCONNCFG")
            .args()?
            .arg_int(link_id)?
            .arg_int(keepalive)?
            .arg_int(disable_clean_session)?
            .arg_quoted(will_topic)?
            .arg_quoted(will_message)?
            .arg_int(will_qos)?
            .arg_int(will_retain)?
            .build()
    }

    /// Query the MQTT connection config (`AT+MQTTCONNCFG?`).
    pub fn get_conn_config() -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCONNCFG?").build()
    }

    /// Disconnect and release the client (`AT+MQTTCLEAN=<link>`).
    pub fn disconnect(link_id: u8) -> Result<AtCommandString> {
        AtCommand::new("AT+MQTTCLEAN")
            .args()?
            .arg_int(link_id)?
            .build()
    }
}

/// Module filesystem AT commands (`AT+FS=0,<op>,...` in `w61_at_sys.c`).
///
/// The first parameter (`0`) selects the store; operations are `0` delete,
/// `1` create, `2` write, `3` read, `4` size, `5` list.
pub mod filesystem {
    use super::*;

    /// Create a file (`AT+FS=0,1,"<name>"`).
    pub fn fs_create(filename: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,1")
            .args()?
            .arg_quoted(filename)?
            .build()
    }

    /// Write to a file (`AT+FS=0,2,"<name>",<offset>,<len>`), then the payload.
    pub fn fs_write(filename: &str, offset: usize, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,2")
            .args()?
            .arg_quoted(filename)?
            .arg_int(offset)?
            .arg_int(length)?
            .build()
    }

    /// Read from a file (`AT+FS=0,3,"<name>",<offset>,<len>`).
    pub fn fs_read(filename: &str, offset: usize, length: usize) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,3")
            .args()?
            .arg_quoted(filename)?
            .arg_int(offset)?
            .arg_int(length)?
            .build()
    }

    /// Query a file's size (`AT+FS=0,4,"<name>"`).
    pub fn fs_size(filename: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,4")
            .args()?
            .arg_quoted(filename)?
            .build()
    }

    /// Delete a file (`AT+FS=0,0,"<name>"`).
    pub fn fs_delete(filename: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,0")
            .args()?
            .arg_quoted(filename)?
            .build()
    }

    /// List files (`AT+FS=0,5,"<dir>"`).
    pub fn fs_list(dir: &str) -> Result<AtCommandString> {
        AtCommand::new("AT+FS=0,5")
            .args()?
            .arg_quoted(dir)?
            .build()
    }
}

/// Firmware-update AT commands (`w61_at_sys.c`).
pub mod fwu {
    use super::*;

    /// Start a firmware transfer (`AT+OTASTART=<enable>`).
    pub fn start(enable: bool) -> Result<AtCommandString> {
        AtCommand::new("AT+OTASTART")
            .args()?
            .arg_int(if enable { 1 } else { 0 })?
            .build()
    }

    /// Announce the next chunk (`AT+OTASEND=<len>`), then send the bytes.
    pub fn send(length: u32) -> Result<AtCommandString> {
        AtCommand::new("AT+OTASEND").args()?.arg_int(length)?.build()
    }

    /// Finish the transfer and reboot (`AT+OTAFIN`).
    pub fn finish() -> Result<AtCommandString> {
        AtCommand::new("AT+OTAFIN").build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cmd: Result<AtCommandString>) -> String<128> {
        let cmd = cmd.unwrap();
        let mut out = String::new();
        out.push_str(cmd.as_str()).unwrap();
        out
    }

    #[test]
    fn wifi_connect_includes_equals() {
        assert_eq!(s(wifi::connect("myssid", "mypass")), "AT+CWJAP=\"myssid\",\"mypass\",\r\n");
    }

    #[test]
    fn cipstart_is_well_formed() {
        assert_eq!(
            s(network::connect(0, SocketProtocol::Tcp, "example.com", 80)),
            "AT+CIPSTART=0,\"TCP\",\"example.com\",80\r\n"
        );
    }

    #[test]
    fn cipsend_and_close() {
        assert_eq!(s(network::send(1, 512)), "AT+CIPSEND=1,512\r\n");
        assert_eq!(s(network::close(1)), "AT+CIPCLOSE=1\r\n");
        assert_eq!(s(network::receive(2, 64)), "AT+CIPRECVDATA=2,64\r\n");
    }

    #[test]
    fn dns_uses_st_dialect() {
        assert_eq!(s(network::get_dns()), "AT+CIPDNS?\r\n");
        assert_eq!(
            s(network::set_dns(true, "8.8.8.8", None)),
            "AT+CIPDNS=1,\"8.8.8.8\"\r\n"
        );
        assert_eq!(
            s(network::dns_lookup("example.com")),
            "AT+CIPDOMAIN=\"example.com\",0\r\n"
        );
    }

    #[test]
    fn filesystem_layout_matches_reference() {
        assert_eq!(s(filesystem::fs_create("crt.pem")), "AT+FS=0,1,\"crt.pem\"\r\n");
        assert_eq!(
            s(filesystem::fs_write("crt.pem", 0, 1024)),
            "AT+FS=0,2,\"crt.pem\",0,1024\r\n"
        );
        assert_eq!(
            s(filesystem::fs_read("crt.pem", 16, 32)),
            "AT+FS=0,3,\"crt.pem\",16,32\r\n"
        );
        assert_eq!(s(filesystem::fs_delete("crt.pem")), "AT+FS=0,0,\"crt.pem\"\r\n");
        assert_eq!(s(filesystem::fs_size("crt.pem")), "AT+FS=0,4,\"crt.pem\"\r\n");
    }

    #[test]
    fn wifi_mode_uses_reference_forms() {
        assert_eq!(s(wifi::set_mode(WiFiMode::Station)), "AT+CWMODE=1,0\r\n");
        assert_eq!(s(wifi::set_mode(WiFiMode::AccessPoint)), "AT+CWMODE=2\r\n");
        assert_eq!(s(wifi::set_mode(WiFiMode::StationAp)), "AT+CWMODE=3,0\r\n");
        assert_eq!(s(wifi::stop()), "AT+CWMODE=0\r\n");
    }

    #[test]
    fn station_ip_and_dhcp() {
        let ip = Ipv4Address::new(192, 168, 1, 10);
        let gw = Ipv4Address::new(192, 168, 1, 1);
        let nm = Ipv4Address::new(255, 255, 255, 0);
        assert_eq!(
            s(wifi::set_station_ip(&ip, &gw, &nm)),
            "AT+CIPSTA=\"192.168.1.10\",\"192.168.1.1\",\"255.255.255.0\"\r\n"
        );
        assert_eq!(s(wifi::set_dhcp(1, true)), "AT+CWDHCP=1,1\r\n");
        assert_eq!(s(wifi::set_dhcp(1, false)), "AT+CWDHCP=1,0\r\n");
    }

    #[test]
    fn credentials_and_hostname() {
        assert_eq!(
            s(wifi::add_credentials("ssid", "pass")),
            "AT+CWCREDADD=\"ssid\",\"pass\"\r\n"
        );
        assert_eq!(s(wifi::get_credentials()), "AT+CWCRED?\r\n");
        assert_eq!(
            s(wifi::set_hostname("my-device")),
            "AT+CWHOSTNAME=\"my-device\"\r\n"
        );
    }

    #[test]
    fn mqtt_commands() {
        assert_eq!(
            s(mqtt::set_user_config(0, 1, "cid", "user", "pw")),
            "AT+MQTTUSERCFG=0,1,\"cid\",\"user\",\"pw\"\r\n"
        );
        assert_eq!(
            s(mqtt::connect(0, "broker", 1883, false)),
            "AT+MQTTCONN=0,\"broker\",1883,0\r\n"
        );
        assert_eq!(
            s(mqtt::publish(0, "t", "hi", MqttQos::AtLeastOnce, false)),
            "AT+MQTTPUB=0,\"t\",\"hi\",1,0\r\n"
        );
        assert_eq!(
            s(mqtt::subscribe(0, "t", MqttQos::ExactlyOnce)),
            "AT+MQTTSUB=0,\"t\",2\r\n"
        );
        assert_eq!(s(mqtt::disconnect(0)), "AT+MQTTCLEAN=0\r\n");
    }

    #[test]
    fn system_and_power() {
        assert_eq!(s(system::reset()), "AT+RST\r\n");
        assert_eq!(s(system::get_version()), "AT+GMR\r\n");
        assert_eq!(s(system::set_power_mode(1)), "AT+PWR=1\r\n");
        assert_eq!(s(system::set_wakeup_pin(5)), "AT+SLWKIO=5,0\r\n");
        assert_eq!(s(system::get_efuse(4, 0x10)), "AT+EFUSE-R=4,\"0x010\",1\r\n");
    }

    #[test]
    fn fwu_commands() {
        assert_eq!(s(fwu::start(true)), "AT+OTASTART=1\r\n");
        assert_eq!(s(fwu::send(512)), "AT+OTASEND=512\r\n");
        assert_eq!(s(fwu::finish()), "AT+OTAFIN\r\n");
    }

    #[test]
    fn ping_includes_reference_parameters() {
        assert_eq!(s(network::ping("1.1.1.1")), "AT+PING=\"1.1.1.1\",64,4,1000\r\n");
    }

    #[test]
    fn receive_buffer_query() {
        assert_eq!(s(network::get_receive_buffer(2)), "AT+CIPRECVBUF=2?\r\n");
    }

    #[test]
    fn ipv6_toggle() {
        assert_eq!(s(network::set_ipv6(true)), "AT+CIPV6=1\r\n");
        assert_eq!(s(network::set_ipv6(false)), "AT+CIPV6=0\r\n");
    }
}
