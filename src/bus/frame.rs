//! ST67W611 SPI frame format.
//!
//! This mirrors the wire format used by the reference X-CUBE-ST67W61 network
//! driver (`Driver/W61_bus/spi_iface.c`, derived from the QCC74x SPI host
//! example). Every SPI transaction starts with an 8-byte header, little-endian
//! on the wire:
//!
//! | offset | size | field                                            |
//! |--------|------|--------------------------------------------------|
//! | 0      | 2    | magic, always [`MAGIC`] (`0x55AA`)               |
//! | 2      | 2    | payload length (header excluded)                 |
//! | 4      | 1    | `version:2` \| `rx_stall:1` \| `flags:5`         |
//! | 5      | 1    | traffic type, see [`TrafficType`]                |
//! | 6      | 2    | reserved                                         |
//!
//! Payloads are padded to a 4-byte boundary with [`PAD_BYTE`] so that the
//! module can move them with 32-bit copies.
//!
//! ```
//! use st67w611::bus::frame::{Header, TrafficType, MAGIC};
//!
//! let h = Header::new(TrafficType::AtCommand, 5);
//! let bytes = h.to_bytes();
//! assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), MAGIC);
//! assert_eq!(Header::from_bytes(&bytes).payload_len(), 5);
//! ```

/// Header magic code that starts every SPI frame.
pub const MAGIC: u16 = 0x55AA;

/// Size of the frame header, in bytes.
pub const HEADER_LEN: usize = 8;

/// Payload size the SPI transport pads to (4 bytes), see `SPI_BUF_ALIGN_MASK`.
pub const ALIGN: usize = 4;

/// Byte used to pad the last partial 32-bit word of a payload.
pub const PAD_BYTE: u8 = 0x88;

/// Maximum payload length accepted in a header.
///
/// This is `W61_MAX_SPI_XFER` from the reference configuration
/// (`Conf/w61_driver_config_template.h`), *not* including the header.
pub const MAX_PAYLOAD: usize = 1520;

/// Number of defined traffic types (`SPI_MSG_CTRL_TRAFFIC_TYPE_MAX`).
pub const TRAFFIC_TYPE_COUNT: u8 = 5;

/// Resolve the total, 4-byte aligned on-wire length of a frame.
///
/// The header is always a multiple of [`ALIGN`], so only the payload needs
/// rounding up. Returns `HEADER_LEN + align4(payload_len)`.
pub const fn frame_len(payload_len: usize) -> usize {
    HEADER_LEN + aligned_len(payload_len)
}

/// Round `len` up to the next multiple of [`ALIGN`].
pub const fn aligned_len(len: usize) -> usize {
    (len + (ALIGN - 1)) & !(ALIGN - 1)
}

/// Kind of traffic carried by a frame (the header `type` byte).
///
/// The values match `SPI_MSG_CTRL_TRAFFIC_*` in `spi_iface.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum TrafficType {
    /// AT commands and their responses.
    AtCommand = 0,
    /// Raw Ethernet frames from the station interface (T02 firmware).
    NetworkSta = 1,
    /// Raw Ethernet frames from the Soft-AP interface (T02 firmware).
    NetworkAp = 2,
    /// Bluetooth HCI data.
    Hci = 3,
    /// OpenThread / 802.15.4 data.
    OpenThread = 4,
}

impl TrafficType {
    /// The raw `type` byte written into the header.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a raw `type` byte.
    ///
    /// Returns `None` for values the reference driver does not define. Such
    /// frames are still valid on the wire; callers that only demultiplex known
    /// types can drop them.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::AtCommand),
            1 => Some(Self::NetworkSta),
            2 => Some(Self::NetworkAp),
            3 => Some(Self::Hci),
            4 => Some(Self::OpenThread),
            _ => None,
        }
    }
}

/// Reason a [`Header`] failed [`Header::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HeaderError {
    /// Magic did not match [`MAGIC`]; the peer is not (yet) speaking the protocol.
    BadMagic,
    /// Payload length exceeded [`MAX_PAYLOAD`].
    TooLong,
}

/// Decoded 8-byte SPI frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Header {
    /// Payload length in bytes, header excluded.
    len: u16,
    /// Protocol version, 2 bits.
    version: u8,
    /// The peer's RX path is stalled: do not attach a payload on the next
    /// transaction until the peer clears this.
    rx_stall: bool,
    /// Free-form flags, 5 bits.
    flags: u8,
    /// Raw traffic type byte.
    traffic_type: u8,
}

impl Header {
    /// Build a header for an outbound frame.
    ///
    /// `version`, `rx_stall` and `flags` are zeroed, as in `SPI_HEADER_INIT`.
    pub const fn new(traffic_type: TrafficType, payload_len: u16) -> Self {
        Self {
            len: payload_len,
            version: 0,
            rx_stall: false,
            flags: 0,
            traffic_type: traffic_type as u8,
        }
    }

