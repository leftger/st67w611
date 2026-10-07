//! Unified socket layer.
//!
//! The two firmwares put TCP/IP in different places, but application code
//! should not have to care. This module exposes one surface either way:
//!
//! * **T01** (`mission-t01`): the module owns TCP/IP and the host drives AT
//!   sockets — [`t01::TcpSocket`].
//! * **T02** (`mission-t02`): the host owns TCP/IP and the xarxa-based
//!   `embassy-net` stack runs on top — [`t02::TcpSocket`].
//!
//! Whichever is active, [`TcpSocket`] implements
//! [`embedded_io_async::Read`] and [`embedded_io_async::Write`], so generic
//! application code — and [`crate::net::tls`] — works unchanged.

#[cfg(feature = "mission-t01")]
pub mod t01;
#[cfg(feature = "mission-t01")]
pub use t01::{SocketError, TcpSocket, UdpSocket};

#[cfg(feature = "mission-t02")]
pub mod t02;
#[cfg(feature = "mission-t02")]
pub use t02::{Stack, TcpSocket, UdpSocket};
