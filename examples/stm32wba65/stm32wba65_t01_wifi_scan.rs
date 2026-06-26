//! ST67W611 WiFi scan with proper RDY flow control
//!
//! This example uses RDY interrupt signaling as per X-CUBE reference design

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    exti::ExtiInput,
    gpio::{Input, Level, Output, Pull, Speed},
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
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// Bind EXTI interrupt for WIFI_RDY pin (PB13 → EXTI13)
bind_interrupts!(struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI13>;
});

// Static signals for RDY flow control
static TXN_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// Type aliases
use embassy_stm32::mode::Async;
use embassy_stm32::spi::mode::Master;
type SpiType = Spi<'static, Async, Master>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("=== ST67W611 WiFi Scan with RDY Flow Control (WBA65RI) ===");

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

    // Configure BOOT pin (PB0): LOW for AT mode
    info!("Configuring BOOT pin (PB0 = LOW for AT mode)...");
    let _boot_pin = Output::new(p.PB0, Level::Low, Speed::Low);

    // Configure CHIP_EN (PB14)
    info!("Configuring CHIP_EN (PB14)...");
    let mut chip_en = Output::new(p.PB14, Level::Low, Speed::Low);
    Timer::after(Duration::from_millis(10)).await;
    info!("Enabling chip (CHIP_EN = HIGH)...");
    chip_en.set_high();

    // Wait for boot
    info!("Waiting 2 seconds for module boot...");
    Timer::after(Duration::from_secs(2)).await;

    // Configure SPI
    info!("Configuring SPI2 at 10 MHz...");
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = Hertz(10_000_000);

    let spi = Spi::new(
        p.SPI2,
        p.PB10,  // CLK
        p.PC3, // MOSI
        p.PA9,  // MISO
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );
    info!("SPI configured");

    // Configure CS pin (PB9, active HIGH)
    let cs = Output::new(p.PB9, Level::Low, Speed::VeryHigh);
    info!("CS pin configured (starts LOW/inactive)");

    // Configure WIFI_RDY (PB13) with EXTI - interrupt-driven only
    info!("Configuring WIFI_RDY (PB13) with EXTI...");
    let wifi_rdy = ExtiInput::new(p.PB13, p.EXTI13, Pull::None, Irqs);
    let rdy_state = wifi_rdy.is_high();
    info!("Current WIFI_RDY state: {}", rdy_state);

    // If RDY is already HIGH, signal it immediately so transport doesn't wait
    if rdy_state {
        info!("RDY already HIGH - signaling transaction ready");
        TXN_READY.signal(());
    }

    // Spawn RDY monitor task - handles interrupts and signals transport
    unwrap!(spawner.spawn(rdy_monitor_task(wifi_rdy)));

    // Short wait for module stabilization
    Timer::after(Duration::from_millis(100)).await;

    // Create SPI transport with interrupt-driven RDY signaling
    info!("Creating SPI transport with RDY flow control...");
    let mut spi_transport = SpiTransportRdy::new(spi, cs, &TXN_READY, &HDR_ACK);

    // Try sending AT command
    info!("Sending AT command...");
    let at_cmd = b"AT\r\n";
    match spi_transport.write(at_cmd).await {
        Ok(len) => info!("AT command sent! ({} bytes)", len),
        Err(e) => {
            error!("Failed to send AT: {:?}", e);
            return;
        }
    }

    // Try reading response
    info!("Reading response...");
    let mut rx_buf = [0u8; 256];
    match spi_transport.read(&mut rx_buf).await {
        Ok(len) => {
            info!("Received {} bytes", len);
            if len > 0 {
                info!("Response: {:a}", &rx_buf[..len]);
            }
        }
        Err(e) => {
            error!("Failed to read: {:?}", e);
        }
    }

    info!("Communication test complete!");

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// RDY monitor task - handles RDY interrupts and signals transport
#[embassy_executor::task]
async fn rdy_monitor_task(mut rdy: ExtiInput<'static>) {
    info!("RDY monitor task started");

    loop {
        // Wait for rising edge (transaction ready)
        rdy.wait_for_rising_edge().await;
        info!("RDY rising edge - transaction ready");
        TXN_READY.signal(());

        // Wait for falling edge (header acknowledged)
        rdy.wait_for_falling_edge().await;
        info!("RDY falling edge - header ack");
        HDR_ACK.signal(());
    }
}
