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

use core::fmt::Write as _;
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_stm32::bind_interrupts;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::{TcpListener, TcpSocket};
use embassy_net::udp::UdpSocket;
use embassy_net::{Stack, StackStorage, StaticPool};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use embedded_io_async::Write;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use st67w611::net::xarxa::{self, HardwareAddress, State as XarxaState, WifiDevice};

mod secrets;

// RDY is on PB13 -> EXTI13; the hardware RNG feeds the stack seed and the
// live telemetry stream.
bind_interrupts!(struct Irqs {
    EXTI13 => embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI13>;
    RNG => embassy_stm32::rng::InterruptHandler<embassy_stm32::peripherals::RNG>;
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

/// Connection and request counters for the live dashboard.
static CONNECTIONS: AtomicU32 = AtomicU32::new(0);
static REQUESTS: AtomicU32 = AtomicU32::new(0);

/// Seconds between the NTP epoch (1900) and the Unix epoch (1970).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// Last NTP sync: Unix seconds at the instant of the reply, plus the local
/// monotonic instant it was received at. `None` until the first sync.
static NTP_SYNC: Mutex<CriticalSectionRawMutex, Cell<Option<(u64, Instant)>>> =
    Mutex::new(Cell::new(None));

/// Current wall time as Unix seconds, extrapolated from the last NTP sync with
/// the monotonic timer, or `None` if we have never synced.
fn unix_now() -> Option<u64> {
    NTP_SYNC
        .lock(|c| c.get())
        .map(|(secs, at)| secs + (Instant::now() - at).as_secs())
}

/// Seconds since the last sync, or `None` if we have never synced.
fn sync_age() -> Option<u64> {
    NTP_SYNC
        .lock(|c| c.get())
        .map(|(_, at)| (Instant::now() - at).as_secs())
}

/// SNTP client: asks `pool.ntp.org` for the time and re-syncs every minute.
///
/// The request is the classic 48-byte SNTP packet (LI=0, VN=3, mode=3); the
/// reply carries the transmit timestamp at bytes 40..48, big-endian 32.32
/// fixed point. Only the integer seconds are used, which is plenty for a
/// dashboard clock.
#[embassy_executor::task]
async fn ntp_task(stack: Stack<'static>) {
    const NTP_PORT: u16 = 123;
    let mut no_dns_logged = false;
    loop {
        // Resolve the pool name. DHCP has already installed the DNS servers.
        let addr = match stack.dns_query("pool.ntp.org", DnsQueryType::A).await {
            Ok(addrs) if !addrs.is_empty() => addrs[0],
            Ok(_) => {
                if !no_dns_logged {
                    warn!("ntp: no A record for pool.ntp.org");
                    no_dns_logged = true;
                }
                Timer::after_secs(10).await;
                continue;
            }
            Err(e) => {
                if !no_dns_logged {
                    warn!("ntp: DNS lookup failed: {:?}", e);
                    no_dns_logged = true;
                }
                Timer::after_secs(10).await;
                continue;
            }
        };

        let mut sock = match UdpSocket::new(stack) {
            Ok(s) => s,
            Err(_) => {
                warn!("ntp: no free UDP socket in the stack");
                Timer::after_secs(10).await;
                continue;
            }
        };
        if sock.bind(0u16, (addr, NTP_PORT)).is_err() {
            Timer::after_secs(10).await;
            continue;
        }

        // LI=0 (no leap warning), VN=3 (version 3), mode=3 (client).
        let mut req = [0u8; 48];
        req[0] = 0x1B;
        if sock.send_to(&req, (addr, NTP_PORT)).await.is_err() {
            Timer::after_secs(10).await;
            continue;
        }

        let mut buf = [0u8; 48];
        let reply = with_timeout(Duration::from_secs(5), sock.recv_from_with(|data, _meta| {
            let n = data.len().min(buf.len());
            buf[..n].copy_from_slice(&data[..n]);
            n
        }))
        .await;

        match reply {
            Ok(Ok(n)) if n >= 48 => {
                let secs = u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]) as u64;
                if secs != 0 {
                    let unix = secs.wrapping_sub(NTP_UNIX_OFFSET);
                    NTP_SYNC.lock(|c| c.set(Some((unix, Instant::now()))));
                    info!("ntp: synced from {} at unix {}", addr, unix);
                    no_dns_logged = false;
                }
            }
            Ok(Ok(_)) => warn!("ntp: short reply from {}", addr),
            Ok(Err(e)) => warn!("ntp: receive error: {:?}", e),
            Err(_) => warn!("ntp: no reply from {} within 5 s", addr),
        }

        Timer::after_secs(60).await;
    }
}

