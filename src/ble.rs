//! Typed BLE driver.
//!
//! This wraps the [`crate::at::ble`] command builders in an ergonomic API and
//! is generic over [`AtTransport`], so the same code runs on T01
//! (`AtProcessor`) and T02 ([`crate::net::xarxa::Control`]).
//!
//! ```no_run
//! use st67w611::ble::Ble;
//! use st67w611::at::BleMode;
//!
//! async fn advertise<C: st67w611::at::AtTransport>(control: C) -> Result<(), st67w611::Error> {
//!     let ble = Ble::new(control);
//!     ble.init(BleMode::Server).await?;
//!     ble.set_name("sensor").await?;
//!     ble.set_adv_data("0201060A0948656C6C6F").await?;
//!     ble.adv_start().await?;
//!     Ok(())
//! }
//! ```
//!
//! Operations whose payload follows the AT line (GATT notify/indicate/write)
//! need a transport that implements
//! [`AtTransport::at_with_payload`](crate::at::AtTransport::at_with_payload);
//! otherwise they return [`Error::NotSupported`](crate::Error::NotSupported).

use crate::at::ble as cmd;
use crate::at::command::{wire_str, AtCommandString};
use crate::at::reply::{AtOutput, AtStatus};
use crate::at::transport::AtTransport;
use crate::at::{BleMode, LineBuffer};
use crate::error::{Error, Result};

pub use crate::at::ble::{char_perm, char_prop, uuid_type};

/// Typed BLE client.
///
/// `T` is any [`AtTransport`]; see the module docs.
pub struct Ble<T> {
    transport: T,
}

impl<T> Ble<T> {
    /// Create a BLE client over `transport`.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// The underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutable access to the underlying transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Consume the client, returning the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: AtTransport> Ble<T> {
    async fn exec(&self, command: Result<AtCommandString>) -> Result<AtOutput> {
        let command = command?;
        self.transport.at(wire_str(&command)).await
    }

