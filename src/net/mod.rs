//! Network layer
//!
//! This module provides networking functionality for the ST67W611 module.
//!
//! # Firmware Architectures
//!
//! The ST67W611 supports two firmware architectures:
//!
//! ## T01 Firmware (default, `mission-t01` feature)
//!
//! TCP/IP stack runs on the module. Use AT command-based sockets:
//! - [`NetworkDevice`] - Socket management via AT commands
//! - [`Socket`] - Individual socket handle
//!
//! ## T02 Firmware (`mission-t02` feature)
//!
//! TCP/IP stack runs on the host MCU using embassy-net:
//! - [`embassy_driver`] - embassy-net driver implementation
//! - Raw Ethernet frames are exchanged with the module
//!
//! # Feature Selection
//!
//! Enable exactly one of:
//! - `mission-t01` (default) - AT command sockets
//! - `mission-t02` - embassy-net integration

// T01 firmware: AT command-based sockets
#[cfg(feature = "mission-t01")]
pub mod device;
#[cfg(feature = "mission-t01")]
pub mod driver;

#[cfg(feature = "mission-t01")]
pub use device::{ConnectionStatus, NetworkDevice, Socket};

// T02 firmware: embassy-net integration
#[cfg(feature = "mission-t02")]
pub mod embassy_driver;

#[cfg(feature = "mission-t02")]
pub use embassy_driver::{
    new_driver, Capabilities, Medium, PacketBuf, RxToken, SpiFrameHeader, St67w611Device,
    St67w611Runner, St67w611Transport, State, TrafficType, TxToken, MTU,
};
