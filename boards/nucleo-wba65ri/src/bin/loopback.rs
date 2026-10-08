#![no_std]
#![no_main]

//! SPI loopback self-test for the ST67W611 wiring on the NUCLEO-WBA65RI.
//!
//! **Unplug the ST67 module first**, then put a single jumper between Arduino
//! **D11 (MOSI / PC3)** and **D12 (MISO / PA9)**. This sends a known pattern out
//! of MOSI, clocks it back in on MISO, and prints both, so we can tell whether
//! the host's transmit path and the jumper are actually good.
//!
//! Why this test: reading the module's `ready` banner proves SCK, CS, MISO, RDY
//! and CHIP_EN all work — but it says *nothing* about MOSI, because the banner is
//! unsolicited. MOSI is the only signal we have no independent evidence for, and
//! a dead MOSI would explain the whole picture exactly: a module that boots,
//! announces itself over MISO, never hears our commands, and answers every
//! transfer with an empty frame.
//!
//! Expected result with the jumper fitted: every received byte equals the sent
//! byte. If instead we read the same empty/garbage patterns the module produces,
//! the fault is on the host side or in the jumper — not the module.

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("SPI loopback test (jumper D11/MOSI -> D12/MISO, module unplugged)");

    // Same SPI2 pins as the real example: SCK=PB10, MOSI=PC3, MISO=PA9.
    let mut spi = Spi::new_blocking(p.SPI2, p.PB10, p.PC3, p.PA9, SpiConfig::default());

    // A pattern that is easy to read by eye in the log.
    let tx = [0xA5u8, 0x5A, 0x00, 0xFF, 0x55, 0xAA, b'A', b'T'];
    let mut rx = [0u8; 8];

    loop {
        match embedded_hal::spi::SpiBus::transfer(&mut spi, &mut rx, &tx) {
            Ok(()) => {
                info!("sent {=[u8]}  got {=[u8]}", &tx[..], &rx[..]);
                if rx == tx {
                    info!("  MATCH - host MOSI, MISO and the jumper are all good");
                } else {
                    info!("  MISMATCH - the transmit path is not coming back");
                }
            }
            Err(_) => info!("transfer failed"),
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}
