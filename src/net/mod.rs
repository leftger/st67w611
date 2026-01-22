//! Network layer - Socket management
//!
//! This module provides socket-based networking using the module's built-in TCP/IP stack.
//! This is the recommended approach for ST67W611 (see `driver.rs` for why embassy-net
//! is not supported).

pub mod device;
pub mod driver;

pub use device::{ConnectionStatus, NetworkDevice, Socket};