/// True if the request line is `GET <path> ...`.
fn wants(req: &[u8], path: &[u8]) -> bool {
    req.starts_with(b"GET ")
        && req.get(4..4 + path.len()) == Some(path)
        && req.get(4 + path.len()) == Some(&b' ')
}

/// Close the transmit half gracefully, then drain until the peer closes too.
///
/// Writing a FIN with data still pending would drop that data, and dropping
/// the socket outright sends a RST, which makes clients throw the response
/// away before reading it.
async fn close_and_drain(sock: &mut TcpSocket<'_, '_>) {
    sock.close();
    let mut discard = [0u8; 64];
    loop {
        match sock.read(&mut discard).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// Serves the live dashboard over HTTP on TCP port 80.
///
/// Three routes on the same listener:
///
/// * `GET /events` — server-sent events, one `data:` line every 500 ms with
///   uptime, counters and 8 bytes from the hardware RNG. The browser keeps
///   this connection open and updates the page as data arrives.
/// * `GET /api/stats` — the same snapshot as a one-shot `text/plain` body,
///   handy for `curl`.
/// * anything else — the dashboard page, which opens an `EventSource` to
///   `/events` and draws the RNG bytes into a canvas.
#[embassy_executor::task]
async fn web_task(stack: Stack<'static>, mut rng: Rng<'static, embassy_stm32::mode::Async>) {
    // Socket buffers, one connection at a time. The page is a few KB, so the
    // transmit side is larger than the request side.
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 2048];

    let mut listener = match TcpListener::new(stack) {
        Ok(listener) => listener,
        Err(_) => {
            error!("web: no TCP listener slot");
            return;
        }
    };
    if let Err(e) = listener.listen(80u16) {
        error!("web: listen(80) failed: {:?}", e);
        return;
    }
    info!("web: HTTP server listening on port 80");

    loop {
        let token = match listener.accept().await {
            Ok(token) => token,
            Err(_) => continue,
        };

        let mut sock = match TcpSocket::new(stack, &mut rx, &mut tx) {
            Ok(sock) => sock,
            Err(_) => continue,
        };
        if sock.accept(token).await.is_err() {
            continue;
        }

        CONNECTIONS.fetch_add(1, Ordering::Relaxed);

        let mut req = [0u8; 256];
        let n = match sock.read(&mut req).await {
            Ok(n) => n,
            Err(_) => {
                close_and_drain(&mut sock).await;
                continue;
            }
        };
        let req = &req[..n];

        if wants(req, b"/events") {
            const SSE_HEAD: &str = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/event-stream\r\n",
                "Cache-Control: no-store\r\n",
                "Connection: keep-alive\r\n",
                "\r\n",
                ": stream open\n\n"
            );
            let _ = sock.write_all(SSE_HEAD.as_bytes()).await;
            loop {
                // SSE frame: one `data:` line, blank line to dispatch it.
                let mut line = heapless::String::<192>::new();
                let mut bytes = [0u8; 8];
                if rng.fill_bytes(&mut bytes).await.is_err() {
                    break;
                }
                let up = Instant::now().as_millis();
                // `t` is Unix seconds (0 until the first NTP sync), `age` is
                // seconds since the last sync (-1 when never synced).
                let t = unix_now().unwrap_or(0);
                let age = sync_age().map(|a| a as i64).unwrap_or(-1);
                let _ = write!(
                    line,
                    "data: up={}.{:03} conns={} reqs={} t={} age={} rng=",
                    up / 1000,
                    up % 1000,
                    CONNECTIONS.load(Ordering::Relaxed),
                    REQUESTS.load(Ordering::Relaxed),
                    t,
                    age
                );
                for b in bytes {
                    let _ = write!(line, "{:02x}", b);
                }
                let _ = line.push_str("\n\n");
                if sock.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                Timer::after_millis(500).await;
            }
        } else if wants(req, b"/api/stats") {
            REQUESTS.fetch_add(1, Ordering::Relaxed);
            let mut body = heapless::String::<128>::new();
            let mut bytes = [0u8; 8];
            let _ = rng.fill_bytes(&mut bytes).await;
            let up = Instant::now().as_millis();
            let t = unix_now().unwrap_or(0);
            let age = sync_age().map(|a| a as i64).unwrap_or(-1);
            let _ = write!(
                body,
                "up={}.{:03} conns={} reqs={} t={} age={} rng=",
                up / 1000,
                up % 1000,
                CONNECTIONS.load(Ordering::Relaxed),
                REQUESTS.load(Ordering::Relaxed),
                t,
                age
            );
            for b in bytes {
                let _ = write!(body, "{:02x}", b);
            }
            let _ = body.push('\n');

            let mut hdr = heapless::String::<160>::new();
            let _ = write!(
                hdr,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(hdr.as_bytes()).await;
            let _ = sock.write_all(body.as_bytes()).await;
        } else {
            REQUESTS.fetch_add(1, Ordering::Relaxed);
            const HEADER: &str = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/html; charset=utf-8\r\n",
                "Cache-Control: no-store\r\n",
                "Connection: close\r\n",
                "\r\n"
            );
            let _ = sock.write_all(HEADER.as_bytes()).await;
            let _ = sock.write_all(INDEX_HTML.as_bytes()).await;
        }

        close_and_drain(&mut sock).await;
    }
}

