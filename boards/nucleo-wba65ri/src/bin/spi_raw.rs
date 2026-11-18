#![no_std]
#![no_main]

//! Deliberately stupid SPI test: no driver, no engine, no AT layer, no stack.
//!
//! This is the "is the wire even alive?" experiment. It talks to the ST67W611
//! through the same four Arduino pins the real example uses, sends hand-built
//! frames, and prints the raw bytes that come back. Nothing is shared with
//! `src/bus/engine.rs`, so a failure here cannot be blamed on the driver.
//!
//! ## Phase 1: poll until the module answers
//!
//! A bare 8-byte header (`len = 0`) is clocked out and the module's own header is
//! read back. Measured on hardware, the module needs a moment after CHIP_EN
//! before it says anything (the first poll right after power-on returns zeros),
//! so this polls every 100 ms until the magic appears.
//!
//! ## Phase 2: send `AT`
//!
//! Once the banner has been consumed, a header with `len = 4` followed by
//! `AT\r\n` is sent and its reply is printed.
//!
//! ## The one thing that is easy to get wrong
//!
//! When the module's header announces a payload, those bytes must be clocked out
//! **inside the same CS assertion**. Walk away and they stay queued in the
//! module, and every later transaction comes back shifted by them - forever.
//!
//! The first version of this test got that wrong and it was instructive: it read
//! a header saying `len = 9`, dropped the nine payload bytes, and everything
//! afterwards was garbage (`dc cb ba ad` repeating, then a wall of `ff`). That
//! desync looked exactly like a dead bus and was entirely self-inflicted.
//!
//! Run: `cargo run --release --bin spi_raw`

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_time::{Duration, Timer};
use embedded_hal::spi::SpiBus;
use {defmt_rtt as _, panic_probe as _};

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

/// `MAGIC` in `src/bus/frame.rs`, little-endian on the wire.
const MAGIC: [u8; 2] = [0xAA, 0x55];
/// Traffic type 0 = AT command (`TrafficType::AtCommand`).
const TYPE_AT: u8 = 0x00;

/// Scratch size: a header plus a generous payload.
const RX_MAX: usize = 64;

/// How long to keep polling for the module's first answer.
const POLL_ATTEMPTS: u32 = 20;
/// How many `AT` commands to send once the link is up.
const AT_ROUNDS: u32 = 5;

/// `len = 0` header: "I am clocking, tell me if you have anything".
const POLL: [u8; 8] = [MAGIC[0], MAGIC[1], 0x00, 0x00, 0x00, TYPE_AT, 0x00, 0x00];

/// `len = 4` header plus `AT\r\n`. 8 + 4 = 12 bytes, already 4-byte aligned, so
/// there is no padding to argue about.
const AT_CMD: [u8; 12] = [
    MAGIC[0], MAGIC[1], 0x04, 0x00, 0x00, TYPE_AT, 0x00, 0x00, // header
    b'A', b'T', b'\r', b'\n', // payload
];

/// Print bytes as hex, and as ASCII where they are printable.
fn dump(tag: &str, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        let c = if b.is_ascii_graphic() || *b == b' ' {
            *b as char
        } else {
            '.'
        };
        info!("  {}[{}] = 0x{:02x} '{}'", tag, i, b, c);
    }
}

/// Run one exchange: assert CS, clock `tx` out, read the module's header, then
/// read whatever payload that header announced - all before releasing CS.
///
/// Returns how many bytes ended up in `rx`.
async fn exchange<B: SpiBus>(
    spi: &mut B,
    cs: &mut Output<'static>,
    tx: &[u8],
    rx: &mut [u8; RX_MAX],
) -> usize {
    rx[..].fill(0);
    let first = tx.len();

    // CS is active **high** in this protocol, and ST's port refuses to re-assert
    // it until 2 us after the previous deassert.
    Timer::after(Duration::from_micros(2)).await;
    let _ = cs.set_high();
    Timer::after(Duration::from_micros(1)).await;

    let ok = spi.transfer(&mut rx[..first], tx).is_ok();

    let mut total = first;
    if ok && rx[0] == MAGIC[0] && rx[1] == MAGIC[1] {
        let payload = u16::from_le_bytes([rx[2], rx[3]]) as usize;
        let frame = 8 + ((payload + 3) & !3);
        if frame > first {
            let extra = (frame - first).min(RX_MAX - first);
            if spi.read(&mut rx[first..first + extra]).is_ok() {
                total += extra;
            }
        }
    }

    Timer::after(Duration::from_micros(1)).await;
    let _ = cs.set_low();
    total
}

