//! Hardware bring-up shared by every proof-of-concept bin, whichever firmware
//! profile the module runs.
//!
//! The AT link is the same SPI link in both profiles, so the xarxa runner is the
//! transport for all of them, and Wi-Fi association is the same AT dialogue
//! (`AT+CWMODE`, `AT+CWJAP`). What differs is who runs TCP/IP afterwards:
//!
//! * **T01**: the module does. See `t01.rs` (AT sockets).
//! * **T02**: this MCU does, with `embassy-net`. See `t02.rs`.
//!
//! Wi-Fi credentials are baked in at build time through `WIFI_SSID` and
//! `WIFI_PASS` (see `scripts/poc.sh`). If `WIFI_SSID` is unset, the bins rely on
//! whatever network the module has stored.
//!
//! Included from each bin with `#[path = "../common.rs"] mod common;`.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_stm32::bind_interrupts;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_stm32::time::Hertz;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use st67w611::net::xarxa::{self, Control, State as XarxaState, WifiDevice};
use st67w611::types::WiFiMode;
use st67w611::wifi::WiFi;

bind_interrupts!(pub struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI13>;
    RNG => embassy_stm32::rng::InterruptHandler<embassy_stm32::peripherals::RNG>;
});

/// panic-probe 1.0 no longer registers `_defmt_panic` for you, so hook it up.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

const N_RX: usize = 4;
const N_TX: usize = 4;

/// Default per-command budget; `AT+CWJAP` and `AT+PING` can take a while.
const AT_TIMEOUT: Duration = Duration::from_secs(30);

type Spi2Bus = AsyncSpi<
    embassy_stm32::spi::Spi<
        'static,
        embassy_stm32::mode::Blocking,
        embassy_stm32::spi::mode::Master,
    >,
>;
type CsPin = Output<'static>;

#[embassy_executor::task]
async fn xarxa_task(runner: xarxa::Runner<'static, Spi2Bus, CsPin, N_RX, N_TX>) {
    runner.run().await
}

/// What `start` hands back.
pub struct Started {
    /// The AT control plane; lives for the rest of the program.
    pub control: &'static Control<'static>,
    /// The raw-frame device. Only T02 uses it (it goes into `embassy-net`).
    pub device: WifiDevice<'static>,
    /// Hardware RNG, for seeding the T02 stack.
    pub rng: Rng<'static, embassy_stm32::mode::Async>,
}

/// Reset the module, start the SPI link, and wait until it answers `AT`.
pub async fn start(spawner: Spawner) -> Started {
    // The RNG defaults to the LSE clock, which this board never starts, so point
    // it at the always-on HSI instead.
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux.rngsel = embassy_stm32::rcc::mux::Rngsel::Hsi;
    let p = embassy_stm32::init(config);

    // Reset the module so it goes through a defined boot and announces `ready`.
    let mut en = Output::new(p.PB14, Level::Low, Speed::Low);
    Timer::after_millis(100).await;
    en.set_high();
    Timer::after_millis(500).await;

    let cs = Output::new(p.PB9, Level::Low, Speed::VeryHigh);

    // The default 16 MHz core clock cannot reach 24 MHz; the HAL rounds down to
    // 8 MHz, plenty for AT traffic and these demos.
    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = Hertz(24_000_000);
    let spi = AsyncSpi(Spi::new_blocking(p.SPI2, p.PB10, p.PC3, p.PA9, spi_cfg));

    let rdy = ExtiInput::new_async(p.PB13, p.EXTI13, Pull::None, Irqs);
    static RDY_SIGNAL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
    static HDR_ACK_SIGNAL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
    static RDY_LEVEL: StaticCell<AtomicBool> = StaticCell::new();
    let ready = RDY_SIGNAL.init(Signal::new());
    let hdr_ack = HDR_ACK_SIGNAL.init(Signal::new());
    let level = RDY_LEVEL.init(AtomicBool::new(false));
    spawner.spawn(unwrap!(rdy_task(rdy, ready, hdr_ack, level)));

    static XARXA: StaticCell<XarxaState<N_RX, N_TX>> = StaticCell::new();
    static CONTROL: StaticCell<Control<'static>> = StaticCell::new();
    // Placeholder MAC: replaced by the module's own station MAC on T02 before
    // the stack starts; unused on T01.
    const MAC: [u8; 6] = [0x02, 0x00, 0x5A, 0x67, 0x61, 0x01];
    let (device, runner, mut control) = xarxa::new(
        spi,
        cs,
        ready,
        hdr_ack,
        level,
        XARXA.init(XarxaState::new()),
        MAC,
    );
    spawner.spawn(unwrap!(xarxa_task(runner)));
    control.set_timeout(AT_TIMEOUT);
    let control: &'static Control<'static> = CONTROL.init(control);
    let rng = Rng::new(p.RNG, Irqs);

    info!("waiting for the module ...");
    if !wait_responsive(control, Duration::from_secs(20)).await {
        error!("module never became responsive; check the shield is seated and powered");
        halt().await;
    }
    Started {
        control,
        device,
        rng,
    }
}

