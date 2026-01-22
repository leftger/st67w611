//! Network layer - embassy-net integration

pub mod device;
pub mod driver;

pub use device::{NetworkDevice, Socket};
pub use driver::St67w611Driver;
