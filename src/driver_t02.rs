//! T02 driver facade.
//!
//! On T02 the host owns the network stack: the module is a raw-L2 device
//! ([`crate::net::xarxa`]) and an xarxa-based `embassy-net` stack runs on top.
//! This facade mirrors the T01 [`crate::Driver`] so application code is written
//! once:
//!
//! ```ignore
//! let wifi = driver.wifi();   // identical on T01 and T02
//! ```
//!
//! The T02 transport ([`Control`]) is stateful and cannot be duplicated, so the
//! clients borrow it — each accessor returns e.g. `WiFi<&Control<'_>>`. That
//! works because `&T` is itself an
//! [`AtTransport`](crate::at::AtTransport).

use crate::ble::Ble;
use crate::fwu::Fwu;
use crate::mqtt::Mqtt;
use crate::net::client::Net;
use crate::net::xarxa::Control;
use crate::wifi::WiFi;

/// T02 driver facade.
pub struct Driver<'d> {
    control: Control<'d>,
}

impl<'d> Driver<'d> {
    /// Wrap a [`Control`] handle obtained from [`crate::net::xarxa::new`].
    pub fn new(control: Control<'d>) -> Self {
        Self { control }
    }

    /// The underlying link handle.
    pub fn control(&self) -> &Control<'d> {
        &self.control
    }

    /// Take the link handle back out.
    pub fn into_control(self) -> Control<'d> {
        self.control
    }

    /// Wi-Fi client: mode, scan, credential store, TWT, Soft-AP, …
    pub fn wifi(&self) -> WiFi<&Control<'d>> {
        WiFi::new(&self.control)
    }

    /// Network configuration/services: IPv6, DNS, SNTP, ping, TCP servers, …
    pub fn net(&self) -> Net<&Control<'d>> {
        Net::new(&self.control)
    }

    /// BLE client.
    pub fn ble(&self) -> Ble<&Control<'d>> {
        Ble::new(&self.control)
    }

    /// Firmware-update client.
    pub fn fwu(&self) -> Fwu<&Control<'d>> {
        Fwu::new(&self.control)
    }

    /// MQTT client.
    pub fn mqtt(&self) -> Mqtt<&Control<'d>> {
        Mqtt::new(&self.control)
    }
}
