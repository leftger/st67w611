//! ST67W611 DEBUG - WIFI_RDY Signal Monitoring (STM32WBA65RI)
//!
//! This example helps diagnose why WIFI_RDY is not going HIGH.
//! It logs detailed information about all control signals.

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
    Config,
};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// Bind EXTI interrupt for WIFI_RDY (PB13 → EXTI13)
bind_interrupts!(struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI13>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("=== ST67W611 WIFI_RDY Debug Tool (STM32WBA65RI) ===");
    info!("This tool monitors WIFI_RDY signal to diagnose boot issues");

    // Initialize with clock configuration
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

    // Configure BOOT pin (PB0): Set LOW for AT mode
    info!("Configuring BOOT pin on PB0 (LOW=AT mode)...");
    let _boot_pin = Output::new(p.PB0, Level::Low, Speed::Low);
    info!("BOOT pin set to LOW (AT mode)");

    // Configure CHIP_EN with proper timing
    info!("Configuring CHIP_EN (PB14)...");
    info!("  Starting with CHIP_EN = LOW");
    let mut wifi_chip_enable = Output::new(p.PB14, Level::Low, Speed::Low);

    // Configure WIFI_RDY as input with EXTI
    info!("Configuring WIFI_RDY (PB13) with EXTI...");
    let mut wifi_rdy = ExtiInput::new(p.PB13, p.EXTI13, Pull::None, Irqs);
    info!("  Initial WIFI_RDY state: {}", wifi_rdy.is_high());

    // Wait required delay before enabling chip
    info!("Waiting 10ms (datasheet requires >3.3ms)...");
    Timer::after(Duration::from_millis(10)).await;

    // Enable the chip
    info!("Setting CHIP_EN = HIGH to enable ST67W611...");
    wifi_chip_enable.set_high();
    info!("  CHIP_EN is now HIGH");
    info!("  Module should now be booting...");

    // Initial quick check - monitor for first 5 seconds with finer granularity
    info!("");
    info!("=== Phase 1: Monitoring first 5 seconds (detailed) ===");
    info!("*** Module should raise RDY within 1-2 seconds ***");
    info!("");

    for i in 0..50 {
        Timer::after(Duration::from_millis(100)).await;
        if i % 10 == 0 {
            info!("  t={}s, RDY={}", i / 10, wifi_rdy.is_high());
        }
    }

    info!("");
    info!("=== Phase 2: Extended monitoring (15 more seconds) ===");
    info!("Module should raise RDY within 2-4 seconds if booting correctly");
    info!("");

    let mut rdy_went_high = false;
    let mut last_state = wifi_rdy.is_high();

    for i in 0..150 {
        // 15 more seconds (100ms intervals)
        Timer::after(Duration::from_millis(100)).await;

        let current_state = wifi_rdy.is_high();
        let elapsed_ms = i * 100;

        // Log every second or when state changes
        if (i % 10 == 0) || (current_state != last_state) {
            if current_state != last_state {
                info!(
                    "  t={}ms: WIFI_RDY = {} (CHANGED!)",
                    elapsed_ms as u32, current_state
                );
            } else {
                info!("  t={}ms: WIFI_RDY = {}", elapsed_ms as u32, current_state);
            }
        }

        if current_state && !last_state {
            info!("");
            info!("*** RDY WENT HIGH at t={}ms! ***", elapsed_ms as u32);
            info!("*** Module signaled ready! ***");
            info!("");
            rdy_went_high = true;
        }

        if !current_state && last_state {
            info!("*** RDY WENT LOW at t={}ms ***", elapsed_ms as u32);
        }

        last_state = current_state;
    }

    info!("");
    info!("=== Monitoring Complete (20 seconds total) ===");
    if rdy_went_high {
        info!("SUCCESS: Module booted and signaled ready!");
        info!("Your hardware is working correctly.");
        info!("You can now proceed with the full wifi_scan example.");
    } else {
        error!("FAILURE: WIFI_RDY never went HIGH after 20 seconds");
        error!("Check power supply and connections");
    }

    // Keep running and continue monitoring
    info!("");
    info!("Continuing to monitor RDY state (Ctrl+C to stop)...");
    loop {
        Timer::after(Duration::from_secs(1)).await;
        info!("RDY = {}", wifi_rdy.is_high());
    }
}
