//! ST67W611 T02 Firmware Example with embassy-net
//!
//! This example demonstrates using the ST67W611 module with T02 firmware,
//! where the TCP/IP stack runs on the host MCU using embassy-net.
//!
//! # Requirements
//!
//! - ST67W611 module with T02 firmware (`st67w611m_mission_t02_*.bin`)
//! - STM32WBA65RI MCU
//!
//! # Architecture
//!
//! With T02 firmware, the module acts as a WiFi MAC/PHY:
//! - WiFi configuration via AT commands
//! - Raw Ethernet frames exchanged over SPI
//! - TCP/IP handled by embassy-net on the host
//!
//! # Build
//!
//! ```bash
//! cd examples/stm32wba65
//! cargo run --release --no-default-features --features mission-t02 --bin stm32wba65_t02_embassy_net
//! ```

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_stm32::{
    bind_interrupts,
    exti::ExtiInput,
    gpio::{Level, Output, Pull, Speed},
    rcc::{
        AHB5Prescaler, AHBPrescaler, APBPrescaler, PllDiv, PllMul, PllPreDiv, PllSource, Sysclk,
        VoltageScale,
    },
    spi::{mode::Master, Config as SpiConfig, Spi},
    time::Hertz,
    Config,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// Use the T02 driver
use st67w611::net::{new_driver, State, MTU};

// Bind EXTI interrupt for WIFI_RDY pin (PB13 -> EXTI13)
bind_interrupts!(struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI13>;
});

// Static signals for RDY flow control
static TXN_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HDR_ACK: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// Network stack resources
static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
static DRIVER_STATE: StaticCell<State<MTU, 4, 4>> = StaticCell::new();

// Type alias for the SPI bus
type SpiBus = Spi<'static, embassy_stm32::mode::Async, Master>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("=== ST67W611 T02 Embassy-Net Example (WBA65RI) ===");

    // Initialize MCU with PLL configuration
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
    let spi: SpiBus = Spi::new(
        p.SPI2,
        p.PB10,
        p.PC3,
        p.PA9,
        p.GPDMA1_CH0,
        p.GPDMA1_CH1,
        spi_config,
    );
    let cs = Output::new(p.PB9, Level::Low, Speed::VeryHigh);

    // Configure WIFI_RDY with EXTI
    let wifi_rdy = ExtiInput::new(p.PB13, p.EXTI13, Pull::None, Irqs);
    if wifi_rdy.is_high() {
        TXN_READY.signal(());
    }

    // Spawn RDY monitor task
    unwrap!(spawner.spawn(rdy_monitor_task(wifi_rdy)));
    Timer::after(Duration::from_millis(100)).await;

    // Create driver state and driver
    let state = DRIVER_STATE.init(State::new());
    let (device, runner) = new_driver(spi, cs, state);

    // Spawn the driver runner task
    unwrap!(spawner.spawn(wifi_runner(runner)));

    // Create network stack
    let seed = 0x1234_5678_9abc_def0; // TODO: Use hardware RNG
    let stack_resources = STACK_RESOURCES.init(StackResources::new());

    // Configure network (DHCP)
    let net_config = embassy_net::Config::dhcpv4(Default::default());

    // Create the stack (embassy-net 0.8 API)
    let (stack, stack_runner) = embassy_net::new(device, net_config, stack_resources, seed);

    // Spawn the network stack runner task
    unwrap!(spawner.spawn(net_task(stack_runner)));

    info!("Network stack initialized");
    info!("");
    info!("NOTE: WiFi association must be done via AT commands first!");
    info!("The T02 driver only handles Ethernet frame transport.");
    info!("");
    info!("To use this example:");
    info!("1. Send AT+CWMODE=1 to set station mode");
    info!("2. Send AT+CWJAP=\"SSID\",\"password\" to connect");
    info!("3. The module will then pass Ethernet frames to embassy-net");

    // Wait for link up (WiFi association)
    info!("Waiting for link up...");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("Link is up!");

    // Wait for DHCP
    info!("Waiting for DHCP...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // Main application loop - TCP client example
    info!("Starting TCP client example...");
    loop {
        // Example: Connect to a TCP server
        // let mut rx_buffer = [0; 1024];
        // let mut tx_buffer = [0; 1024];
        // let mut socket = embassy_net::tcp::TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        //
        // match socket.connect(("192.168.1.100", 80)).await {
        //     Ok(()) => {
        //         info!("Connected!");
        //         // Send/receive data...
        //     }
        //     Err(e) => {
        //         warn!("Connect failed: {:?}", e);
        //     }
        // }

        Timer::after(Duration::from_secs(10)).await;
    }
}

/// RDY monitor task - signals edges on the RDY pin
#[embassy_executor::task]
async fn rdy_monitor_task(mut rdy: ExtiInput<'static>) {
    loop {
        rdy.wait_for_rising_edge().await;
        TXN_READY.signal(());

        rdy.wait_for_falling_edge().await;
        HDR_ACK.signal(());
    }
}

/// WiFi driver runner task
#[embassy_executor::task]
async fn wifi_runner(
    runner: st67w611::net::St67w611Runner<'static, SpiBus, Output<'static>, MTU, 4, 4>,
) {
    runner.run().await
}

/// Network stack runner task
#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, st67w611::net::St67w611Device<'static, MTU, 4, 4>>,
) {
    runner.run().await
}
