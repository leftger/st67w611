#![no_std]
#![no_main]

//! T01 proof of concept #2: host a website from the ST67W611, built to be taken
//! on the road.
//!
//! **Over Wi-Fi** the module runs the TCP server (`AT+CIPSERVER`) and the whole
//! TCP/IP stack; this MCU just answers what arrives. Open `http://<module-ip>/`
//! from any machine on the same network:
//!
//! * `/`: "mission control", a live UTC clock from the module's own SNTP client,
//!   plus weather, the ISS position, people in space and an activity idea that the
//!   **module** fetched from other websites,
//! * `/api`: the same data as JSON (the page polls it),
//! * anything else gets a 404.
//!
//! **Over USB** the board also enumerates on the user USB-C connector (CN9) as a
//! CDC-NCM Ethernet gadget with its own DHCP server. Plug it into a laptop and open
//! <http://192.168.7.1/>: a second website, served by this MCU's own `embassy-net`
//! stack, that scans for access points and lets you pick one, type its password and
//! join it. It does not depend on Wi-Fi, so it works at a new place where the
//! stored network is absent; the module remembers what you join.
//!
//! **Meant to run unattended.** A supervisor waits for the module to be associated
//! and have an address, starts the server, and when the link drops (or the setup page
//! moves the board to another network) goes back to waiting and starts it again.
//! An independent watchdog resets the board if the supervisor ever wedges, and a
//! panic ends in a reset too, so it comes back by itself. No Wi-Fi credentials are
//! compiled in: it uses the network the module has stored, or the one you set up
//! over USB.
//!
//! How serving works: the module pushes socket events on its own (`+CIP:<link>,
//! CONNECTED`); the runner hands them to us through `Control::socket_event`. Each
//! link that shows up is read with `AT+CIPRECVDATA`, and once a request line
//! arrives we reply with `AT+CIPSEND` and close the link (`Connection: close`).
//! Whenever the server is idle it refreshes one web source (every ~20 s) over a
//! separate outbound link, so a page load is always served from cache. The USB page
//! and this server share the module's single AT channel, which `Control`
//! serialises, so a scan or join from the setup page briefly pauses the web page.
//!
//! Run it with `scripts/poc.sh t01_web_server`; to flash it so it keeps running with
//! nothing attached, `scripts/poc.sh t01_web_server --flash-only`.

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering};

