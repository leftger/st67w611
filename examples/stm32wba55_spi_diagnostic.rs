//! SPI Frame Protocol Diagnostic
//!
//! A minimal diagnostic tool to debug the ST67W611 SPI frame protocol.
//! Tests each component in isolation with detailed logging.
//!
//! # What This Tests
//!
//! 1. RDY pin state monitoring
//! 2. Raw SPI frame send/receive
//! 3. AT command round-trip timing
//! 4. Frame header validation
//!
//! # Build & Run
//!
//! ```bash
//! cargo build --example stm32wba55_spi_diagnostic --features defmt --release
//! probe-rs run --chip STM32WBA55CG --speed 4000 target/thumbv8m.main-none-eabihf/release/examples/stm32wba55_spi_diagnostic
//! ```

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    gpio::{Input, Level, Output, Pull, Speed},
    rcc::{
        AHB5Prescaler, AHBPrescaler, APBPrescaler, PllDiv, PllMul, PllPreDiv, PllSource, Sysclk,
        VoltageScale,
    },
    spi::{Config as SpiConfig, Spi},
    time::Hertz,
    Config,
};
use embassy_time::{Duration, Instant, Timer};
use {defmt_rtt as _, panic_probe as _};

// Frame protocol constants
const SPI_HEADER_MAGIC: u16 = 0x55AA;
const PADDING_BYTE: u8 = 0x88;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("╔════════════════════════════════════════════════════════════╗");
    info!("║     ST67W611 SPI Frame Protocol Diagnostic                 ║");
    info!("╚════════════════════════════════════════════════════════════╝");
    info!("");

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
    info!("[OK] MCU initialized at 96MHz");

    // ═══════════════════════════════════════════════════════════════
    // GPIO Setup
    // ═══════════════════════════════════════════════════════════════

    // BOOT pin (PB0) - LOW for AT mode
    let _boot = Output::new(p.PB0, Level::Low, Speed::Low);
    info!("[OK] BOOT pin LOW (AT mode)");

    // CHIP_EN (PB14)
    let mut chip_en = Output::new(p.PB14, Level::Low, Speed::Low);
    info!("[..] CHIP_EN LOW, waiting 50ms...");
    Timer::after(Duration::from_millis(50)).await;

    chip_en.set_high();
    info!("[OK] CHIP_EN HIGH - module powering up");

    // RDY pin (PB6) - input for monitoring (polling mode for simplicity)
    let rdy = Input::new(p.PB13, Pull::None);

    // CS pin (PA12) - active HIGH for this module
    let mut cs = Output::new(p.PB9, Level::Low, Speed::VeryHigh);

    // SPI setup
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = Hertz(10_000_000);
    let mut spi = Spi::new(
        p.SPI2,
        p.PB10,
        p.PC3,
        p.PA9,
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );
    info!("[OK] SPI configured at 10MHz");

    // ═══════════════════════════════════════════════════════════════
    // Test 1: Monitor RDY during boot
    // ═══════════════════════════════════════════════════════════════
    info!("");
    info!("┌─ TEST 1: RDY Pin Monitor During Boot ─────────────────────┐");

    let boot_start = Instant::now();
    let mut rdy_transitions = 0u32;
    let mut last_rdy = rdy.is_high();

    info!(
        "│ Initial RDY state: {}",
        if last_rdy { "HIGH" } else { "LOW" }
    );
    info!("│ Monitoring for 3 seconds...");

    while boot_start.elapsed() < Duration::from_secs(3) {
        let current_rdy = rdy.is_high();
        if current_rdy != last_rdy {
            rdy_transitions += 1;
            let elapsed_ms = boot_start.elapsed().as_millis();
            info!(
                "│  @{}ms: RDY {} -> {}",
                elapsed_ms,
                if last_rdy { "HIGH" } else { "LOW" },
                if current_rdy { "HIGH" } else { "LOW" }
            );
            last_rdy = current_rdy;
        }
        Timer::after(Duration::from_micros(100)).await;
    }

    info!("│ Total RDY transitions: {}", rdy_transitions);
    info!(
        "│ Final RDY state: {}",
        if rdy.is_high() { "HIGH" } else { "LOW" }
    );
    info!("└──────────────────────────────────────────────────────────┘");

    // ═══════════════════════════════════════════════════════════════
    // Test 2: Raw SPI Transfer (No Frame Protocol)
    // ═══════════════════════════════════════════════════════════════
    info!("");
    info!("┌─ TEST 2: Raw SPI Transfer ───────────────────────────────┐");

    // Wait for RDY HIGH
    info!("│ Waiting for RDY HIGH...");
    let wait_start = Instant::now();
    while !rdy.is_high() {
        if wait_start.elapsed() > Duration::from_secs(2) {
            error!("│ TIMEOUT waiting for RDY HIGH!");
            break;
        }
        Timer::after(Duration::from_millis(10)).await;
    }

    if rdy.is_high() {
        info!("│ RDY is HIGH after {}ms", wait_start.elapsed().as_millis());

        // Simple 8-byte read to see what's there
        cs.set_high();
        Timer::after(Duration::from_micros(10)).await;

        let mut raw_rx = [0u8; 16];
        let _ = spi.read(&mut raw_rx).await;

        Timer::after(Duration::from_micros(10)).await;
        cs.set_low();

        info!("│ Raw RX (16 bytes): {:02x}", raw_rx);
        info!("│ As text: {:a}", &raw_rx[..]);
    }
    info!("└──────────────────────────────────────────────────────────┘");

    // Small delay before next test
    Timer::after(Duration::from_millis(200)).await;

    // ═══════════════════════════════════════════════════════════════
    // Test 3: Frame Protocol - Send AT, Receive Response
    // ═══════════════════════════════════════════════════════════════
    info!("");
    info!("┌─ TEST 3: Frame Protocol - AT Command ────────────────────┐");

    let at_cmd = b"AT\r\n";
    let mut tx_frame = [0u8; 64];
    let mut rx_frame = [0u8; 64];

    // Build TX frame with header
    let payload_len = at_cmd.len();
    let padded_len = (payload_len + 3) & !3; // 4-byte align

    // Header (8 bytes)
    tx_frame[0] = (SPI_HEADER_MAGIC & 0xFF) as u8; // magic low
    tx_frame[1] = (SPI_HEADER_MAGIC >> 8) as u8; // magic high
    tx_frame[2] = (padded_len & 0xFF) as u8; // length low
    tx_frame[3] = (padded_len >> 8) as u8; // length high
    tx_frame[4] = 0; // flags (version=0, rx_stall=0)
    tx_frame[5] = 0; // type = AT command
    tx_frame[6] = 0; // reserved
    tx_frame[7] = 0; // reserved

    // Payload
    tx_frame[8..8 + payload_len].copy_from_slice(at_cmd);

    // Padding
    for i in payload_len..padded_len {
        tx_frame[8 + i] = PADDING_BYTE;
    }

    let total_len = 8 + padded_len;

    info!("│ TX Frame ({} bytes):", total_len);
    info!("│   Header: {:02x}", &tx_frame[..8]);
    info!("│   Payload: {:a}", &tx_frame[8..8 + payload_len]);

    // Wait for RDY HIGH
    info!("│ Waiting for RDY HIGH...");
    let wait_start = Instant::now();
    while !rdy.is_high() {
        if wait_start.elapsed() > Duration::from_secs(2) {
            error!("│ TIMEOUT!");
            break;
        }
        Timer::after(Duration::from_millis(1)).await;
    }

    if rdy.is_high() {
        info!("│ RDY HIGH - sending frame");

        // Assert CS
        cs.set_high();
        Timer::after(Duration::from_micros(10)).await;

        // Send frame
        let send_start = Instant::now();
        let _ = spi.write(&tx_frame[..total_len]).await;
        info!("│ TX complete in {}us", send_start.elapsed().as_micros());

        // Wait for RDY falling edge (header ack)
        info!("│ Waiting for RDY falling edge (header ack)...");
        let ack_start = Instant::now();
        while rdy.is_high() {
            if ack_start.elapsed() > Duration::from_millis(500) {
                warn!("│ No falling edge after 500ms");
                break;
            }
            Timer::after(Duration::from_micros(10)).await;
        }

        if !rdy.is_high() {
            info!("│ RDY fell after {}us", ack_start.elapsed().as_micros());
        }

        // Deassert CS
        Timer::after(Duration::from_micros(10)).await;
        cs.set_low();

        // Now wait for response
        info!("│ Waiting for RDY HIGH (response ready)...");
        let resp_start = Instant::now();
        while !rdy.is_high() {
            if resp_start.elapsed() > Duration::from_secs(2) {
                error!("│ TIMEOUT waiting for response!");
                break;
            }
            Timer::after(Duration::from_millis(1)).await;
        }

        if rdy.is_high() {
            info!(
                "│ RDY HIGH after {}ms - reading response",
                resp_start.elapsed().as_millis()
            );

            // Read response
            cs.set_high();
            Timer::after(Duration::from_micros(10)).await;

            let _ = spi.read(&mut rx_frame).await;

            // Wait for falling edge
            let ack_start = Instant::now();
            while rdy.is_high() {
                if ack_start.elapsed() > Duration::from_millis(100) {
                    break;
                }
                Timer::after(Duration::from_micros(10)).await;
            }

            Timer::after(Duration::from_micros(10)).await;
            cs.set_low();

            // Parse response header
            let rx_magic = u16::from_le_bytes([rx_frame[0], rx_frame[1]]);
            let rx_len = u16::from_le_bytes([rx_frame[2], rx_frame[3]]);
            let rx_flags = rx_frame[4];
            let rx_type = rx_frame[5];

            info!("│ RX Header:");
            info!(
                "│   Magic: 0x{:04x} {}",
                rx_magic,
                if rx_magic == SPI_HEADER_MAGIC {
                    "(VALID)"
                } else {
                    "(INVALID!)"
                }
            );
            info!("│   Length: {}", rx_len);
            info!(
                "│   Flags: 0x{:02x} (rx_stall={})",
                rx_flags,
                (rx_flags >> 2) & 1
            );
            info!(
                "│   Type: {} ({})",
                rx_type,
                match rx_type {
                    0 => "AT",
                    1 => "STA",
                    2 => "AP",
                    3 => "HCI",
                    4 => "OT",
                    _ => "?",
                }
            );

            if rx_len > 0 && rx_len < 50 {
                info!("│ RX Payload: {:a}", &rx_frame[8..8 + (rx_len as usize)]);
            }
        }
    }
    info!("└──────────────────────────────────────────────────────────┘");

    // ═══════════════════════════════════════════════════════════════
    // Test 4: Query Firmware Version
    // ═══════════════════════════════════════════════════════════════
    info!("");
    info!("┌─ TEST 4: Query Firmware Version (AT+GMR) ────────────────┐");

    Timer::after(Duration::from_millis(500)).await;

    let at_cmd = b"AT+GMR\r\n";
    let payload_len = at_cmd.len();
    let padded_len = (payload_len + 3) & !3;

    tx_frame[2] = (padded_len & 0xFF) as u8;
    tx_frame[3] = (padded_len >> 8) as u8;
    tx_frame[8..8 + payload_len].copy_from_slice(at_cmd);
    for i in payload_len..padded_len {
        tx_frame[8 + i] = PADDING_BYTE;
    }

    let total_len = 8 + padded_len;

    // Wait for RDY
    while !rdy.is_high() {
        Timer::after(Duration::from_millis(10)).await;
    }

    cs.set_high();
    Timer::after(Duration::from_micros(10)).await;
    let _ = spi.write(&tx_frame[..total_len]).await;

    // Wait for ack
    let ack_start = Instant::now();
    while rdy.is_high() && ack_start.elapsed() < Duration::from_millis(500) {
        Timer::after(Duration::from_micros(10)).await;
    }
    Timer::after(Duration::from_micros(10)).await;
    cs.set_low();

    info!("│ Command sent, reading responses...");

    // Read multiple responses (GMR returns multiple lines)
    for i in 0..5 {
        Timer::after(Duration::from_millis(200)).await;

        if !rdy.is_high() {
            // Wait for RDY
            let wait_start = Instant::now();
            while !rdy.is_high() {
                if wait_start.elapsed() > Duration::from_millis(500) {
                    break;
                }
                Timer::after(Duration::from_millis(10)).await;
            }
        }

        if rdy.is_high() {
            cs.set_high();
            Timer::after(Duration::from_micros(10)).await;

            let mut rx = [0u8; 128];
            let _ = spi.read(&mut rx).await;

            let ack_start = Instant::now();
            while rdy.is_high() && ack_start.elapsed() < Duration::from_millis(100) {
                Timer::after(Duration::from_micros(10)).await;
            }

            Timer::after(Duration::from_micros(10)).await;
            cs.set_low();

            let rx_magic = u16::from_le_bytes([rx[0], rx[1]]);
            let rx_len = u16::from_le_bytes([rx[2], rx[3]]);

            if rx_magic == SPI_HEADER_MAGIC && rx_len > 0 && rx_len < 120 {
                info!("│ Response {}: {:a}", i + 1, &rx[8..8 + (rx_len as usize)]);
            } else if rx_magic != SPI_HEADER_MAGIC {
                warn!("│ Invalid magic: 0x{:04x}", rx_magic);
                break;
            } else if rx_len == 0 {
                info!("│ Empty response (len=0)");
            }
        } else {
            info!("│ No more responses (RDY stayed LOW)");
            break;
        }
    }
    info!("└──────────────────────────────────────────────────────────┘");

    // ═══════════════════════════════════════════════════════════════
    // Test 5: Continuous RDY monitoring
    // ═══════════════════════════════════════════════════════════════
    info!("");
    info!("┌─ TEST 5: Continuous RDY Monitor ─────────────────────────┐");
    info!("│ Monitoring RDY pin for 10 seconds...");
    info!("│ (Look for unexpected transitions)");

    let monitor_start = Instant::now();
    let mut last_state = rdy.is_high();
    let mut transition_count = 0u32;

    while monitor_start.elapsed() < Duration::from_secs(10) {
        let current = rdy.is_high();
        if current != last_state {
            transition_count += 1;
            info!(
                "│  @{}ms: RDY -> {}",
                monitor_start.elapsed().as_millis(),
                if current { "HIGH" } else { "LOW" }
            );
            last_state = current;
        }
        Timer::after(Duration::from_millis(1)).await;
    }

    info!("│ Total transitions in 10s: {}", transition_count);
    info!("└──────────────────────────────────────────────────────────┘");

    info!("");
    info!("════════════════════════════════════════════════════════════");
    info!("  Diagnostic Complete");
    info!("════════════════════════════════════════════════════════════");

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
