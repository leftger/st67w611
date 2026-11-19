#![no_std]
#![no_main]

//! GY-MCU90640 thermal camera probe over USART1.
//!
//! The GY-MCU90640 module has its own MCU and streams 1544-byte frames over a
//! TTL UART (default 115200 8N1, continuous output). This probe receives those
//! frames, validates the checksum, and prints the temperature statistics.
//!
//! ## Wiring (NUCLEO-WBA65RI morpho headers, per UM3448)
//!
//! | GY-MCU90640 | Nucleo       | MCU pin | Note                        |
//! |-------------|--------------|---------|-----------------------------|
//! | VCC         | CN3-16 (3V3) | —       | power at 3.3V keeps TTL 3.3V|
//! | GND         | CN3-19/20    | —       |                             |
//! | TX (data)   | CN3-35       | PA8     | USART1_RX                  |
//! | RX (cmds)   | CN3-37       | PB12    | USART1_TX (optional)        |
//!
//! **Important:** by default USART1 is bridged to the ST-LINK VCP (UM3448
//! §7.5.4: SB5 on PA8/RX, SB3 on PB12/TX). The ST-LINK's idle UART TX will
//! fight the camera's TX on PA8, so **remove SB5** (and SB3 if you also send
//! commands) before connecting the camera. Logging here is RTT-only, so the
//! VCP is not needed.
//!
//! Frame format (from the GY-MCU9064 user manual):
//!
//! ```text
//!   [0..2)   0x5A 0x5A          frame header
//!   [2..4)   u16 LE data amount = 1538
//!   [4..1540) 768 x i16 LE      pixel temps x 100 degC, row-major
//!   [1540..1542) i16 LE         MLX90640 own temp (Ta/ambient) x 100
//!   [1542..1544) u16 LE         checksum: 16-bit sum of the first 771 words
//! ```
//!
//! Run: `cargo run --release --bin thermal_probe`

use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_stm32::usart::{self, BufferedUart, Config as UartConfig};
use embassy_stm32::{bind_interrupts, peripherals};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// USART1 interrupt for the buffered (non-DMA) UART.
bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
});

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

/// Total bytes in one GY-MCU90640 frame.
const FRAME_LEN: usize = 1544;
/// Number of thermal pixels (32 columns x 24 rows).
const PIXELS: usize = 768;
/// Byte index of the ambient-temperature word within the frame.
const TA_OFFSET: usize = 4 + PIXELS * 2;

/// Streaming frame parser: resyncs on `5A 5A` and assembles complete frames.
struct Parser {
    buf: [u8; FRAME_LEN],
    pos: usize,
}

impl Parser {
    const fn new() -> Self {
        Self {
            buf: [0u8; FRAME_LEN],
            pos: 0,
        }
    }

    /// Feed one byte. Returns `true` every time a full frame is assembled.
    fn push(&mut self, b: u8) -> bool {
        match self.pos {
            0 => {
                // Look for the first 0x5A.
                if b == 0x5A {
                    self.buf[0] = b;
                    self.pos = 1;
                }
                false
            }
            1 => {
                if b == 0x5A {
                    self.buf[1] = b;
                    self.pos = 2;
                } else {
                    // Second header byte wrong: restart the search.
                    self.pos = 0;
                }
                false
            }
            2..=3 => {
                self.buf[self.pos] = b;
                self.pos += 1;
                false
            }
            4 => {
                self.buf[self.pos] = b;
                self.pos += 1;
                // Byte 4 is the first data byte; the length in bytes 2..3 must
                // match 768 pixels + the ambient word, or we have mis-synced.
                let amount = u16::from_le_bytes([self.buf[2], self.buf[3]]) as usize;
                if amount != PIXELS * 2 + 2 {
                    warn!("bad frame length {} (expected {})", amount, PIXELS * 2 + 2);
                    self.pos = 0;
                }
                false
            }
            _ => {
                self.buf[self.pos] = b;
                self.pos += 1;
                if self.pos == FRAME_LEN {
                    self.pos = 0;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// 16-bit sum of the first 771 words, per the manual.
    fn checksum_ok(&self) -> bool {
        let mut sum: u16 = 0;
        for i in 0..(FRAME_LEN - 2) / 2 {
            sum = sum.wrapping_add(u16::from_le_bytes([self.buf[2 * i], self.buf[2 * i + 1]]));
        }
        let expected = u16::from_le_bytes([self.buf[FRAME_LEN - 2], self.buf[FRAME_LEN - 1]]);
        sum == expected
    }

    /// Min, max, center (4 center pixels averaged) and ambient temperatures,
    /// all in hundredths of a degree C.
    fn stats(&self) -> (i16, i16, i16, i16) {
        let mut min = i16::MAX;
        let mut max = i16::MIN;
        for i in 0..PIXELS {
            let v = i16::from_le_bytes([self.buf[4 + 2 * i], self.buf[5 + 2 * i]]);
            min = min.min(v);
            max = max.max(v);
        }
        // Center of the 32x24 grid: pixels (15,11), (16,11), (15,12), (16,12).
        let center = [11 * 32 + 15, 11 * 32 + 16, 12 * 32 + 15, 12 * 32 + 16]
            .into_iter()
            .map(|i| i16::from_le_bytes([self.buf[4 + 2 * i], self.buf[5 + 2 * i]]))
            .sum::<i16>()
            / 4;
        let ta = i16::from_le_bytes([self.buf[TA_OFFSET], self.buf[TA_OFFSET + 1]]);
        (min, max, center, ta)
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("thermal_probe: GY-MCU90640 on USART1 (PB12 TX / PA8 RX)");

    // Buffered (interrupt-driven) UART, no DMA. The camera streams, so the TX
    // buffer can stay tiny; the RX buffer only needs to smooth out scheduling.
    static TX_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 512]> = StaticCell::new();

    // Default is 115200 8N1, matching the module's power-on default. To run the
    // module at 460800 (for the full 8 Hz rate), send `A5 15 03 BD` then
    // `A5 65 01 0B` (save) once - see the manual, section IV.
    let config = UartConfig::default();
    let uart = unwrap!(BufferedUart::new(
        p.USART1,
        p.PB12,
        p.PA8,
        Irqs,
        TX_BUF.init([0u8; 64]),
        RX_BUF.init([0u8; 512]),
        config,
    ));
    let (_tx, mut rx) = uart.split();

    info!("waiting for frames (expect one every ~134 ms at 115200)");

    let mut parser = Parser::new();
    let mut chunk = [0u8; 256];
    let mut frames: u32 = 0;
    let mut bad_checksums: u32 = 0;

    loop {
        match rx.read(&mut chunk).await {
            Ok(0) => {}
            Ok(n) => {
                for &b in &chunk[..n] {
                    if parser.push(b) {
                        frames += 1;
                        if parser.checksum_ok() {
                            let (min, max, center, ta) = parser.stats();
                            info!(
                                "frame {}: min={}.{:02} max={}.{:02} center={}.{:02} ta={}.{:02} C (checksum ok)",
                                frames,
                                min / 100,
                                min.abs() % 100,
                                max / 100,
                                max.abs() % 100,
                                center / 100,
                                center.abs() % 100,
                                ta / 100,
                                ta.abs() % 100,
                            );
                        } else {
                            bad_checksums += 1;
                            warn!(
                                "frame {}: BAD CHECKSUM ({} bad so far)",
                                frames, bad_checksums
                            );
                        }
                    }
                }
            }
            Err(e) => error!("uart error: {:?}", e),
        }
    }
}
