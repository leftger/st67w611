//! Minimal test to verify STM32WBA65RI basic setup

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    gpio::{Level, Output, Speed},
    rcc::{
        AHB5Prescaler, AHBPrescaler, APBPrescaler, PllDiv, PllMul, PllPreDiv, PllSource, Sysclk,
        VoltageScale,
    },
    Config,
};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("=== Minimal Test Starting (STM32WBA65RI) ===");

    let mut config = Config::default();

    // Configure PLL1 for 96 MHz system clock (required for STM32WBA)
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

    info!("MCU initialized successfully!");

    // Configure BOOT pin (PB0): Set LOW for AT mode
    info!("Configuring BOOT pin on PB0 (LOW=AT mode)...");
    let _boot_pin = Output::new(p.PB0, Level::Low, Speed::Low);

    // Configure WIFI_CHIP_ENABLE (PB14): Start LOW, then enable after delay
    // CRITICAL: Datasheet requires minimum 3.3ms delay between power-up and CHIP_EN going HIGH
    info!("Configuring WIFI_CHIP_ENABLE on PB14 (starts LOW)...");
    let mut _wifi_chip_enable = Output::new(p.PB14, Level::Low, Speed::Low);
    info!("Waiting 10ms before enabling chip...");
    Timer::after(Duration::from_millis(10)).await;
    _wifi_chip_enable.set_high();
    info!("ST67W611 chip enabled (if connected)");

    // Blink LED on PA9 (or use another available GPIO)
    info!("Configuring test output pin on PA9...");
    let mut led = Output::new(p.PA9, Level::Low, Speed::Low);

    info!("Starting blink loop...");
    loop {
        info!("LED ON");
        led.set_high();
        Timer::after(Duration::from_millis(500)).await;

        info!("LED OFF");
        led.set_low();
        Timer::after(Duration::from_millis(500)).await;
    }
}
