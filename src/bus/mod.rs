//! Bus layer - SPI communication with the ST67W611 module

pub mod spi;

pub use spi::{SpiTransport, SpiTransportAuto};
