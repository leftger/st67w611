//! ST67W611 Firmware Programming Tool for STM32WBA55CG
//!
//! This example puts the ST67W611 module into UART bootloader mode

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
    info!("=== ST67W611 Firmware Programming Tool (STM32WBA55CG) ===");

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

    // Configure control pins
    info!("Configuring control pins...");
    let mut boot_pin = Output::new(p.PB0, Level::Low, Speed::Low); // BOOT control
    let mut chip_en = Output::new(p.PB14, Level::Low, Speed::Low); // CHIP_EN control
    info!("  BOOT (PB0): LOW");
    info!("  CHIP_EN (PB14): LOW");

    info!("");
    info!("=== ENTERING BOOTLOADER MODE ===");
    info!("");

    // BOOTLOADER ENTRY SEQUENCE
    info!("Step 1: Setting BOOT = HIGH (bootloader mode)...");
    boot_pin.set_high();
    info!("  BOOT pin is HIGH");

    info!("Step 2: Resetting module...");
    info!("  Pulling CHIP_EN LOW...");
    chip_en.set_low();
    Timer::after(Duration::from_millis(100)).await;
    info!("  Releasing CHIP_EN HIGH...");
    chip_en.set_high();
    info!("  Reset pulse complete");

    info!("Step 3: Waiting 100ms for bootloader to start...");
    Timer::after(Duration::from_millis(100)).await;

    info!("Step 4: Clearing BOOT = LOW...");
    boot_pin.set_low();
    info!("  BOOT pin is LOW");

    info!("");
    info!("=== BOOTLOADER ACTIVE ===");
    info!("Module is now in UART bootloader mode.");
    info!("Connect USB-UART to X-NUCLEO CN7 and use QConn_Flash tool.");

    // Keep module in bootloader mode
    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("Bootloader active...");
    }
}
