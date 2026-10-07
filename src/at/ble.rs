//! Bluetooth LE AT commands (`Driver/W61_at/w61_at_ble.c`).
//!
//! Command formats are transcribed from the reference driver and pinned by unit
//! tests. Commands whose payload follows the AT line (`+BLEGATTSNTFY`,
//! `+BLEGATTSIND`, `+BLEGATTCWR`, `+BLEGATTSRD`) end with a trailing comma, as
//! the reference does; the bytes are sent as the next SPI write.
//!
//! ```
//! use st67w611::at::ble;
//!
//! let cmd = ble::set_name("sensor").unwrap();
//! assert_eq!(cmd.as_str(), "AT+BLENAME=\"sensor\"\r\n");
//! ```

use core::fmt::Write as _;
use heapless::String;

use crate::at::command::{AtCommand, AtCommandString};
use crate::error::{Error, Result};

/// BLE role selected with [`init`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum BleMode {
    /// GATT client only.
    Client = 1,
    /// GATT server only.
    Server = 2,
    /// GATT client and server.
    Dual = 3,
}

/// Characteristic property bits (`W6X_BLE_CHAR_PROP_*`).
pub mod char_prop {
    /// Characteristic may be read.
    pub const READ: u16 = 2;
    /// Characteristic may be written without a response.
    pub const WRITE_WITHOUT_RESPONSE: u16 = 4;
    /// Characteristic may be written with a response.
    pub const WRITE_WITH_RESPONSE: u16 = 8;
    /// Characteristic supports notifications.
    pub const NOTIFY: u16 = 16;
    /// Characteristic supports indications.
    pub const INDICATE: u16 = 32;
}

/// Characteristic permission bits (`W6X_BLE_CHAR_PERM_*`).
pub mod char_perm {
    /// Characteristic may be read.
    pub const READ: u16 = 1;
    /// Characteristic may be written.
    pub const WRITE: u16 = 2;
}

/// UUID type (`W6X_BLE_UUID_TYPE_*`).
pub mod uuid_type {
    /// 16-bit UUID.
    pub const UUID16: u16 = 0;
    /// 128-bit UUID.
    pub const UUID128: u16 = 2;
}

/// Render a little-endian MAC as `aa:bb:cc:dd:ee:ff`.
fn mac_string(mac: &[u8; 6]) -> Result<String<18>> {
    let mut s = String::<18>::new();
    write!(
        s,
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
    .map_err(|_| Error::BufferTooSmall)?;
    Ok(s)
}

/// Initialise BLE (`AT+BLEINIT=<mode>`).
pub fn init(mode: BleMode) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEINIT")
        .args()?
        .arg_int(mode as u16)?
        .build()
}

/// De-initialise BLE (`AT+BLEINIT=0`).
pub fn deinit() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEINIT=0").build()
}

/// Query the BLE init mode (`AT+BLEINIT?`).
pub fn get_init_mode() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEINIT?").build()
}

/// Set the TX power (`AT+BLETXPWR=<power>`).
pub fn set_tx_power(power: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLETXPWR")
        .args()?
        .arg_int(power)?
        .build()
}

/// Query the TX power (`AT+BLETXPWR?`).
pub fn get_tx_power() -> Result<AtCommandString> {
    AtCommand::new("AT+BLETXPWR?").build()
}

/// Set the device name (`AT+BLENAME="<name>"`).
pub fn set_name(name: &str) -> Result<AtCommandString> {
    AtCommand::new("AT+BLENAME")
        .args()?
        .arg_quoted(name)?
        .build()
}

/// Query the device name (`AT+BLENAME?`).
pub fn get_name() -> Result<AtCommandString> {
    AtCommand::new("AT+BLENAME?").build()
}

/// Set the BD address (`AT+BLEADDR="<aa:bb:cc:dd:ee:ff>"`).
pub fn set_bd_address(mac: &[u8; 6]) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADDR")
        .args()?
        .arg_quoted(&mac_string(mac)?)?
        .build()
}

/// Query the BD address (`AT+BLEADDR?`).
pub fn get_bd_address() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADDR?").build()
}

/// Set advertising parameters
/// (`AT+BLEADVPARAM=<int_min>,<int_max>,<type>,<channel>`).
pub fn set_adv_param(
    interval_min: u32,
    interval_max: u32,
    adv_type: u16,
    channel: u16,
) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADVPARAM")
        .args()?
        .arg_int(interval_min)?
        .arg_int(interval_max)?
        .arg_int(adv_type)?
        .arg_int(channel)?
        .build()
}

