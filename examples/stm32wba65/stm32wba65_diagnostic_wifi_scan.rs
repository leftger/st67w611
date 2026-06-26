//! Simplified ST67W611 WiFi scan example for debugging (STM32WBA65RI)

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
    spi::{Config as SpiConfig, Spi},
    time::Hertz,
    Config,
};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("=== ST67W611 WiFi Scan - Simplified Test (STM32WBA65RI) ===");

    let mut config = Config::default();

    // Configure PLL1 for 96 MHz system clock
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
    info!("MCU initialized successfully");

    // Configure BOOT pin (PB0): Set LOW for AT mode
    info!("Configuring BOOT pin on PB0 (LOW=AT mode)...");
    let _boot_pin = Output::new(p.PB0, Level::Low, Speed::Low);
    info!("BOOT pin set to LOW (AT mode)");

    // Configure WIFI_CHIP_ENABLE (PB14): Start LOW, then enable after delay
    // CRITICAL: Datasheet requires minimum 3.3ms delay between power-up and CHIP_EN going HIGH
    info!("Configuring WIFI_CHIP_ENABLE on PB14 (starts LOW)...");
    let mut wifi_chip_enable = Output::new(p.PB14, Level::Low, Speed::Low);
    info!("Waiting 10ms before enabling chip (datasheet requires >3.3ms)...");
    Timer::after(Duration::from_millis(10)).await;

    // Now enable the chip
    info!("Setting CHIP_EN HIGH to enable ST67W611...");
    wifi_chip_enable.set_high();
    info!("ST67W611 chip enabled");

    // Configure SPI
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = Hertz(10_000_000);

    let _spi = Spi::new(
        p.SPI2,
        p.PB10,  // CLK
        p.PC3, // MOSI
        p.PA9,  // MISO
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );
    info!("SPI configured successfully");

    // Configure CS pin (PB9) as manual output
    // IMPORTANT: ST67W611 uses active-HIGH CS (HIGH=active, LOW=inactive)
    let _cs = Output::new(p.PB9, Level::Low, Speed::VeryHigh);
    info!("CS pin configured (starts LOW/inactive)");

    info!("All hardware initialized - entering loop");

    loop {
        info!("Tick...");
        Timer::after(Duration::from_secs(1)).await;
    }
}
