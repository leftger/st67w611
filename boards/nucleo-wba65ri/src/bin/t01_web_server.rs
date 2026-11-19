#![no_std]
#![no_main]

//! T01 proof of concept #2: host a website from the ST67W611.
//!
//! The module runs the TCP server (`AT+CIPSERVER`) and the whole TCP/IP stack;
//! this MCU just answers what arrives. Open the address the log prints
//! (`http://<module-ip>/`) from any machine on the same network:
//!
//! * `/`: "mission control", a live UTC clock from the module's own SNTP client,
//!   plus weather, the ISS position, people in space and an activity idea that the
//!   **module** fetched from other websites,
//! * `/api`: the same data as JSON (the page polls it),
//! * anything else gets a 404.
//!
//! How it works: the module pushes socket events on its own (`+CIP:<link>,
//! CONNECTED`); the runner hands them to us through `Control::socket_event`. Each
//! link that shows up is read with `AT+CIPRECVDATA`, and once a request line
//! arrives we reply with `AT+CIPSEND` and close the link (`Connection: close`).
//! Whenever the server is idle it refreshes one web source (every ~20 s) over a
//! separate outbound link, so a page load is always served from cache.
//!
//! Run it with `scripts/poc.sh t01_web_server`.

use core::fmt::Write as _;

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant};

#[path = "../common.rs"]
mod common;
#[path = "../dashboard.rs"]
mod dashboard;
#[path = "../site.rs"]
mod site;
#[path = "../t01.rs"]
mod t01;

use dashboard::Dash;
use st67w611::net::client::Net;
use st67w611::net::xarxa::Control;

const PORT: u16 = 80;

/// Largest piece sent per `AT+CIPSEND`; comfortably inside one SPI frame.
const SEND_CHUNK: usize = 900;

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("t01_web_server: hosting a website from the module");
    let control = common::start(spawner).await.control;

    if common::profile(control).await != common::Profile::T01 {
        warn!("module is not on T01; run scripts/ncp_switch.sh t01 first");
        info!("RESULT: FAILED (not T01)");
        common::halt().await;
    }

    let Some(ip) = t01::join(control).await else {
        warn!("no network: set WIFI_SSID / WIFI_PASS (scripts/poc.sh)");
        info!("RESULT: FAILED (no network)");
        common::halt().await;
    };

    if !t01::setup_sockets(control).await {
        info!("RESULT: FAILED (socket setup)");
        common::halt().await;
    }

    // The module allows five links in all. Keep the top one free for the
    // outbound fetches by capping the server at three.
    let net = Net::new(control);
    let _ = net.set_server_max_connections(3).await;
    // Start the module's own SNTP client; the loop below reads its clock.
    let _ = net.configure_sntp(true, 0, "pool.ntp.org").await;

    // `AT+CIPSERVER=1,<port>,"TCP",<ca_enable>,<keepalive>`, as ST's driver sends it.
    if let Err(e) = net.start_server(PORT, "TCP", 0, 0).await {
        warn!("AT+CIPSERVER failed: {:?}", e);
        info!("RESULT: FAILED (server start)");
        common::halt().await;
    }
    info!("RESULT: OK (serving)");
    info!("browse to  http://{}/", ip.as_str());

    let started = Instant::now();
    let mut served: u32 = 0;
    let mut dash = Dash::new();
    dash.refresh_version(control).await;

    // Background refresh schedule: the clock every 30 s, signal strength every
    // 10 s, and one web source at a time (the first lap is quick so the page
    // fills in fast, then every 20 s).
    let mut next_time = Instant::now();
    let mut next_link = Instant::now();
    let mut next_source = Instant::now();
    let mut source_idx = 0usize;
    let mut source_failures = 0u8;

    // Links that have connected but not yet sent a complete request.
    let mut open: heapless::Vec<u8, 8> = heapless::Vec::new();
    loop {
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
            served += 1;
            info!("request #{}: GET {} (link {})", served, path.as_str(), link);

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
                    let json = dash.api_json(started.elapsed().as_secs(), served);
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
                warn!("  could not answer request #{}", served);
            }
        }

        // Only do slow background work while nobody is waiting on us.
        if open.is_empty() {
            let now = Instant::now();
            if now >= next_time {
                dash.refresh_time(control).await;
                next_time = now + Duration::from_secs(30);
            } else if now >= next_link {
                dash.refresh_link(control).await;
                next_link = now + Duration::from_secs(10);
            } else if now >= next_source {
                // A slow source can miss once; retry it soon (twice at most)
                // rather than leaving its card blank until the next lap.
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
}
