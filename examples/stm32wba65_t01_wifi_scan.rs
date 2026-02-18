//! ST67W611 WiFi scan on STM32WBA65RI (T01 firmware)
//!
//! Pinout:
//!   CHIP_EN  → PH3
//!   SPI_CLK  → PB4  (SPI1_SCK)
//!   SPI_MISO → PB3  (SPI1_MISO)
//!   SPI_MOSI → PA15 (SPI1_MOSI)
//!   WIFI_CS  → PD14 (active HIGH)
//!   WIFI_RDY → PD8  (EXTI8)
//!
//! # Build & Flash
//!
//! ```bash
//! cargo build --example stm32wba65_t01_wifi_scan --features "mission-t01,defmt" --release
//! probe-rs run --chip STM32WBA65RI \
//!     target/thumbv8m.main-none-eabihf/release/examples/stm32wba65_t01_wifi_scan
//! ```

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    exti::ExtiInput,
    gpio::{Level, Output, Pull, Speed},
    rcc::{
        AHB5Prescaler, AHBPrescaler, APBPrescaler, PllDiv, PllMul, PllPreDiv, PllSource, Sysclk,
        VoltageScale,
    },
    spi::{Config as SpiConfig, Spi},
    time::Hertz,
    Config,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use st67w611::bus::SpiTransportRdy;
use {defmt_rtt as _, panic_probe as _};

// WIFI_RDY is on PD8 → EXTI8
bind_interrupts!(struct Irqs {
    EXTI8 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI8>;
});

// RDY flow-control signals shared between the monitor task and the transport
static TXN_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();

use embassy_stm32::mode::Async;
use embassy_stm32::spi::mode::Master;
type SpiType = Spi<'static, Async, Master>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("=== ST67W611 WiFi Scan (STM32WBA65RI, T01) ===");

    // 96 MHz clock: HSI × 30 / 5
    let mut config = Config::default();
    config.rcc.pll1 = Some(embassy_stm32::rcc::Pll {
        source: PllSource::HSI,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL30,
        divr: Some(PllDiv::DIV5),
        divq: None,
        divp: Some(PllDiv::DIV30),
        frac: Some(0),
    });
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    config.rcc.apb7_pre = APBPrescaler::DIV1;
    config.rcc.ahb5_pre = AHB5Prescaler::DIV4;
    config.rcc.voltage_scale = VoltageScale::RANGE1;
    config.rcc.sys = Sysclk::PLL1_R;

    let p = embassy_stm32::init(config);
    info!("MCU initialized (96 MHz)");

    // CHIP_EN: start LOW, then assert HIGH to power the module
    let mut chip_en = Output::new(p.PH3, Level::Low, Speed::Low);
    Timer::after(Duration::from_millis(10)).await;
    chip_en.set_high();
    info!("CHIP_EN HIGH — module powering up");

    // Wait for module boot (T01 firmware typically ready within 1-2 s)
    Timer::after(Duration::from_secs(2)).await;

    // SPI1: CLK=PB4, MOSI=PA15, MISO=PB3 at 10 MHz
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = Hertz(10_000_000);
    let spi = Spi::new(
        p.SPI1,
        p.PB4,  // CLK
        p.PA15, // MOSI
        p.PB3,  // MISO
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );

    // CS: PD14, active HIGH — start LOW (inactive)
    let cs = Output::new(p.PD14, Level::Low, Speed::VeryHigh);

    // WIFI_RDY: PD8 with EXTI8
    let wifi_rdy = ExtiInput::new(p.PD8, p.EXTI8, Pull::None, Irqs);
    info!("WIFI_RDY initial state: {}", wifi_rdy.is_high());

    // If RDY is already HIGH at startup, pre-signal so the transport
    // doesn't block waiting for the first rising edge.
    if wifi_rdy.is_high() {
        TXN_READY.signal(());
    }

    unwrap!(spawner.spawn(rdy_monitor_task(wifi_rdy)));

    Timer::after(Duration::from_millis(100)).await;

    // Build SPI transport with RDY flow control
    let mut transport = SpiTransportRdy::new(spi, cs, &TXN_READY, &HDR_ACK);

    // ── Step 1: AT (basic comms test) ──────────────────────────────────────
    info!("Sending AT...");
    match transport.write(b"AT\r\n").await {
        Ok(n) => info!("  TX {} bytes", n),
        Err(e) => {
            error!("  TX failed: {:?}", e);
            loop {
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    }

    let mut rx = [0u8; 256];
    match transport.read(&mut rx).await {
        Ok(n) if n > 0 => info!("  RX: {:a}", &rx[..n]),
        Ok(_) => warn!("  RX: empty"),
        Err(e) => error!("  RX failed: {:?}", e),
    }

    Timer::after(Duration::from_millis(200)).await;

    // ── Step 2: AT+CWMODE=1 (station mode) ─────────────────────────────────
    info!("Setting station mode (AT+CWMODE=1)...");
    match transport.write(b"AT+CWMODE=1\r\n").await {
        Ok(_) => {}
        Err(e) => error!("  Failed: {:?}", e),
    }
    match transport.read(&mut rx).await {
        Ok(n) if n > 0 => info!("  {:a}", &rx[..n]),
        _ => {}
    }

    Timer::after(Duration::from_millis(500)).await;

    // ── Step 3: AT+CWLAP (WiFi scan) ───────────────────────────────────────
    info!("Scanning for WiFi networks (AT+CWLAP)...");
    match transport.write(b"AT+CWLAP\r\n").await {
        Ok(_) => {}
        Err(e) => {
            error!("  Scan command failed: {:?}", e);
            loop {
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    }

    // Collect +CWLAP responses until we see OK or timeout (~15 s)
    let deadline = embassy_time::Instant::now() + Duration::from_secs(15);
    let mut ap_count = 0u32;

    loop {
        if embassy_time::Instant::now() > deadline {
            warn!("Scan timeout reached");
            break;
        }

        match transport.read(&mut rx).await {
            Ok(0) => {}
            Ok(n) => {
                let response = &rx[..n];
                // Print raw response for analysis
                info!("  {:a}", response);

                // Count +CWLAP entries
                for line in response.split(|&b| b == b'\n') {
                    if line.starts_with(b"+CWLAP") {
                        ap_count += 1;
                    }
                    // Scan is done when module sends "OK"
                    if line == b"OK\r" || line == b"OK" {
                        info!("Scan complete — {} APs found", ap_count);
                        loop {
                            Timer::after(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
            Err(e) => {
                error!("Read error: {:?}", e);
                break;
            }
        }
    }

    info!("Scan ended — {} APs seen", ap_count);

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// Monitors WIFI_RDY edges and signals the transport accordingly.
///
/// Rising edge  → transaction ready (module has data or is ready to accept)
/// Falling edge → header acknowledged
#[embassy_executor::task]
async fn rdy_monitor_task(mut rdy: ExtiInput<'static>) {
    loop {
        rdy.wait_for_rising_edge().await;
        TXN_READY.signal(());

        rdy.wait_for_falling_edge().await;
        HDR_ACK.signal(());
    }
}
