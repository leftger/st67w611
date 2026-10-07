//! T02 sockets: the host owns TCP/IP.
//!
//! The module hands the host raw L2 frames (see [`crate::net::xarxa`]) and the
//! xarxa-based `embassy-net` stack runs on top. Sockets are therefore
//! `embassy-net` sockets, re-exported here so that application code can name
//! them through [`crate::stack`] regardless of firmware.
//!
//! Note that this `embassy-net` is the xarxa-based one, whose construction API
//! differs from the crates.io release: a [`Stack`] is created from a
//! `StackStorage` and a random seed (see `embassy_net::Stack::new`), and the
//! interface [`config`] lives in a submodule.

pub use embassy_net;
pub use embassy_net::config;
pub use embassy_net::tcp::TcpSocket;
pub use embassy_net::udp::UdpSocket;
pub use embassy_net::Stack;
