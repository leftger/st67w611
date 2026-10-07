//! Network layer.
//!
//! Two halves, split along the firmware boundary:
//!
//! * the **transport-generic clients** ([`Net`]) — interface options, DNS,
//!   SNTP, ping, TCP servers, TLS options — available on either firmware;
//! * the **firmware's own stack glue**:
//!   * T01 (`mission-t01`): the module owns TCP/IP, so [`NetworkDevice`] and
//!     the AT socket API are the data path;
//!   * T02 (`mission-t02`): the host owns TCP/IP, so [`xarxa`] provides the
//!     raw-L2 driver for the xarxa-based `embassy-net` stack.

// Transport-generic high-level client (works on both firmwares)
pub mod client;

pub use client::Net;

// TLS over any embedded-io-async socket, using `embedded-tls`
#[cfg(feature = "tls")]
pub mod tls;

// T01: the module owns TCP/IP — AT command sockets
#[cfg(feature = "mission-t01")]
pub mod device;
#[cfg(feature = "mission-t01")]
pub mod driver;

#[cfg(feature = "mission-t01")]
pub use device::{ConnectionStatus, NetworkDevice, Socket};

// T02: the host owns TCP/IP — raw-L2 driver for the xarxa-based embassy-net
#[cfg(feature = "mission-t02")]
pub mod xarxa;

#[cfg(feature = "mission-t02")]
pub use xarxa::{
    AtOutput, AtStatus, Control, Runner as XarxaRunner, State as XarxaState, WifiDevice,
};
