#![no_std]
#![no_main]

//! st67w611 bring-up on the **STM32N6570-DK** — Milestone A: detection.
//!
//! Wiring (Arduino header, verified against schematic MB1939 rev C02):
//!
//! | Signal   | Arduino | MCU pin | Notes                                  |
//! |----------|---------|---------|----------------------------------------|
//! | SCK      | D13     | PE15    | `SPI5_SCK`  (AF5)                      |
//! | MISO     | D12     | PH8     | `SPI5_MISO` (AF5)                      |
//! | MOSI     | D11     | PG2     | `SPI5_MOSI` (AF5)                      |
//! | CS       | D10     | PA3     | `SPI5_NSS`, used as plain GPIO         |
//! | RDY      | D3      | PE9     | EXTI9                                  |
//! | CHIP_EN  | D5      | PE10    | module enable                          |
//!
//! The DK has no M.2 socket (there is no wireless sheet in the schematic), so
//! the module is jumpered to the Arduino header.
//!
//! What this proves, in order: the SPI link clocks, RDY/EXTI fires, the
//! full-duplex frame protocol frames correctly, and the AT layer gets a reply.
//! The `AT+GMR` response tells us whether the module runs T01 or T02 — which
//! decides whether Milestone B is `embassy-net` (T02) or AT sockets (T01).
//!
//! Run: `cargo run --release` from this directory (loads to RAM via probe-rs).
//!
//! Not yet verified on hardware — see README notes for the two or three spots
//! most likely to need adjusting.

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{error, info, unwrap};
use embassy_executor::Spawner;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_stm32::{bind_interrupts, Config};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use st67w611::bus::engine::{Engine, Outbound};
use st67w611::bus::frame::MAX_PAYLOAD;

// RDY is on PE9 -> EXTI9.
bind_interrupts!(struct Irqs {
    EXTI9 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI9>;
});

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

/// Adapts embassy's *blocking* SPI to the async `SpiBus` the driver expects.
///
/// Bring-up deliberately avoids DMA: no GPDMA channel names, no interrupt
/// binding, nothing chip-specific to get wrong. The driver is the only thing
/// pacing the bus (it waits on RDY), so busy-waiting transfers are fine here.
struct AsyncSpi<S>(S);

impl<S> AsyncSpi<S> {
    fn new(inner: S) -> Self {
        Self(inner)
    }
}

impl<S: embedded_hal::spi::SpiBus> embedded_hal_async::spi::ErrorType for AsyncSpi<S> {
    type Error = S::Error;
}

impl<S: embedded_hal::spi::SpiBus> embedded_hal_async::spi::SpiBus for AsyncSpi<S> {
    async fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.0.read(words)
    }
    async fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.0.write(words)
    }
    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        self.0.transfer(read, write)
    }
    async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.0.transfer_in_place(words)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush()
    }
}

static RDY_SIGNAL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
static HDR_ACK_SIGNAL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
static RDY_LEVEL: StaticCell<AtomicBool> = StaticCell::new();

/// Mirror the RDY pin, signalling the engine on every edge.
///
/// The engine samples `rdy_level` after being woken, so the atomic must be
/// updated *before* the signal.
#[embassy_executor::task]
async fn rdy_task(
    mut rdy: ExtiInput<'static>,
    signal: &'static Signal<CriticalSectionRawMutex, ()>,
    level: &'static AtomicBool,
) {
    level.store(rdy.is_high(), Ordering::Relaxed);
    signal.signal(());
    loop {
        rdy.wait_for_high().await;
        level.store(true, Ordering::Relaxed);
        signal.signal(());
        rdy.wait_for_low().await;
        level.store(false, Ordering::Relaxed);
        signal.signal(());
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Config::default());
    info!("st67w611 / STM32N6570-DK bring-up (Arduino-header wiring)");

    // Module enable (D5 / PE10). Polarity is not documented anywhere I could
    // find — if the module never responds, try Level::Low.
    let _en = Output::new(p.PE10, Level::High, Speed::Low);
    info!("step: CHIP_EN high");

    // CS (D10 / PA3). This driver drives CS **active high**, so idle is low.
    // (The old T01 `SpiTransport` was active-low; that inconsistency is a known
    // follow-up in the crate.)
    let cs = Output::new(p.PA3, Level::Low, Speed::Low);
    info!("step: CS idle low");

    // SPI5 on the Arduino header.
    info!("step: creating SPI5 (blocking) ...");
    let spi = AsyncSpi::new(Spi::new_blocking(
        p.SPI5,
        p.PE15,
        p.PG2,
        p.PH8,
        SpiConfig::default(),
    ));
    info!("step: SPI5 ready");

    // RDY (D3 / PE9), active-high when the module has something to send.
    let rdy = ExtiInput::new(p.PE9, p.EXTI9, Pull::None, Irqs);
    info!("step: RDY/EXTI9 ready");

    let ready = RDY_SIGNAL.init(Signal::new());
    // The DK has no HDR_ACK net; the engine tolerates it never firing.
    let hdr_ack = HDR_ACK_SIGNAL.init(Signal::new());
    let level = RDY_LEVEL.init(AtomicBool::new(false));
    unwrap!(spawner.spawn(rdy_task(rdy, ready, level)));

    let mut engine = Engine::new(spi, cs, ready, hdr_ack, level);
    let mut rx = [0u8; MAX_PAYLOAD + 8];

    // Step 1: a bare AT. If this comes back, SPI + RDY + framing + AT all work.
    match engine.exchange(Some(Outbound::at(b"AT\r\n")), &mut rx).await {
        Ok(recv) => {
            let n = recv.len.min(rx.len());
            info!("AT -> {} bytes: {=[u8]}", n, &rx[..n]);
        }
        Err(e) => error!("AT failed: {:?}", e),
    }

    // Step 2: version/general info. The reply is what tells us T01 vs T02 —
    // it is expected to contain `mission_t01` / `mission_t02`.
    match engine
        .exchange(Some(Outbound::at(b"AT+GMR\r\n")), &mut rx)
        .await
    {
        Ok(recv) => {
            let n = recv.len.min(rx.len());
            info!("AT+GMR -> {} bytes: {=[u8]}", n, &rx[..n]);
        }
        Err(e) => error!("AT+GMR failed: {:?}", e),
    }

    // Idle. Milestone B (xarxa -> embassy-net -> DHCP) hangs off here once we
    // know which firmware is on the module.
    loop {
        Timer::after_secs(10).await;
        info!("alive");
    }
}
