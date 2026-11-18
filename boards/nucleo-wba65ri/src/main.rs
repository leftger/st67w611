#![no_std]
#![no_main]

//! st67w611 on the **NUCLEO-WBA65RI** (STM32WBA65RI, Cortex-M33).
//!
//! Why this board: on the STM32N6570-DK the USB-C cable physically blocks the
//! ST67 module from seating on the Arduino header, which left the header
//! contacts marginal — the module would sometimes answer perfectly and
//! sometimes read garbage or nothing at all. This board has the same Arduino
//! header positions, with nothing in the way.
//!
//! Wiring (Arduino header; from Zephyr's `nucleo_wba65ri` devicetree — the
//! `arduino_r3_connector.dtsi` map plus the SPI2 pinctrl):
//!
//! | Signal  | Arduino | MCU pin | Notes                        |
//! |---------|---------|---------|------------------------------|
//! | SCK     | D13     | PB10    | `SPI2_SCK`                   |
//! | MISO    | D12     | PA9     | `SPI2_MISO`                  |
//! | MOSI    | D11     | PC3     | `SPI2_MOSI`                  |
//! | CS      | D10     | PB9     | `SPI2_NSS`, used as plain GPIO |
//! | RDY     | D3      | PB13    | EXTI13                       |
//! | CHIP_EN | D5      | PB14    | module enable                |
//!
//! Run: `cargo run --release` from this directory.

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{error, info, unwrap};
use embassy_executor::Spawner;
use embassy_stm32::bind_interrupts;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_net::{Stack, StackStorage, StaticPool};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use st67w611::net::xarxa::{self, HardwareAddress, State as XarxaState, WifiDevice};

mod secrets;

// RDY is on PB13 -> EXTI13.
bind_interrupts!(struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI13>;
});

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

/// Size of the driver channel's RX/TX buffer pools.
const N_RX: usize = 4;
const N_TX: usize = 4;

/// Concrete bus/pin types — needed to name the runner task.
type Spi2Bus = AsyncSpi<
    embassy_stm32::spi::Spi<'static, embassy_stm32::mode::Blocking, embassy_stm32::spi::mode::Master>,
>;
type CsPin = Output<'static>;

/// Drives the SPI link: AT traffic and raw L2 frames share this one task.
#[embassy_executor::task]
async fn xarxa_task(runner: xarxa::Runner<'static, Spi2Bus, CsPin, N_RX, N_TX>) {
    runner.run().await
}

/// Runs the embassy-net stack itself (xarxa flavour).
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static>) {
    runner.run().await
}

/// Packet pool for the whole stack, one static so every buffer comes from the
/// same place. 1514 bytes = Ethernet MTU; the xarxa pool holds 16 of them.
static POOL: StaticPool<1514, 16> = StaticPool::new();

/// Thin async wrapper over the blocking SPI bus; this bring-up needs no DMA.
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