    /// Payload length in bytes.
    pub const fn payload_len(&self) -> usize {
        self.len as usize
    }

    /// Raw traffic type byte.
    pub const fn traffic_type_raw(&self) -> u8 {
        self.traffic_type
    }

    /// Decoded traffic type, if known.
    pub const fn traffic_type(&self) -> Option<TrafficType> {
        TrafficType::from_u8(self.traffic_type)
    }

    /// Protocol version bits.
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// Whether the peer reported its RX path stalled.
    pub const fn rx_stall(&self) -> bool {
        self.rx_stall
    }

    /// The 5 free-form flag bits.
    pub const fn flags(&self) -> u8 {
        self.flags
    }

    /// Total on-wire length of the frame this header describes.
    pub const fn frame_len(&self) -> usize {
        frame_len(self.len as usize)
    }

    /// Serialize to the 8 bytes that go on the wire.
    pub const fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let magic = MAGIC.to_le_bytes();
        let len = self.len.to_le_bytes();
        // byte 4: version:2 | rx_stall:1 | flags:5
        let control = (self.version & 0b11) | ((self.rx_stall as u8) << 2) | ((self.flags & 0b1_1111) << 3);
        [
            magic[0], magic[1], len[0], len[1], control, self.traffic_type, 0, 0,
        ]
    }

    /// Parse an 8-byte header.
    ///
    /// This never fails: use [`validate`](Self::validate) to reject garbage.
    pub const fn from_bytes(bytes: &[u8; HEADER_LEN]) -> Self {
        let control = bytes[4];
        Self {
            len: u16::from_le_bytes([bytes[2], bytes[3]]),
            version: control & 0b11,
            rx_stall: (control & 0b100) != 0,
            flags: control >> 3,
            traffic_type: bytes[5],
        }
    }

    /// Check magic and length, as `spi_header_validate` does.
    pub const fn validate(&self, bytes: &[u8; HEADER_LEN]) -> Result<(), HeaderError> {
        if u16::from_le_bytes([bytes[0], bytes[1]]) != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        if self.len as usize > MAX_PAYLOAD {
            return Err(HeaderError::TooLong);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = Header::new(TrafficType::NetworkSta, 1514);
        let bytes = h.to_bytes();
        let back = Header::from_bytes(&bytes);
        assert_eq!(back, h);
        assert_eq!(back.payload_len(), 1514);
        assert_eq!(back.traffic_type(), Some(TrafficType::NetworkSta));
        assert_eq!(back.frame_len(), HEADER_LEN + 1516);
    }

    #[test]
    fn magic_is_little_endian() {
        let bytes = Header::new(TrafficType::AtCommand, 0).to_bytes();
        assert_eq!(&bytes[..2], &[0xAA, 0x55]);
    }

    #[test]
    fn control_bits_pack_and_unpack() {
        // version 2, rx_stall set, flags 0b10101
        let raw = 0b1010_1110u8; // flags=10101, rx_stall=1, version=10
        let h = Header::from_bytes(&[0xAA, 0x55, 0, 0, raw, 0, 0, 0]);
        assert_eq!(h.version(), 2);
        assert!(h.rx_stall());
        assert_eq!(h.flags(), 0b10101);
        assert_eq!(h.to_bytes()[4], raw);
    }

    #[test]
    fn validate_rejects_bad_magic_and_length() {
        let mut bytes = Header::new(TrafficType::AtCommand, 0).to_bytes();
        bytes[0] = 0;
        assert_eq!(
            Header::from_bytes(&bytes).validate(&bytes),
            Err(HeaderError::BadMagic)
        );

        let h = Header::new(TrafficType::AtCommand, (MAX_PAYLOAD + 1) as u16);
        let bytes = h.to_bytes();
        assert_eq!(h.validate(&bytes), Err(HeaderError::TooLong));
    }

    #[test]
    fn alignment() {
        assert_eq!(aligned_len(0), 0);
        assert_eq!(aligned_len(1), 4);
        assert_eq!(aligned_len(4), 4);
        assert_eq!(aligned_len(5), 8);
        assert_eq!(frame_len(0), HEADER_LEN);
        assert_eq!(frame_len(5), HEADER_LEN + 8);
        // The header alone is already aligned.
        assert_eq!(frame_len(1514), HEADER_LEN + 1516);
    }

    #[test]
    fn traffic_type_roundtrip() {
        for t in [
            TrafficType::AtCommand,
            TrafficType::NetworkSta,
            TrafficType::NetworkAp,
            TrafficType::Hci,
            TrafficType::OpenThread,
        ] {
            assert_eq!(TrafficType::from_u8(t.as_u8()), Some(t));
        }
        assert_eq!(TrafficType::from_u8(TRAFFIC_TYPE_COUNT), None);
    }
}