use defmt::{info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_stm32::peripherals::IWDG;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_time::{Duration, Instant, Timer};

#[path = "../common.rs"]
mod common;
#[path = "../dashboard.rs"]
mod dashboard;
#[path = "../portal.rs"]
mod portal;
#[path = "../site.rs"]
mod site;
#[path = "../t01.rs"]
mod t01;

use dashboard::Dash;
use st67w611::net::client::Net;
use st67w611::net::xarxa::Control;
use st67w611::types::WiFiMode;
use st67w611::wifi::WiFi;

const PORT: u16 = 80;

/// Largest piece sent per `AT+CIPSEND`; comfortably inside one SPI frame.
const SEND_CHUNK: usize = 900;

/// The watchdog bites if it is not petted for this long.
const WATCHDOG_TIMEOUT_US: u32 = 8_000_000;

/// The supervisor must report progress at least this often (seconds) or the
/// watchdog is left to reset the board. A join or scan can hold the AT channel for
/// 30 s, so this is generous.
const STALL_LIMIT_S: u32 = 180;

/// Seconds since boot at the supervisor's last sign of life.
static LAST_PROGRESS: AtomicU32 = AtomicU32::new(0);

/// Record that the supervisor is still making progress.
fn progress() {
    LAST_PROGRESS.store(Instant::now().as_secs() as u32, Ordering::Relaxed);
}

/// Pets the watchdog while the supervisor is alive. If the supervisor wedges, this
/// stops petting and the board resets; a panic (a hard fault) stops it too.
#[embassy_executor::task]
async fn watchdog_task(mut wdt: IndependentWatchdog<'static, IWDG>) -> ! {
    wdt.unleash();
    loop {
        let idle =
            (Instant::now().as_secs() as u32).wrapping_sub(LAST_PROGRESS.load(Ordering::Relaxed));
        if idle < STALL_LIMIT_S {
            wdt.pet();
        } else {
            warn!(
                "supervisor stalled for {} s; letting the watchdog reset the board",
                idle
            );
        }
        Timer::after_secs(2).await;
    }
}

/// Link id from a socket event: `+CIP:<link>,CONNECTED`, `+CIP:<link>,DISCONNECTED`
/// or `+IPD:<link>,<len>,...`. Returns `(link, connected)`; `connected` is false
/// for a disconnect.
fn parse_event(line: &str) -> Option<(u8, bool)> {
    let rest = line
        .strip_prefix("+CIP:")
        .or_else(|| line.strip_prefix("+IPD:"))?;
    let mut fields = rest.split(',');
    let link = fields.next()?.trim().parse::<u8>().ok()?;
    let disconnected = fields
        .next()
        .is_some_and(|f| f.trim().starts_with("DISCONNECT"));
    Some((link, !disconnected))
}

/// The request path from `AT+CIPRECVDATA`, if the request has arrived.
async fn read_request(control: &Control<'_>, link: u8) -> Option<heapless::String<64>> {
    let data = control.recv(link, 1024).await.ok()?;
    site::request_path(core::str::from_utf8(&data).ok()?)
}

/// One `AT+CIPSEND`.
async fn send(control: &Control<'_>, link: u8, bytes: &[u8]) -> bool {
    let mut cmd = heapless::String::<40>::new();
    let _ = write!(cmd, "AT+CIPSEND={},{}", link, bytes.len());
    matches!(control.at_with_payload(&cmd, bytes).await, Ok(out) if out.is_ok())
}

/// Send a complete HTTP response, in as many sends as it takes, and close the link.
async fn respond(
    control: &Control<'_>,
    link: u8,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> bool {
    let mut head = heapless::String::<200>::new();
    let _ = write!(
        head,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    let mut ok = send(control, link, head.as_bytes()).await;
    for chunk in body.chunks(SEND_CHUNK) {
        if !ok {
            break;
        }
        ok = send(control, link, chunk).await;
    }
    let _ = t01::close(control, link).await;
    ok
}

/// Serve the dashboard until the Wi-Fi link drops, then return so the caller can
/// wait for it to come back.
async fn serve(control: &Control<'static>, dash: &mut Dash, served: &mut u32, boot: Instant) {
    // The server lives on the module, so if the module restarts (a reset, a
    // brownout, the setup page recovering from a failed join) it is gone even if
    // Wi-Fi comes straight back. Remember which boot this server belongs to.
    let boots_at_start = control.boot_count();
    // Background refresh schedule: the clock every 30 s, link health every 10 s,
    // and one web source at a time (the first lap is quick so the page fills in
    // fast, then every 20 s).
    let mut next_time = Instant::now();
    let mut next_link = Instant::now() + Duration::from_secs(10);
    let mut next_source = Instant::now();
    let mut source_idx = 0usize;
    let mut source_failures = 0u8;
    // Consecutive failed link checks; two in a row (about 20 s) means it is gone.
    let mut link_misses = 0u8;

    // Links that have connected but not yet sent a complete request.
    let mut open: heapless::Vec<u8, 8> = heapless::Vec::new();
    loop {
        progress();

        if control.boot_count() != boots_at_start {
            warn!("the module restarted; its server is gone, so setting it up again");
            // A module can announce itself more than once while it boots, so wait
            // until the count has been still for a few seconds before reacting;
            // otherwise the server is rebuilt once per announcement.
            let mut seen = control.boot_count();
            loop {
                Timer::after_secs(4).await;
                progress();
                let now = control.boot_count();
                if now == seen {
                    break;
                }
                seen = now;
            }
            dash.online = false;
            return;
        }

        if let Some(event) = control.socket_event(Duration::from_millis(150)).await {
            info!("event: {}", event.as_str());
            match parse_event(event.as_str()) {
                Some((link, true)) if link != dashboard::CLIENT_LINK => {
                    if !open.contains(&link) {
                        let _ = open.push(link);
                    }
                }
                Some((link, false)) => open.retain(|l| *l != link),
                _ => {}
            }
        }

        // Work on a copy: serving a link edits `open`.
        let pending = open.clone();
        for link in pending {
            let Some(path) = read_request(control, link).await else {
                continue;
            };
            open.retain(|l| *l != link);
            *served += 1;
            info!(
                "request #{}: GET {} (link {})",
                *served,
                path.as_str(),
                link
            );

            let ok = match path.as_str() {
                "/" | "/index.html" => {
                    respond(
                        control,
                        link,
                        "200 OK",
                        "text/html; charset=utf-8",
                        dashboard::PAGE.as_bytes(),
                    )
                    .await
                }
                "/api" => {
                    let json = dash.api_json(boot.elapsed().as_secs(), *served);
                    respond(
                        control,
                        link,
                        "200 OK",
                        "application/json; charset=utf-8",
                        json.as_bytes(),
                    )
                    .await
                }
                _ => respond(control, link, "404 Not Found", "text/plain", b"not found\n").await,
            };
            if !ok {
                warn!("  could not answer request #{}", *served);
            }
        }

        // Only do slow background work while nobody is waiting on us.
        if !open.is_empty() {
            continue;
        }
        let now = Instant::now();
        if now >= next_time {
            dash.refresh_time(control).await;
            next_time = now + Duration::from_secs(30);
        } else if now >= next_link {
            // Still on Wi-Fi? And is the address still the same? Moving to another
            // network through the USB page changes it.
            match t01::online_ip(control).await {
                Some(ip) => {
                    link_misses = 0;
                    if ip != dash.ip {
                        info!("address is now {}", ip.as_str());
                        dash.ip = ip;
                    }
                    dash.refresh_link(control).await;
                }
                None => {
                    link_misses += 1;
                    warn!("Wi-Fi link check failed ({}/2)", link_misses);
                    if link_misses >= 2 {
                        dash.online = false;
                        return;
                    }
                }
            }
            next_link = Instant::now() + Duration::from_secs(10);
        } else if now >= next_source {
            // A slow source can miss once; retry it soon (twice at most) rather
            // than leaving its card blank until the next lap.
            let ok = dash.refresh_source(control, source_idx).await;
            let retry = !ok && source_failures < 2;
            if retry {
                source_failures += 1;
            } else {
                source_failures = 0;
                source_idx += 1;
            }
            next_source = Instant::now()
                + if retry {
                    Duration::from_secs(5)
                } else if source_idx < 4 {
                    Duration::from_secs(1)
                } else {
                    Duration::from_secs(20)
                };
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("t01_web_server: hosting websites from the module (Wi-Fi) and the MCU (USB)");
    let boot = Instant::now();
    let board = common::start_usb(spawner).await;
    let control = board.control;
    progress();

    // The USB setup page comes up first: it needs no Wi-Fi, so it works at a new
    // place where the stored network is absent.
    portal::spawn(
        spawner,
        board.usb.otg,
        board.usb.dp,
        board.usb.dm,
        common::Irqs,
        control,
        portal::WifiLink::Module,
    );

    // From here on a hang or a panic ends in a reset instead of a dead board.
    spawner.spawn(unwrap!(watchdog_task(IndependentWatchdog::new(
        board.iwdg,
        WATCHDOG_TIMEOUT_US
    ))));

    if common::profile(control).await != common::Profile::T01 {
        warn!("module is not on T01; run scripts/ncp_switch.sh t01 first");
        info!("RESULT: FAILED (not T01)");
        // Keep the supervisor alive for the watchdog, and the USB page up.
        loop {
            progress();
            Timer::after_secs(10).await;
        }
    }

    // Station mode, and let the module re-join its stored network by itself (also
    // after a power cycle). Joining a *new* network is the USB page's job.
    let _ = WiFi::new(control).set_mode(WiFiMode::Station).await;
    let _ = control.at("AT+CWAUTOCONN=1").await;

    let mut dash = Dash::new();
    dash.refresh_version(control).await;
    let mut served: u32 = 0;
    let mut announced = false;

    loop {
        // ---- Offline: wait for an access point and an address.
        dash.online = false;
        let mut waited = 0u32;
        let ip = loop {
            progress();
            if let Some(ip) = t01::online_ip(control).await {
                break ip;
            }
            if waited % 30 == 0 {
                info!("waiting for Wi-Fi; set one up over USB at http://192.168.7.1/");
            }
            waited += 3;
            Timer::after_secs(3).await;
        };

        // ---- Online: (re)start the server. A server left over from before a
        // re-join is stopped first, so this is safe to repeat.
        info!("Wi-Fi is up, address {}", ip.as_str());
        dash.ip = ip.clone();
        dash.online = true;

        let net = Net::new(control);
        if !t01::setup_sockets(control).await {
            warn!("socket setup failed; retrying");
            Timer::after_secs(5).await;
            continue;
        }
        // The module allows five links in all. Keep the top one free for the
        // outbound fetches by capping the server at three.
        let _ = net.set_server_max_connections(3).await;
        // Start the module's own SNTP client; `serve` reads its clock.
        let _ = net.configure_sntp(true, 0, "pool.ntp.org").await;
        let _ = net.stop_server(1).await;
        // `AT+CIPSERVER=1,<port>,"TCP",<ca_enable>,<keepalive>`, as ST's driver sends it.
        if let Err(e) = net.start_server(PORT, "TCP", 0, 0).await {
            warn!("AT+CIPSERVER failed: {:?}; retrying", e);
            Timer::after_secs(5).await;
            continue;
        }
        if !announced {
            // The first time only, so a script watching for it sees one verdict.
            info!("RESULT: OK (serving)");
            announced = true;
        }
        info!("browse to  http://{}/", ip.as_str());

        // ---- Serve until the link drops, then start over.
        serve(control, &mut dash, &mut served, boot).await;
        warn!("Wi-Fi link lost; waiting for it to come back");
    }
}
