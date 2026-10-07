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

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::rcc::{
    CpuClk, IcConfig, Icint, Icsel, Pll, Plldivm, Pllpdiv, Pllsel, SupplyConfig, SysClk,
};
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_stm32::{bind_interrupts, Config};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Instant, Timer};
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
///
/// Note we only signal at startup if RDY is *already* high. Signalling
/// unconditionally defeats the engine's readiness gate: it would return from
/// `ready.wait()` immediately even with RDY low and clock a module that isn't up
/// (which is exactly what made the first bring-up read back garbage instead of
/// reporting a timeout).
#[embassy_executor::task]
async fn rdy_task(
    mut rdy: ExtiInput<'static, embassy_stm32::mode::Async>,
    signal: &'static Signal<CriticalSectionRawMutex, ()>,
    level: &'static AtomicBool,
) {
    let initial = rdy.is_high();
    level.store(initial, Ordering::Relaxed);
    if initial {
        signal.signal(());
    }
    loop {
        rdy.wait_for_high().await;
        level.store(true, Ordering::Relaxed);
        signal.signal(());
        rdy.wait_for_low().await;
        level.store(false, Ordering::Relaxed);
        signal.signal(());
    }
}

/// Clock setup for the STM32N6570-DK.
///
/// Mirrors embassy's own hardware-validated N6 examples (PLL1 = 800 MHz,
/// cpu = IC1, sys = IC2). The stock default leaves PLL1 bypassed and the IC
/// muxes unset; peripheral kernel clocks derived from them then read as 0,
/// which silently disables the peripheral instead of erroring.
fn rcc_config() -> Config {
    let mut config = Config::default();

    // The DK uses an EXTERNAL SMPS (UM3300 Tab.6); embassy's internal-SMPS
    // default hangs init() at VOSRDY.
    config.rcc.supply_config = SupplyConfig::External;

    // PLL1 = HSI(64 MHz) / 4 * 50 = 800 MHz.
    config.rcc.pll1 = Some(Pll::Oscillator {
        source: Pllsel::Hsi,
        divm: Plldivm::Div4,
        fractional: 0,
        divn: 50,
        divp1: Pllpdiv::Div1,
        divp2: Pllpdiv::Div1,
    });

    config.rcc.ic1 = Some(IcConfig {
        source: Icsel::Pll1,
        divider: Icint::Div1,
    });
    let sys_ic = IcConfig {
        source: Icsel::Pll1,
        divider: Icint::Div4,
    };
    config.rcc.ic2 = Some(sys_ic);
    config.rcc.ic6 = Some(sys_ic);
    config.rcc.ic11 = Some(sys_ic);
    config.rcc.cpu = CpuClk::Ic1; // 800 MHz
    config.rcc.sys = SysClk::Ic2; // 200 MHz

    // SPI5's kernel clock is IC14. ST's own MSP sets Spi5ClockSelection = IC14
    // with IC14 = PLL1 / 20 (40 MHz here). At zero the baud generator emits no
    // SCK and a blocking SPI transfer never completes.
    config.rcc.ic14 = Some(IcConfig {
        source: Icsel::Pll1,
        divider: Icint::Div20,
    });

    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let config = rcc_config();
    let p = embassy_stm32::init(config);

    // embassy's own N6 examples do this immediately after init. Interrupts are
    // not enabled on this part at that point, and without it the time driver's
    // TIM interrupt (and every EXTI) is never delivered — which makes every
    // `with_timeout` wait forever and silently turns the driver's timeouts into
    // infinite hangs.
    unsafe {
        cortex_m::interrupt::enable();
    }

    info!("st67w611 / STM32N6570-DK bring-up (Arduino-header wiring)");

    // Module enable (D5 / PE10). Polarity is not documented anywhere I could
    // find, so drive a reset pulse: it exercises both levels and puts the module
    // through a defined power-on sequence either way.
    let mut en = Output::new(p.PE10, Level::Low, Speed::Low);
    info!("step: CHIP_EN low (reset)");
    Timer::after_millis(100).await;
    en.set_high();
    info!("step: CHIP_EN high");

    // The module needs time to boot before it will answer on SPI.
    // (Previously this delay was impossible: the time driver never ticked.)
    Timer::after_millis(500).await;
    info!("step: module settle delay done");

    // CS (D10 / PA3). This driver drives CS **active high**, so idle is low.
    // (The old T01 `SpiTransport` was active-low; that inconsistency is a known
    // follow-up in the crate.)
    let cs = Output::new(p.PA3, Level::Low, Speed::Low);
    info!("step: CS idle low");

    // SPI5 on the Arduino header.
    info!("step: creating SPI5 (blocking) ...");
    let mut spi = AsyncSpi::new(Spi::new_blocking(
        p.SPI5,
        p.PE15,
        p.PG2,
        p.PH8,
        SpiConfig::default(),
    ));
    info!("step: SPI5 ready");

    // Sanity check: can the bus be clocked at all? This bypasses the frame
    // protocol and the RDY wait, separating "the SPI peripheral has no kernel
    // clock" from "the engine is stuck waiting for RDY".
    info!("step: raw SPI transfer ...");
    let mut probe = [0u8; 8];
    let ok = embedded_hal_async::spi::SpiBus::transfer_in_place(&mut spi, &mut probe)
        .await
        .is_ok();
    info!("step: raw SPI done (ok={})", ok);

    // Does the embassy-time driver actually tick? Nothing has tested this yet,
    // and if it doesn't, every `with_timeout` in the engine silently becomes an
    // infinite wait — which matches the symptom exactly (hang, no Err(Timeout)).
    //
    // First: is the hardware counter even running? A busy poll on Instant::now()
    // needs no interrupt, so if this loop exits, TIM5 is counting and the bug is
    // in the interrupt/wake path. If it hangs, the counter itself is dead.
    info!("step: busy-polling the time counter ...");
    let t0 = Instant::now();
    loop {
        if Instant::now() - t0 > Duration::from_millis(10) {
            break;
        }
    }
    info!("step: time counter advances");

    info!("step: waiting 500 ms for the time driver ...");
    Timer::after_millis(500).await;
    info!("step: time driver ticks");

    // RDY (D3 / PE9), active-high when the module has something to send.
    let rdy = ExtiInput::new(p.PE9, p.EXTI9, Pull::None, Irqs);
    // ST's driver gates every transfer on this line (`spi_port_is_ready()`).
    // If the module is not up — wrong CHIP_EN polarity, no power, still booting
    // — RDY stays low and clocking it anyway just reads back garbage.
    info!("step: RDY/EXTI9 ready (rdy={})", rdy.is_high());

    let ready = RDY_SIGNAL.init(Signal::new());
    // The DK has no HDR_ACK net; the engine tolerates it never firing.
    let hdr_ack = HDR_ACK_SIGNAL.init(Signal::new());
    let level = RDY_LEVEL.init(AtomicBool::new(false));
    // embassy-executor 0.10: the *task function* returns
    // `Result<SpawnToken, SpawnError>` (fallible task pools), while
    // `spawn()` itself returns `()`.
    spawner.spawn(rdy_task(rdy, ready, level).unwrap());

    let mut engine = Engine::new(spi, cs, ready, hdr_ack, level);
    let mut rx = [0u8; MAX_PAYLOAD + 8];

    // The module emits a "\r\nready\r\n" banner while it boots, and it is
    // sitting in the queue waiting for us. Drain it first: otherwise every
    // response we read is offset by one — which is exactly what we saw, with a
    // bare AT appearing to answer "ready" and AT+GMR appearing to answer "OK".
    info!("step: draining pending frames ...");
    for i in 0..4 {
        match with_timeout(
            Duration::from_millis(150),
            engine.exchange(None, &mut rx),
        )
        .await
        {
            Ok(Ok(recv)) => {
                let n = recv.len.min(rx.len());
                info!("  drain[{}]: {} bytes: {=[u8]}", i, n, &rx[..n]);
            }
            _ => break,
        }
    }

    // Step 1: a bare AT. If this comes back, SPI + RDY + framing + AT all work.
    match engine.exchange(Some(Outbound::at(b"AT\r\n")), &mut rx).await {
        Ok(recv) => {
            let n = recv.len.min(rx.len());
            info!("AT -> {} bytes: {=[u8]}", n, &rx[..n]);
        }
        Err(e) => error!("AT failed: {:?}", e),
    }

    // Step 2: firmware identification.
    //
    // `AT+GMR` is the command ST's own driver uses for this
    // (w61_at_sys.c: W61_AT_Send `AT+GMR`, parsing lines such as
    // "AT version:", "component_version_macsw_", "component_version_sdk_…").
    // The SDK version is what distinguishes the two firmware builds:
    // T01 is 2.0.97, T02 is 2.0.106.
    info!("step: AT+GMR (firmware identification) ...");
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

    // The version block spans several lines/frames, so drain the rest of it.
    info!("step: draining AT+GMR remainder ...");
    for i in 0..6 {
        match with_timeout(
            Duration::from_millis(150),
            engine.exchange(None, &mut rx),
        )
        .await
        {
            Ok(Ok(recv)) => {
                let n = recv.len.min(rx.len());
                info!("  gmR[{}]: {} bytes: {=[u8]}", i, n, &rx[..n]);
            }
            _ => break,
        }
    }

    // Idle. Milestone B (xarxa -> embassy-net -> DHCP) hangs off here once we
    // know which firmware is on the module.
    loop {
        Timer::after_secs(10).await;
        info!("alive");
    }
}