/// True if `rx` starts with a valid frame header.
fn has_header(rx: &[u8]) -> bool {
    rx.len() >= 8 && rx[0] == MAGIC[0] && rx[1] == MAGIC[1]
}

/// Describe a frame header and, if there is one, its payload.
fn report(rx: &[u8], total: usize) {
    if !has_header(rx) {
        info!(
            "  no header: first bytes {:02x} {:02x} (expected aa 55)",
            rx[0], rx[1]
        );
        return;
    }
    let len = u16::from_le_bytes([rx[2], rx[3]]) as usize;
    let rx_stall = (rx[4] & 0b0000_0010) != 0;
    info!(
        "  header ok: len={} flags=0x{:02x} rx_stall={} type={}",
        len, rx[4], rx_stall, rx[5]
    );
    let end = (8 + len).min(total);
    if len > 0 && end > 8 {
        dump("payload", &rx[8..end]);
    }
    if rx_stall {
        info!("  module reports its RX path stalled: it will not take our payload");
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("=== stupid SPI test: raw frames, no driver ===");

    // Module enable (D5 / PB14): reset pulse, then settle.
    let mut en = Output::new(p.PB14, Level::Low, Speed::Low);
    Timer::after_millis(100).await;
    en.set_high();
    Timer::after_millis(100).await;
    info!("module power-on done");

    // RDY (D3 / PB13) as a plain input; this test only samples the level.
    let rdy = Input::new(p.PB13, Pull::None);
    info!("RDY level at start = {}", rdy.is_high());

    // CS (D10 / PB9). Active high, so idle is low.
    let mut cs = Output::new(p.PB9, Level::Low, Speed::High);

    // SPI2: SCK=PB10, MOSI=PC3, MISO=PA9. `SpiConfig::default()` is 1 MHz,
    // mode 0, MSB first - what ST's own port uses. For mode 3 instead, set
    // `.mode` to `Mode { polarity: Polarity::IdleHigh,
    // phase: Phase::CaptureOnSecondTransition }`.
    let mut spi = Spi::new_blocking(p.SPI2, p.PB10, p.PC3, p.PA9, SpiConfig::default());
    info!("SPI2 up: 1 MHz, mode 0, MSB first");

    let mut rx = [0u8; RX_MAX];

    // ------------------------------------------------------------------
    // Phase 1: poll until the module answers with a real header.
    // ------------------------------------------------------------------
    info!("phase 1: polling for the module ...");
    let mut up = false;
    for attempt in 0..POLL_ATTEMPTS {
        let total = exchange(&mut spi, &mut cs, &POLL, &mut rx).await;
        info!("poll {} (RDY={})", attempt, rdy.is_high());
        dump("hdr", &rx[..8]);
        report(&rx, total);
        if has_header(&rx) {
            info!("module is up after {} poll(s)", attempt + 1);
            up = true;
            break;
        }
        Timer::after(Duration::from_millis(100)).await;
    }

    if !up {
        info!("module never answered: check power, CHIP_EN and the SPI bridges");
    } else {
        // ------------------------------------------------------------------
        // Phase 2: a real command. If this comes back with a payload, the host
        // transmit path is proven - the module parsed what we sent.
        // ------------------------------------------------------------------
        Timer::after(Duration::from_millis(100)).await;
        for round in 0..AT_ROUNDS {
            let total = exchange(&mut spi, &mut cs, &AT_CMD, &mut rx).await;
            info!("AT round {} (RDY={})", round, rdy.is_high());
            dump("hdr", &rx[..8]);
            report(&rx, total);
            Timer::after(Duration::from_millis(200)).await;
        }
    }

    info!("done.");
    loop {
        Timer::after_secs(30).await;
        info!("idle");
    }
}