/// Watch the RDY pin and drive the two engine signals, the way ST's port does:
///
/// * rising edge  -> `spi_on_txn_data_ready()` -> the module is ready for a
///   transaction (we signal `ready`),
/// * falling edge -> `spi_on_header_ack()`     -> the module acknowledged our
///   header (we signal `hdr_ack`).
///
/// The level itself is also latched for the engine, because the engine skips a
/// wait when RDY is already high (which is how the reference's
/// `SPI_XFER_F_SKIP_FIRST_TXN_WAIT` path works). The reference has no edge
/// interrupt on RDY either - the level is the source of truth there too.
#[embassy_executor::task]
async fn rdy_task(
    rdy: ExtiInput<'static, embassy_stm32::mode::Async>,
    ready: &'static Signal<CriticalSectionRawMutex, ()>,
    hdr_ack: &'static Signal<CriticalSectionRawMutex, ()>,
    level: &'static AtomicBool,
) {
    let mut last = rdy.is_high();
    level.store(last, Ordering::Relaxed);
    if last {
        ready.signal(());
    }
    loop {
        Timer::after(Duration::from_micros(200)).await;
        let now_high = rdy.is_high();
        if now_high != last {
            last = now_high;
            level.store(now_high, Ordering::Relaxed);
            info!("RDY -> {}", now_high);
            if now_high {
                ready.signal(());
            } else {
                hdr_ack.signal(());
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("st67w611 bring-up on NUCLEO-WBA65RI");

    // Module enable (D5 / PB14), driven as a reset pulse so the module goes
    // through a defined power-on sequence.
    let mut en = Output::new(p.PB14, Level::Low, Speed::Low);
    Timer::after_millis(100).await;
    en.set_high();
    Timer::after_millis(500).await;
    info!("step: module power-on");

    // CS (D10 / PB9). This driver drives CS **active high**, so idle is low.
    let cs = Output::new(p.PB9, Level::Low, Speed::High);

    // SPI2 on the Arduino header: SCK=PB10, MOSI=PC3, MISO=PA9.
    // `SpiConfig::default()` is mode 0, MSB first — matching ST's SPI init.
    let spi = AsyncSpi::new(Spi::new_blocking(
        p.SPI2,
        p.PB10,
        p.PC3,
        p.PA9,
        SpiConfig::default(),
    ));

    // RDY (D3 / PB13).
    let rdy = ExtiInput::new_async(p.PB13, p.EXTI13, Pull::None, Irqs);
    let ready = RDY_SIGNAL.init(Signal::new());
    let hdr_ack = HDR_ACK_SIGNAL.init(Signal::new());
    let level = RDY_LEVEL.init(AtomicBool::new(false));
    spawner.spawn(unwrap!(rdy_task(rdy, ready, hdr_ack, level)));

    static XARXA: StaticCell<XarxaState<N_RX, N_TX>> = StaticCell::new();
    static DEVICE: StaticCell<WifiDevice<'static>> = StaticCell::new();

    // Placeholder MAC: `xarxa::new` needs one before the module can be asked
    // for its real station MAC. It is replaced via `AT+CIPSTAMAC?` below,
    // before DHCP starts.
    const MAC: [u8; 6] = [0x02, 0x00, 0x5A, 0x67, 0x61, 0x01];

    let (device, runner, control) = xarxa::new(
        spi,
        cs,
        ready,
        hdr_ack,
        level,
        XARXA.init(XarxaState::new()),
        MAC,
    );
    spawner.spawn(unwrap!(xarxa_task(runner)));

    // T02: the module is a WiFi MAC/PHY and the host runs the whole TCP/IP
    // stack. Start the stack now, but do NOT add the interface yet: the stack
    // caches the hardware address when the interface is added, and the real
    // MAC is only known after `AT+CIPSTAMAC?` below.
    static STACK_STORAGE: StaticCell<StackStorage<'static>> = StaticCell::new();
    let (stack, net_runner) = Stack::new(
        STACK_STORAGE.init(StackStorage::new()),
        &POOL,
        0x00c0_ffee_1234_5678,
    );
    spawner.spawn(unwrap!(net_task(net_runner)));
    info!("embassy-net stack running (interface attaches after MAC discovery)");

    // The module announces readiness with an UNSOLICITED "ready" line over the
    // SPI link; ST's driver blocks on exactly that and then waits a further
    // 100 ms. It can announce more than once, so wait for a "ready" and probe.
    info!("waiting for the module to become responsive ...");
    let mut responsive = false;
    for attempt in 0..6 {
        if control.wait_ready(Duration::from_secs(15)).await.is_err() {
            info!("  no `ready` yet (attempt {})", attempt + 1);
            continue;
        }
        match control.at("AT").await {
            Ok(out) if out.is_ok() => {
                info!("  module responsive after {} announcement(s)", attempt + 1);
                responsive = true;
                break;
            }
            Ok(_) => info!("  announced ready but did not answer (attempt {})", attempt + 1),
            Err(e) => info!("  probe failed (attempt {}): {:?}", attempt + 1, e),
        }
    }
    if !responsive {
        error!("module never became responsive");
    }

    // Which firmware is actually flashed? This is the definitive test, and ST's
    // own W6X_Init branches on exactly it (Core/w6x_sys.c):
    //
    //   1 = TCP/IP on the NCP   -> T01, AT sockets, the module owns the stack
    //   0 = TCP/IP on the host  -> T02, raw L2, which is what xarxa wants
    info!("step: asking the module which firmware it runs ...");
    match control.at("AT+CWNETMODE?").await {
        Ok(out) => {
            info!("  +CWNETMODE ok={}, {} line(s)", out.is_ok(), out.lines.len());
            for line in out.lines.iter() {
                info!("      |{}|", line.as_str());
            }
        }
        Err(e) => error!("  +CWNETMODE query failed: {:?}", e),
    }

    // AT+GMR carries the image identification (we previously saw
    // "SW image:spi_wifi_lwip_onhost", which reads as lwip-on-host = T02).
    match control.at("AT+GMR").await {
        Ok(out) => {
            info!("  +GMR ok={}, {} line(s)", out.is_ok(), out.lines.len());
            for line in out.lines.iter() {
                info!("      |{}|", line.as_str());
            }
        }
        Err(e) => error!("  +GMR failed: {:?}", e),
    }

    // T02: the module filters inbound unicast frames by its own Wi-Fi MAC, so
    // the host stack must transmit with that same address. Ask for it, then add
    // the interface to the stack with that address, and only then start DHCP.
    match control.station_mac().await {
        Ok(mac) => {
            info!(
                "module station MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
            control.set_hardware_address(HardwareAddress::Ethernet(mac));
            let iface = unwrap!(stack.add_iface_borrowed(DEVICE.init(device)));
            unwrap!(iface.set_dhcpv4(Some(Default::default())));
            info!("step: interface attached with the module's MAC, DHCP started");
        }
        Err(e) => error!("station MAC query failed: {:?}", e),
    }

    for i in 0..5 {
        match control.at("AT").await {
            Ok(out) => info!("  AT[{}] -> ok={}, {} line(s)", i, out.is_ok(), out.lines.len()),
            Err(e) => error!("  AT[{}] failed: {:?}", i, e),
        }
        Timer::after_millis(100).await;
    }

    info!("step: scanning for APs (AT+CWLAP) ...");
    match control.scan().await {
        Ok(out) => {
            info!("  scan ok={}, {} line(s)", out.is_ok(), out.lines.len());
            for line in out.lines.iter() {
                info!("  {}", line.as_str());
            }
        }
        Err(e) => error!("scan failed: {:?}", e),
    }

    info!("step: joining Wi-Fi ...");
    match control.connect(secrets::WIFI_SSID, secrets::WIFI_PASSWORD).await {
        Ok(()) => info!("step: Wi-Fi joined"),
        Err(e) => error!("Wi-Fi join failed: {:?}", e),
    }

    loop {
        Timer::after_secs(30).await;
        info!("alive");
    }
}