/// The dashboard page. No frameworks, no build step: one canvas and one
/// `EventSource`.
const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>ST67W611 live</title>
<style>
:root{color-scheme:dark;--bg:#0d1117;--card:#161b22;--bd:#30363d;--ac:#58a6ff;--ok:#3fb950}
*{box-sizing:border-box}
body{margin:0;font-family:system-ui,-apple-system,sans-serif;background:var(--bg);color:#e6edf3;padding:18px}
.wrap{max-width:900px;margin:0 auto;display:grid;gap:14px}
h1{margin:0;font-size:1.15rem;color:var(--ac);display:flex;align-items:center;gap:10px}
.dot{width:9px;height:9px;border-radius:50%;background:#484f58;display:inline-block;transition:background .2s}
.dot.on{background:var(--ok);box-shadow:0 0 8px var(--ok)}
.dot.off{background:#f85149}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px}
.card{background:var(--card);border:1px solid var(--bd);border-radius:10px;padding:14px}
.k{font-size:.72rem;letter-spacing:.08em;text-transform:uppercase;color:#8b949e}
.v{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:1.5rem;color:var(--ok);margin-top:6px}
canvas{width:100%;height:auto;border-radius:10px;border:1px solid var(--bd);background:#0d1117;display:block}
.f{margin:0;color:#8b949e;font-size:.78rem}
</style></head><body><div class="wrap">
<h1><span id="dot" class="dot"></span>ST67W611 live telemetry</h1>
<p class="f">T02 firmware &middot; embassy-net/xarxa over SPI Wi-Fi &middot; server-sent events</p>
<div class="grid">
<div class="card"><div class="k">utc time (ntp)</div><div class="v" id="clock">&mdash;</div><div class="k" id="sync" style="margin-top:4px">waiting for sync</div></div>
<div class="card"><div class="k">uptime</div><div class="v" id="up">&mdash;</div></div>
<div class="card"><div class="k">connections</div><div class="v" id="conns">&mdash;</div></div>
<div class="card"><div class="k">requests</div><div class="v" id="reqs">&mdash;</div></div>
</div>
<canvas id="c" width="880" height="180"></canvas>
</div>
<script>
const cv=document.getElementById('c'),cx=cv.getContext('2d');
let hist=[];
// Wall-clock state: the last NTP-derived value plus when the browser saw it,
// so the clock keeps ticking between 500 ms updates.
let base=null,baseAt=0;
function clockText(){
  if(base===null)return '\u2014';
  const t=base+(performance.now()-baseAt)/1000;
  const d=new Date(t*1000);
  return d.toISOString().slice(11,19)+' UTC';
}
function tick(){document.getElementById('clock').textContent=clockText();}
function draw(){
  const w=cv.width,h=cv.height;
  cx.fillStyle='#0d1117';cx.fillRect(0,0,w,h);
  cx.strokeStyle='#21262d';cx.lineWidth=1;cx.beginPath();
  for(let i=1;i<4;i++){const y=Math.round(h*i/4)+.5;cx.moveTo(0,y);cx.lineTo(w,y);}
  cx.stroke();
  const n=hist.length,bw=w/128;
  for(let i=0;i<n;i++){
    const x=w-(n-i)*bw,bh=(hist[i]/255)*(h-10);
    cx.fillStyle='#58a6ff';cx.fillRect(x,h-bh-2,Math.max(bw-.5,.5),bh);
  }
}
const dot=document.getElementById('dot');
const es=new EventSource('/events');
es.onopen=()=>dot.className='dot on';
es.onerror=()=>dot.className='dot off';
es.onmessage=e=>{
  const d={};
  e.data.split(' ').forEach(kv=>{const i=kv.indexOf('=');if(i>0)d[kv.slice(0,i)]=kv.slice(i+1);});
  document.getElementById('up').textContent=(d.up||'\u2014')+' s';
  document.getElementById('conns').textContent=d.conns||'\u2014';
  document.getElementById('reqs').textContent=d.reqs||'\u2014';
  const t=parseInt(d.t||'0');
  if(t>0){base=t;baseAt=performance.now();}
  const age=parseInt(d.age||'-1');
  document.getElementById('sync').textContent = age<0 ? 'waiting for sync' : ('synced '+age+' s ago');
  if(d.rng){for(const b of d.rng.match(/../g)||[])hist.push(parseInt(b,16));hist=hist.slice(-128);}
  tick();draw();
};
setInterval(tick,200);
draw();tick();
</script></body></html>
"#;

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
    // The RNG defaults to the LSE clock, which this board never starts (no
    // 32.768 kHz crystal guaranteed), so point it at the always-on HSI instead.
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux.rngsel = embassy_stm32::rcc::mux::Rngsel::Hsi;
    let p = embassy_stm32::init(config);
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
    //
    // The stack seed comes from the on-chip hardware RNG instead of a constant,
    // so TCP initial sequence numbers are not predictable from the firmware
    // source. The same peripheral feeds the dashboard's live RNG stream.
    let mut rng = Rng::new(p.RNG, Irqs);
    let mut seed_bytes = [0u8; 8];
    unwrap!(rng.fill_bytes(&mut seed_bytes).await);
    let seed = u64::from_le_bytes(seed_bytes);
    info!("net: stack seeded from hardware RNG");

    static STACK_STORAGE: StaticCell<StackStorage<'static>> = StaticCell::new();
    let (stack, net_runner) = Stack::new(
        STACK_STORAGE.init(StackStorage::new()),
        &POOL,
        seed,
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
            spawner.spawn(unwrap!(web_task(stack, rng)));
            spawner.spawn(unwrap!(ntp_task(stack)));
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