/// Query advertising parameters (`AT+BLEADVPARAM?`).
pub fn get_adv_param() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADVPARAM?").build()
}

/// Set advertising data (`AT+BLEADVDATA="<hex>"`).
pub fn set_adv_data(data: &str) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADVDATA")
        .args()?
        .arg_quoted(data)?
        .build()
}

/// Set scan-response data (`AT+BLESCANRSPDATA="<hex>"`).
pub fn set_scan_rsp_data(data: &str) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESCANRSPDATA")
        .args()?
        .arg_quoted(data)?
        .build()
}

/// Start advertising (`AT+BLEADVSTART`).
pub fn adv_start() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADVSTART").build()
}

/// Stop advertising (`AT+BLEADVSTOP`).
pub fn adv_stop() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEADVSTOP").build()
}

/// Set scan parameters
/// (`AT+BLESCANPARAM=<type>,<own_addr>,<filter>,<interval>,<window>`).
pub fn set_scan_param(
    scan_type: u16,
    own_addr_type: u16,
    filter_policy: u16,
    scan_interval: u32,
    scan_window: u32,
) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESCANPARAM")
        .args()?
        .arg_int(scan_type)?
        .arg_int(own_addr_type)?
        .arg_int(filter_policy)?
        .arg_int(scan_interval)?
        .arg_int(scan_window)?
        .build()
}

/// Query scan parameters (`AT+BLESCANPARAM?`).
pub fn get_scan_param() -> Result<AtCommandString> {
    AtCommand::new("AT+BLESCANPARAM?").build()
}

/// Start or stop scanning (`AT+BLESCAN=<0|1>`).
pub fn scan(enable: bool) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESCAN")
        .args()?
        .arg_int(if enable { 1 } else { 0 })?
        .build()
}

/// Connect to a peer (`AT+BLECONN=<conn>,"<aa:bb:cc:dd:ee:ff>"`).
pub fn connect(conn_handle: u32, mac: &[u8; 6]) -> Result<AtCommandString> {
    AtCommand::new("AT+BLECONN")
        .args()?
        .arg_int(conn_handle)?
        .arg_quoted(&mac_string(mac)?)?
        .build()
}

/// Query the connection (`AT+BLECONN?`).
pub fn get_connection() -> Result<AtCommandString> {
    AtCommand::new("AT+BLECONN?").build()
}

/// Set connection parameters
/// (`AT+BLECONNPARAM=<conn>,<int_min>,<int_max>,<latency>,<timeout>`).
pub fn set_conn_param(
    conn_handle: u32,
    interval_min: u32,
    interval_max: u32,
    latency: u32,
    timeout: u32,
) -> Result<AtCommandString> {
    AtCommand::new("AT+BLECONNPARAM")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(interval_min)?
        .arg_int(interval_max)?
        .arg_int(latency)?
        .arg_int(timeout)?
        .build()
}

/// Query connection parameters (`AT+BLECONNPARAM?`).
pub fn get_conn_param() -> Result<AtCommandString> {
    AtCommand::new("AT+BLECONNPARAM?").build()
}

/// Disconnect a peer (`AT+BLEDISCONN=<conn>`).
pub fn disconnect(conn_handle: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEDISCONN")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Exchange MTU (`AT+BLEEXCHANGEMTU=<conn>`).
pub fn exchange_mtu(conn_handle: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEEXCHANGEMTU")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Set the data length (`AT+BLEDATALEN=<conn>,<tx_bytes>,<tx_time>`).
pub fn set_data_length(conn_handle: u32, tx_bytes: u32, tx_time: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEDATALEN")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(tx_bytes)?
        .arg_int(tx_time)?
        .build()
}

/// Create a GATT service (`AT+BLEGATTSSRVCRE=<idx>,"<uuid>",1,<uuid_type>`).
pub fn gatts_create_service(service_index: u16, uuid: &str, uuid_type: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSSRVCRE")
        .args()?
        .arg_int(service_index)?
        .arg_quoted(uuid)?
        .arg_int(1)?
        .arg_int(uuid_type)?
        .build()
}

/// Delete a GATT service (`AT+BLEGATTSSRVDEL=<idx>`).
pub fn gatts_delete_service(service_index: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSSRVDEL")
        .args()?
        .arg_int(service_index)?
        .build()
}

/// List local services (`AT+BLEGATTSSRV?`).
pub fn gatts_get_services() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSSRV?").build()
}

/// Create a GATT characteristic
/// (`AT+BLEGATTSCHARCRE=<svc>,<char>,"<uuid>",<prop>,<perm>,<uuid_type>`).
pub fn gatts_create_char(
    service_index: u16,
    char_index: u16,
    uuid: &str,
    property: u16,
    permission: u16,
    uuid_type: u16,
) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSCHARCRE")
        .args()?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .arg_quoted(uuid)?
        .arg_int(property)?
        .arg_int(permission)?
        .arg_int(uuid_type)?
        .build()
}