/// Wait for the module's `ready` line and prove it answers `AT`.
pub async fn wait_responsive(control: &Control<'_>, budget: Duration) -> bool {
    if control.wait_ready(budget).await.is_err() {
        return false;
    }
    for _ in 0..5 {
        if matches!(control.at("AT").await, Ok(out) if out.is_ok()) {
            return true;
        }
        Timer::after_millis(300).await;
    }
    false
}

/// The firmware profile the module reports.
#[derive(defmt::Format, PartialEq, Clone, Copy)]
pub enum Profile {
    /// TCP/IP on the module (`+CWNETMODE:1`).
    T01,
    /// TCP/IP on the host (`+CWNETMODE:0`).
    T02,
    Unknown,
}

/// Ask the module which profile it runs (`AT+CWNETMODE?`).
pub async fn profile(control: &Control<'_>) -> Profile {
    let Ok(out) = control.at("AT+CWNETMODE?").await else {
        return Profile::Unknown;
    };
    for line in out.lines.iter() {
        if let Some(rest) = line.as_str().strip_prefix("+CWNETMODE:") {
            return match rest.trim() {
                "1" => Profile::T01,
                "0" => Profile::T02,
                _ => Profile::Unknown,
            };
        }
    }
    Profile::Unknown
}

/// First double-quoted field of `s`, e.g. `+CIPSTA:ip:"10.0.0.5"` -> `10.0.0.5`.
pub fn quoted(s: &str) -> Option<&str> {
    let start = s.find('"')? + 1;
    let len = s[start..].find('"')?;
    Some(&s[start..start + len])
}

/// Associate with an access point. Identical AT dialogue on both profiles.
///
/// Uses `WIFI_SSID` / `WIFI_PASS` from the build environment when present;
/// otherwise relies on the module's stored network. Returns whether the module
/// reports an association. DHCP is a separate step (module-side on T01,
/// host-side on T02).
pub async fn join_ap(control: &Control<'_>) -> bool {
    let wifi = WiFi::new(control);
    if let Err(e) = wifi.set_mode(WiFiMode::Station).await {
        warn!("AT+CWMODE=1 failed: {:?}", e);
    }

    match option_env!("WIFI_SSID") {
        Some(ssid) if !ssid.is_empty() => {
            info!("joining \"{}\" ...", ssid);
            if let Err(e) = wifi
                .connect(ssid, option_env!("WIFI_PASS").unwrap_or(""))
                .await
            {
                error!("AT+CWJAP failed: {:?}", e);
                return false;
            }
        }
        _ => {
            info!("no WIFI_SSID at build time; relying on the module's stored network");
            let _ = control.at("AT+CWAUTOCONN=1").await;
        }
    }

    // The module re-associates by itself in the stored-network case; give it time.
    for _ in 0..20 {
        if let Ok(out) = wifi.current_ap().await {
            for line in out.lines.iter() {
                // Never echo the BSSID or password; the SSID is enough.
                if line.as_str().starts_with("+CWJAP:") {
                    if let Some(ssid) = quoted(line.as_str()) {
                        info!("associated with \"{}\"", ssid);
                        return true;
                    }
                }
            }
        }
        Timer::after_millis(500).await;
    }
    error!("module did not associate with an access point");
    false
}

/// Log the information lines of a response, bounded so a chatty reply cannot
/// flood the RTT channel.
pub fn log_lines(label: &str, out: &st67w611::at::reply::AtOutput) {
    info!(
        "  {} -> ok={}, {} line(s)",
        label,
        out.is_ok(),
        out.lines.len()
    );
    for line in out.lines.iter().take(6) {
        info!("      |{}|", line.as_str());
    }
}

/// Park forever; used after a verdict has been logged.
pub async fn halt() -> ! {
    loop {
        Timer::after_secs(3600).await;
    }
}

/// Thin async wrapper over the blocking SPI bus; these tools need no DMA.
struct AsyncSpi<S>(S);

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

/// Watch RDY and drive the engine's two signals: rising edge = module ready for
/// a transaction, falling edge = module acknowledged our header. The level is
/// latched too, because the engine skips a wait when RDY is already high.
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
            if now_high {
                ready.signal(());
            } else {
                hdr_ack.signal(());
            }
        }
    }
}
