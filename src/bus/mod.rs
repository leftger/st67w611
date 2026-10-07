//! Bus layer - SPI communication with the ST67W611 module
//!
//! Two generations of transport live here:
//!
//! * [`frame`] and [`engine`] implement the reference X-CUBE-ST67W61/QCC74x SPI
//!   protocol: an 8-byte header, full-duplex exchange, `rx_stall` flow control
//!   and a two-part transfer for oversized frames. This is what the T02
//!   raw-L2 (embassy-net) path uses.
//! * [`spi`] and [`spi_rdy`] are the original, simpler AT-only transports kept
//!   for the existing T01 socket path.
//!
//! Note that [`spi::SpiTransport`] drives chip-select **active low**, while the
//! real module (and [`engine::Engine`], and [`spi_rdy::SpiTransportRdy`]) use
//! **active high**. New code should use [`engine::Engine`].

pub mod engine;
pub mod frame;
pub mod spi;
pub mod spi_rdy;

pub use frame::{Header, HeaderError, TrafficType, HEADER_LEN, MAGIC, MAX_PAYLOAD};
pub use spi::{SpiTransport, SpiTransportAuto};
pub use spi_rdy::SpiTransportRdy;

pub use engine::{Engine, Outbound, Received};