/// List local characteristics (`AT+BLEGATTSCHAR?`).
pub fn gatts_get_chars() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSCHAR?").build()
}

/// Register characteristics, then start the server (`AT+BLEGATTSREGISTER=1`).
pub fn gatts_register() -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSREGISTER=1").build()
}

/// Notify a characteristic (`AT+BLEGATTSNTFY=<svc>,<char>,<len>,` then data).
pub fn gatts_notify(service_index: u16, char_index: u16, len: u32) -> Result<AtCommandString> {
    let cmd = AtCommand::new("AT+BLEGATTSNTFY")
        .args()?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .arg_int(len)?;
    // The reference ends with a trailing comma so the payload can follow the
    // same transfer.
    cmd.suffix(",")?.build()
}

/// Indicate a characteristic (`AT+BLEGATTSIND=<svc>,<char>,<len>` then data).
pub fn gatts_indicate(service_index: u16, char_index: u16, len: u32) -> Result<AtCommandString> {
    let cmd = AtCommand::new("AT+BLEGATTSIND")
        .args()?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .arg_int(len)?;
    cmd.suffix(",")?.build()
}

/// Provide data for a client read (`AT+BLEGATTSRD=<svc>,<char>,<len>` then data).
pub fn gatts_set_read_data(service_index: u16, char_index: u16, len: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTSRD")
        .args()?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .arg_int(len)?
        .build()
}

/// Discover a peer's services (`AT+BLEGATTCSRVDIS=<conn>`).
pub fn gattc_discover_services(conn_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTCSRVDIS")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Discover a service's characteristics (`AT+BLEGATTCCHARDIS=<conn>,<svc>`).
pub fn gattc_discover_chars(conn_handle: u16, service_index: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTCCHARDIS")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(service_index)?
        .build()
}

/// Write to a peer characteristic
/// (`AT+BLEGATTCWR=<conn>,<svc>,<char>,<len>` then data).
pub fn gattc_write(
    conn_handle: u16,
    service_index: u16,
    char_index: u16,
    len: u32,
) -> Result<AtCommandString> {
    let cmd = AtCommand::new("AT+BLEGATTCWR")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .arg_int(len)?;
    cmd.suffix(",")?.build()
}

/// Read a peer characteristic (`AT+BLEGATTCRD=<conn>,<svc>,<char>`).
pub fn gattc_read(conn_handle: u16, service_index: u16, char_index: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTCRD")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(service_index)?
        .arg_int(char_index)?
        .build()
}

/// Subscribe to a peer characteristic
/// (`AT+BLEGATTCSUBSCRIBE=<conn>,<desc>,<value>,<prop>`).
pub fn gattc_subscribe(
    conn_handle: u16,
    char_desc_handle: u16,
    char_value_handle: u16,
    char_prop: u16,
) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTCSUBSCRIBE")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(char_desc_handle)?
        .arg_int(char_value_handle)?
        .arg_int(char_prop)?
        .build()
}

/// Unsubscribe from a peer characteristic
/// (`AT+BLEGATTCUNSUBSCRIBE=<conn>,<char>`).
pub fn gattc_unsubscribe(conn_handle: u16, char_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLEGATTCUNSUBSCRIBE")
        .args()?
        .arg_int(conn_handle)?
        .arg_int(char_handle)?
        .build()
}

/// Set security parameters (`AT+BLESECPARAM=<param>`).
pub fn set_security_param(param: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECPARAM")
        .args()?
        .arg_int(param)?
        .build()
}

/// Query security parameters (`AT+BLESECPARAM?`).
pub fn get_security_param() -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECPARAM?").build()
}

