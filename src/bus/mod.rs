//! Bus layer - SPI communication with the ST67W611 module

pub mod spi;
pub mod spi_rdy;

pub use spi::{SpiTransport, SpiTransportAuto};
pub use spi_rdy::SpiTransportRdy;