    async fn exec_ok(&self, command: Result<AtCommandString>) -> Result<()> {
        let out = self.exec(command).await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    async fn exec_payload(&self, command: Result<AtCommandString>, payload: &[u8]) -> Result<()> {
        let command = command?;
        let out = self.transport.at_with_payload(wire_str(&command), payload).await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Run a query and return the value after `prefix` (e.g. `"+BLENAME:"`).
    async fn query(&self, command: Result<AtCommandString>, prefix: &str) -> Result<LineBuffer> {
        let out = self.exec(command).await?;
        let value = out.value_after(prefix).ok_or(Error::InvalidResponse)?;
        let mut line = LineBuffer::new();
        line.push_str(value).map_err(|_| Error::BufferTooSmall)?;
        Ok(line)
    }

    /// Initialise BLE in the given role.
    pub async fn init(&self, mode: BleMode) -> Result<()> {
        self.exec_ok(cmd::init(mode)).await
    }

    /// De-initialise BLE.
    pub async fn deinit(&self) -> Result<()> {
        self.exec_ok(cmd::deinit()).await
    }

    /// Set the device name.
    pub async fn set_name(&self, name: &str) -> Result<()> {
        self.exec_ok(cmd::set_name(name)).await
    }

    /// Read the device name.
    pub async fn name(&self) -> Result<LineBuffer> {
        self.query(cmd::get_name(), "+BLENAME:").await
    }

    /// Set the BD address.
    pub async fn set_bd_address(&self, mac: &[u8; 6]) -> Result<()> {
        self.exec_ok(cmd::set_bd_address(mac)).await
    }

    /// Read the BD address.
    pub async fn bd_address(&self) -> Result<LineBuffer> {
        self.query(cmd::get_bd_address(), "+BLEADDR:").await
    }

    /// Set the TX power.
    pub async fn set_tx_power(&self, power: u32) -> Result<()> {
        self.exec_ok(cmd::set_tx_power(power)).await
    }

    /// Set advertising parameters.
    pub async fn set_adv_param(
        &self,
        interval_min: u32,
        interval_max: u32,
        adv_type: u16,
        channel: u16,
    ) -> Result<()> {
        self.exec_ok(cmd::set_adv_param(interval_min, interval_max, adv_type, channel))
            .await
    }

    /// Set advertising data (hex string).
    pub async fn set_adv_data(&self, data: &str) -> Result<()> {
        self.exec_ok(cmd::set_adv_data(data)).await
    }

    /// Set scan-response data (hex string).
    pub async fn set_scan_rsp_data(&self, data: &str) -> Result<()> {
        self.exec_ok(cmd::set_scan_rsp_data(data)).await
    }

    /// Start advertising.
    pub async fn adv_start(&self) -> Result<()> {
        self.exec_ok(cmd::adv_start()).await
    }

    /// Stop advertising.
    pub async fn adv_stop(&self) -> Result<()> {
        self.exec_ok(cmd::adv_stop()).await
    }

    /// Set scanning parameters.
    pub async fn set_scan_param(
        &self,
        scan_type: u16,
        own_addr_type: u16,
        filter_policy: u16,
        scan_interval: u32,
        scan_window: u32,
    ) -> Result<()> {
        self.exec_ok(cmd::set_scan_param(
            scan_type,
            own_addr_type,
            filter_policy,
            scan_interval,
            scan_window,
        ))
        .await
    }

    /// Start or stop scanning.
    pub async fn scan(&self, enable: bool) -> Result<()> {
        self.exec_ok(cmd::scan(enable)).await
    }

    /// Connect to a peer.
    pub async fn connect(&self, conn_handle: u32, mac: &[u8; 6]) -> Result<()> {
        self.exec_ok(cmd::connect(conn_handle, mac)).await
    }

    /// Disconnect a peer.
    pub async fn disconnect(&self, conn_handle: u32) -> Result<()> {
        self.exec_ok(cmd::disconnect(conn_handle)).await
    }

    /// Query the current connections.
    pub async fn connections(&self) -> Result<AtOutput> {
        self.exec(cmd::get_connection()).await
    }

    /// Exchange MTU on a connection.
    pub async fn exchange_mtu(&self, conn_handle: u32) -> Result<()> {
        self.exec_ok(cmd::exchange_mtu(conn_handle)).await
    }

    /// Set the connection data length.
    pub async fn set_data_length(&self, conn_handle: u32, tx_bytes: u32, tx_time: u32) -> Result<()> {
        self.exec_ok(cmd::set_data_length(conn_handle, tx_bytes, tx_time))
            .await
    }

    // ---- GATT server -----------------------------------------------------

    /// Create a GATT service.
    pub async fn gatts_create_service(&self, index: u16, uuid: &str, uuid_type: u16) -> Result<()> {
        self.exec_ok(cmd::gatts_create_service(index, uuid, uuid_type))
            .await
    }

    /// Delete a GATT service.
    pub async fn gatts_delete_service(&self, index: u16) -> Result<()> {
        self.exec_ok(cmd::gatts_delete_service(index)).await
    }

    /// Create a GATT characteristic.
    pub async fn gatts_create_char(
        &self,
        service_index: u16,
        char_index: u16,
        uuid: &str,
        property: u16,
        permission: u16,
        uuid_type: u16,
    ) -> Result<()> {
        self.exec_ok(cmd::gatts_create_char(
            service_index,
            char_index,
            uuid,
            property,
            permission,
            uuid_type,
        ))
        .await
    }

    /// Register the created characteristics and start the server.
    pub async fn gatts_register(&self) -> Result<()> {
        self.exec_ok(cmd::gatts_register()).await
    }

    /// List the local services and characteristics.
    pub async fn gatts_services(&self) -> Result<AtOutput> {
        self.exec(cmd::gatts_get_services()).await
    }

    /// Notify a characteristic (payload follows the command).
    pub async fn gatts_notify(&self, service_index: u16, char_index: u16, data: &[u8]) -> Result<()> {
        self.exec_payload(
            cmd::gatts_notify(service_index, char_index, data.len() as u32),
            data,
        )
        .await
    }

    /// Indicate a characteristic (payload follows the command).
    pub async fn gatts_indicate(&self, service_index: u16, char_index: u16, data: &[u8]) -> Result<()> {
        self.exec_payload(
            cmd::gatts_indicate(service_index, char_index, data.len() as u32),
            data,
        )
        .await
    }

    /// Provide the value returned when a client reads a characteristic.
    pub async fn gatts_set_read_data(
        &self,
        service_index: u16,
        char_index: u16,
        data: &[u8],
    ) -> Result<()> {
        self.exec_payload(
            cmd::gatts_set_read_data(service_index, char_index, data.len() as u32),
            data,
        )
        .await
    }

    // ---- GATT client -----------------------------------------------------

    /// Discover a peer's services.
    pub async fn gattc_discover_services(&self, conn_handle: u16) -> Result<AtOutput> {
        self.exec(cmd::gattc_discover_services(conn_handle)).await
    }

    /// Discover a service's characteristics.
    pub async fn gattc_discover_chars(&self, conn_handle: u16, service_index: u16) -> Result<AtOutput> {
        self.exec(cmd::gattc_discover_chars(conn_handle, service_index))
            .await
    }

    /// Read a peer characteristic.
    pub async fn gattc_read(
        &self,
        conn_handle: u16,
        service_index: u16,
        char_index: u16,
    ) -> Result<AtOutput> {
        self.exec(cmd::gattc_read(conn_handle, service_index, char_index))
            .await
    }

    /// Write a peer characteristic (payload follows the command).
    pub async fn gattc_write(
        &self,
        conn_handle: u16,
        service_index: u16,
        char_index: u16,
        data: &[u8],
    ) -> Result<()> {
        self.exec_payload(
            cmd::gattc_write(conn_handle, service_index, char_index, data.len() as u32),
            data,
        )
        .await
    }

    /// Subscribe to a peer characteristic.
    pub async fn gattc_subscribe(
        &self,
        conn_handle: u16,
        char_desc_handle: u16,
        char_value_handle: u16,
        char_prop: u16,
    ) -> Result<()> {
        self.exec_ok(cmd::gattc_subscribe(
            conn_handle,
            char_desc_handle,
            char_value_handle,
            char_prop,
        ))
        .await
    }

    /// Unsubscribe from a peer characteristic.
    pub async fn gattc_unsubscribe(&self, conn_handle: u16, char_handle: u16) -> Result<()> {
        self.exec_ok(cmd::gattc_unsubscribe(conn_handle, char_handle))
            .await
    }

    // ---- Security --------------------------------------------------------

    /// Set security parameters.
    pub async fn set_security_param(&self, param: u16) -> Result<()> {
        self.exec_ok(cmd::set_security_param(param)).await
    }

    /// Start security on a connection.
    pub async fn security_start(&self, conn_handle: u16) -> Result<()> {
        self.exec_ok(cmd::security_start(conn_handle)).await
    }

    /// Enter the peer passkey.
    pub async fn security_passkey(&self, conn_handle: u16, passkey: u32) -> Result<()> {
        self.exec_ok(cmd::security_passkey(conn_handle, passkey)).await
    }

    /// Confirm a passkey.
    pub async fn security_passkey_confirm(&self, conn_handle: u16) -> Result<()> {
        self.exec_ok(cmd::security_passkey_confirm(conn_handle)).await
    }

    /// Confirm pairing.
    pub async fn security_pairing_confirm(&self, conn_handle: u16) -> Result<()> {
        self.exec_ok(cmd::security_pairing_confirm(conn_handle)).await
    }

    /// Remove a bond.
    pub async fn security_unpair(&self, mac: &[u8; 6], reason: u32) -> Result<()> {
        self.exec_ok(cmd::security_unpair(mac, reason)).await
    }

    /// List bonded devices.
    pub async fn security_bonded_devices(&self) -> Result<AtOutput> {
        self.exec(cmd::security_get_bonded_devices()).await
    }

    /// Set the GAP appearance.
    pub async fn set_gap_appearance(&self, appearance: u16) -> Result<()> {
        self.exec_ok(cmd::set_gap_appearance(appearance)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::{Cell, RefCell};
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    /// Minimal host executor for the tests (no timers involved).
    fn block_on<F: Future>(fut: F) -> F::Output {
        let mut fut = pin!(fut);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn ok_output() -> AtOutput {
        AtOutput {
            status: AtStatus::Ok,
            lines: heapless::Vec::new(),
        }
    }

    fn output_with(line: &str) -> AtOutput {
        let mut lines = heapless::Vec::new();
        let mut b = LineBuffer::new();
        b.push_str(line).unwrap();
        lines.push(b).unwrap();
        AtOutput {
            status: AtStatus::Ok,
            lines,
        }
    }

    /// Records the commands it is asked to run.
    #[derive(Default)]
    struct Mock {
        reply: Option<AtOutput>,
        last: RefCell<heapless::String<128>>,
        last_payload: Cell<usize>,
        payload_supported: bool,
    }

    impl Mock {
        fn record(&self, command: &str) {
            let mut s = heapless::String::new();
            s.push_str(command).unwrap();
            *self.last.borrow_mut() = s;
        }
    }

    impl AtTransport for Mock {
        async fn at(&self, command: &str) -> Result<AtOutput> {
            self.record(command);
            Ok(self.reply.clone().unwrap_or_else(ok_output))
        }

        async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
            if !self.payload_supported {
                return Err(Error::NotSupported);
            }
            self.record(command);
            self.last_payload.set(payload.len());
            Ok(ok_output())
        }
    }

    #[test]
    fn init_sends_correct_command() {
        let ble = Ble::new(Mock::default());
        block_on(ble.init(BleMode::Server)).unwrap();
        assert_eq!(ble.transport().last.borrow().as_str(), "AT+BLEINIT=2");
    }

    #[test]
    fn name_parses_information_line() {
        let ble = Ble::new(Mock {
            reply: Some(output_with("+BLENAME:my sensor")),
            ..Default::default()
        });
        let name = block_on(ble.name()).unwrap();
        assert_eq!(name.as_str(), "my sensor");
        assert_eq!(ble.transport().last.borrow().as_str(), "AT+BLENAME?");
    }

    #[test]
    fn missing_information_line_is_an_error() {
        let ble = Ble::new(Mock::default());
        assert_eq!(block_on(ble.name()), Err(Error::InvalidResponse));
    }

    #[test]
    fn notify_requires_payload_support() {
        let ble = Ble::new(Mock::default());
        assert_eq!(block_on(ble.gatts_notify(0, 0, &[1, 2, 3])), Err(Error::NotSupported));
    }

    #[test]
    fn notify_streams_payload_and_command() {
        let ble = Ble::new(Mock {
            payload_supported: true,
            ..Default::default()
        });
        block_on(ble.gatts_notify(0, 1, &[0xAA, 0xBB])).unwrap();
        assert_eq!(ble.transport().last.borrow().as_str(), "AT+BLEGATTSNTFY=0,1,2,");
        assert_eq!(ble.transport().last_payload.get(), 2);
    }

    #[test]
    fn adv_and_scan_commands() {
        let ble = Ble::new(Mock::default());
        block_on(ble.adv_start()).unwrap();
        assert_eq!(ble.transport().last.borrow().as_str(), "AT+BLEADVSTART");
        block_on(ble.scan(true)).unwrap();
        assert_eq!(ble.transport().last.borrow().as_str(), "AT+BLESCAN=1");
    }

    #[test]
    fn error_status_becomes_at_command_failed() {
        let ble = Ble::new(Mock {
            reply: Some(AtOutput {
                status: AtStatus::Error,
                lines: heapless::Vec::new(),
            }),
            ..Default::default()
        });
        assert_eq!(block_on(ble.adv_start()), Err(Error::AtCommandFailed));
    }
}
