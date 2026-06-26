//! Proof of concept: WiFi commands via RDY transport
//!
//! Tests WiFi mode setting and scan using the new RDY flow control

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

// Bind EXTI interrupt for WIFI_RDY pin (PB6 → EXTI6)
bind_interrupts!(struct Irqs {
    EXTI6 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI6>;
});

// Static signals for RDY flow control
static TXN_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();

use embassy_stm32::mode::Async;
use embassy_stm32::spi::mode::Master;
type SpiType = Spi<'static, Async, Master>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("=== ST67W611 WiFi Test with RDY (WBA55CG) ===");

    // Initialize MCU
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
    info!("MCU initialized");

    // Configure BOOT (PB0 = LOW for AT mode)
    let _boot_pin = Output::new(p.PB0, Level::Low, Speed::Low);

    // Configure CHIP_EN (PB14)
    let mut chip_en = Output::new(p.PB14, Level::Low, Speed::Low);
    Timer::after(Duration::from_millis(10)).await;
    chip_en.set_high();
    info!("Chip enabled");

    // Wait for boot
    Timer::after(Duration::from_secs(2)).await;

    // Configure SPI
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = Hertz(10_000_000);
    let spi = Spi::new(
        p.SPI1,
        p.PB4,
        p.PA15,
        p.PB3,
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );
    let cs = Output::new(p.PA12, Level::Low, Speed::VeryHigh);

    // Configure WIFI_RDY with EXTI
    let wifi_rdy = ExtiInput::new(p.PB6, p.EXTI6, Pull::None, Irqs);
    if wifi_rdy.is_high() {
        TXN_READY.signal(());
    }

    unwrap!(spawner.spawn(rdy_monitor_task(wifi_rdy)));
    Timer::after(Duration::from_millis(100)).await;

    let mut spi_transport = SpiTransportRdy::new(spi, cs, &TXN_READY, &HDR_ACK);

    // Test 1: AT handshake
    info!("Test 1: AT handshake");
    match spi_transport.write(b"AT\r\n").await {
        Ok(_) => {
            let mut buf = [0u8; 256];
            if let Ok(len) = spi_transport.read(&mut buf).await {
                info!("  Response: {:a}", &buf[..len]);
            }
        }
        Err(e) => error!("  Failed: {:?}", e),
    }

    Timer::after(Duration::from_millis(500)).await;

    // First, drain any boot messages
    info!("Draining any buffered boot messages...");
    for _i in 0..5 {
        let mut buf = [0u8; 256];
        match spi_transport.read(&mut buf).await {
            Ok(len) if len > 0 => info!("  Drained: {:a}", &buf[..len]),
            _ => break,
        }
        Timer::after(Duration::from_millis(100)).await;
    }

    Timer::after(Duration::from_millis(500)).await;

    // Test 2: Set WiFi mode to Station
    info!("Test 2: Set WiFi mode to Station");
    match spi_transport.write(b"AT+CWMODE=1\r\n").await {
        Ok(_) => {
            Timer::after(Duration::from_millis(500)).await;
            let mut buf = [0u8; 256];
            if let Ok(len) = spi_transport.read(&mut buf).await {
                if len > 0 {
                    info!("  Response: {:a}", &buf[..len]);
                } else {
                    info!("  No response");
                }
            }
        }
        Err(e) => error!("  Failed: {:?}", e),
    }

    Timer::after(Duration::from_millis(500)).await;

    // Test 2b: Configure scan options (required before scan!)
    info!("Test 2b: Configure scan options");
    match spi_transport
        .write(b"AT+CWLAPOPT=1,1695,-100,255,20\r\n")
        .await
    {
        Ok(_) => {
            Timer::after(Duration::from_millis(500)).await;
            let mut buf = [0u8; 256];
            if let Ok(len) = spi_transport.read(&mut buf).await {
                if len > 0 {
                    info!("  Scan opts response: {:a}", &buf[..len]);
                } else {
                    info!("  No response");
                }
            }
        }
        Err(e) => error!("  Failed: {:?}", e),
    }

    Timer::after(Duration::from_millis(500)).await;

    // Test 3: WiFi scan (takes 5-10 seconds!)
    info!("Test 3: WiFi scan - this will take 10+ seconds...");
    match spi_transport.write(b"AT+CWLAP=1\r\n").await {
        Ok(_) => {
            info!("  Scan command sent!");
            info!("  Module is now scanning for WiFi networks...");

            // WiFi scan takes time - wait 5 seconds before first read
            Timer::after(Duration::from_secs(5)).await;

            info!("  Reading scan results (each network arrives as separate line):");
            info!("");

            let mut total_networks = 0;

            // Read for up to 15 seconds total
            for _attempt in 0..60 {
                let mut buf = [0u8; 512];
                match spi_transport.read(&mut buf).await {
                    Ok(len) if len > 0 => {
                        let data = &buf[..len];

                        // Process each line in the response
                        let mut start = 0;
                        for i in 1..len {
                            if data[i - 1] == b'\r' && data[i] == b'\n' {
                                let line = &data[start..i - 1];

                                // Check if this is a scan result line
                                if line.starts_with(b"+CWLAP:") {
                                    total_networks += 1;
                                    // Parse SSID (format: +CWLAP:(sec,"SSID",rssi,"MAC",ch,cipher,proto,wps))
                                    if let Some(ssid_start) = line.iter().position(|&b| b == b'"') {
                                        if let Some(ssid_end) =
                                            line[ssid_start + 1..].iter().position(|&b| b == b'"')
                                        {
                                            let ssid =
                                                &line[ssid_start + 1..ssid_start + 1 + ssid_end];

                                            // Try to extract RSSI (3rd field after 2nd comma)
                                            let after_ssid = &line[ssid_start + 1 + ssid_end + 2..]; // Skip ","
                                            if let Some(comma_pos) =
                                                after_ssid.iter().position(|&b| b == b',')
                                            {
                                                let rssi = &after_ssid[..comma_pos];
                                                info!(
                                                    "Network #{}: SSID=\"{:a}\" RSSI={:a}",
                                                    total_networks, ssid, rssi
                                                );
                                            } else {
                                                info!(
                                                    "Network #{}: SSID=\"{:a}\"",
                                                    total_networks, ssid
                                                );
                                            }
                                        }
                                    }
                                } else if line.starts_with(b"+CW:SCAN_DONE") {
                                    info!("");
                                    info!("SCAN_DONE! Found {} networks", total_networks);
                                } else if line.starts_with(b"OK") {
                                    info!("OK received");
                                    start = i + 1;
                                    break;
                                } else if line.len() > 0 && !line.starts_with(b"\r") {
                                    info!("  Other: {:a}", line);
                                }

                                start = i + 1;
                            }
                        }
                    }
                    Ok(_) => {
                        // No data - wait before next read
                        Timer::after(Duration::from_millis(250)).await;
                    }
                    Err(_) => {
                        // Timeout - scan might be complete
                        if total_networks > 0 {
                            info!("Scan complete! {} networks found", total_networks);
                        }
                        break;
                    }
                }

                // Brief pause between read attempts
                Timer::after(Duration::from_millis(50)).await;
            }

            if total_networks == 0 {
                error!("  No networks found - check WiFi environment or antenna");
            } else {
                info!("");
                info!("SUCCESS! Scanned {} WiFi networks", total_networks);
            }
        }
        Err(e) => error!("  Failed to send scan command: {:?}", e),
    }

    info!("All tests complete!");
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
async fn rdy_monitor_task(mut rdy: ExtiInput<'static>) {
    loop {
        rdy.wait_for_rising_edge().await;
        TXN_READY.signal(());

        rdy.wait_for_falling_edge().await;
        HDR_ACK.signal(());
    }
}