/// Start security on a connection (`AT+BLESECSTART=<conn>`).
pub fn security_start(conn_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECSTART")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Enter the peer passkey (`AT+BLESECPASSKEY=<conn>,<000000>`).
pub fn security_passkey(conn_handle: u16, passkey: u32) -> Result<AtCommandString> {
    let mut hex = String::<8>::new();
    write!(hex, "{:06}", passkey).map_err(|_| Error::BufferTooSmall)?;
    AtCommand::new("AT+BLESECPASSKEY")
        .args()?
        .arg_int(conn_handle)?
        .arg_str(&hex)?
        .build()
}

/// Confirm a passkey (`AT+BLESECPASSKEYCONFIRM=<conn>`).
pub fn security_passkey_confirm(conn_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECPASSKEYCONFIRM")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Confirm pairing (`AT+BLESECPAIRINGCONFIRM=<conn>`).
pub fn security_pairing_confirm(conn_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECPAIRINGCONFIRM")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Select the scan channel (`AT+BLESECCANNEL=<conn>`).
pub fn security_scan_channel(conn_handle: u16) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECCANNEL")
        .args()?
        .arg_int(conn_handle)?
        .build()
}

/// Remove a bond (`AT+BLESECUNPAIR="<mac>",<reason>`).
pub fn security_unpair(mac: &[u8; 6], reason: u32) -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECUNPAIR")
        .args()?
        .arg_quoted(&mac_string(mac)?)?
        .arg_int(reason)?
        .build()
}

/// List bonded devices (`AT+BLESECGETLTKLIST?`).
pub fn security_get_bonded_devices() -> Result<AtCommandString> {
    AtCommand::new("AT+BLESECGETLTKLIST?").build()
}

/// Set the GAP appearance (`AT+BLESETGAPAPPEARANCE=<00000>`).
pub fn set_gap_appearance(appearance: u16) -> Result<AtCommandString> {
    let mut value = String::<8>::new();
    write!(value, "{:05}", appearance).map_err(|_| Error::BufferTooSmall)?;
    AtCommand::new("AT+BLESETGAPAPPEARANCE")
        .args()?
        .arg_str(&value)?
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cmd: Result<AtCommandString>) -> String<128> {
        let mut out = String::new();
        out.push_str(cmd.unwrap().as_str()).unwrap();
        out
    }

    #[test]
    fn init_and_name() {
        assert_eq!(s(init(BleMode::Dual)), "AT+BLEINIT=3\r\n");
        assert_eq!(s(deinit()), "AT+BLEINIT=0\r\n");
        assert_eq!(s(set_name("sensor")), "AT+BLENAME=\"sensor\"\r\n");
    }

    #[test]
    fn bd_address_formatting() {
        assert_eq!(
            s(set_bd_address(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF])),
            "AT+BLEADDR=\"aa:bb:cc:dd:ee:ff\"\r\n"
        );
    }

    #[test]
    fn advertising() {
        assert_eq!(
            s(set_adv_param(32, 48, 0, 7)),
            "AT+BLEADVPARAM=32,48,0,7\r\n"
        );
        assert_eq!(s(adv_start()), "AT+BLEADVSTART\r\n");
        assert_eq!(s(adv_stop()), "AT+BLEADVSTOP\r\n");
    }

    #[test]
    fn scanning() {
        assert_eq!(s(scan(true)), "AT+BLESCAN=1\r\n");
        assert_eq!(s(scan(false)), "AT+BLESCAN=0\r\n");
    }

    #[test]
    fn gatt_server() {
        assert_eq!(
            s(gatts_create_service(0, "A002", uuid_type::UUID16)),
            "AT+BLEGATTSSRVCRE=0,\"A002\",1,0\r\n"
        );
        assert_eq!(
            s(gatts_create_char(
                0,
                0,
                "C301",
                char_prop::NOTIFY | char_prop::READ,
                char_perm::READ,
                uuid_type::UUID16
            )),
            "AT+BLEGATTSCHARCRE=0,0,\"C301\",18,1,0\r\n"
        );
        assert_eq!(s(gatts_register()), "AT+BLEGATTSREGISTER=1\r\n");
        assert_eq!(s(gatts_notify(0, 0, 4)), "AT+BLEGATTSNTFY=0,0,4,\r\n");
    }

    #[test]
    fn gatt_client_and_security() {
        assert_eq!(s(gattc_discover_services(0)), "AT+BLEGATTCSRVDIS=0\r\n");
        assert_eq!(
            s(gattc_subscribe(0, 1, 2, char_prop::NOTIFY)),
            "AT+BLEGATTCSUBSCRIBE=0,1,2,16\r\n"
        );
        assert_eq!(s(security_passkey(0, 123456)), "AT+BLESECPASSKEY=0,123456\r\n");
        assert_eq!(s(security_start(0)), "AT+BLESECSTART=0\r\n");
    }

    #[test]
    fn appearance_is_zero_padded() {
        assert_eq!(s(set_gap_appearance(64)), "AT+BLESETGAPAPPEARANCE=00064\r\n");
    }
}
