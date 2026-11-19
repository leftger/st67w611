#![no_std]
#![no_main]

//! T02 proof of concept #2: host a website from the MCU.
//!
//! The twin of `t01_web_server`, and it serves the same pages (`/`, `/json`, 404).
//! The difference is who does the work: here the **STM32 runs the TCP server**
//! (`embassy-net`'s `TcpListener`), and the module only carries raw Ethernet
//! frames. Open the address the log prints (`http://<ip>/`) from any machine on
//! the same network.
//!
//! Run it with `scripts/poc.sh t02_web_server`.

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::tcp::{TcpListener, TcpSocket};
use embassy_net::Stack;
use embassy_time::{with_timeout, Duration, Instant};
use embedded_io_async::Write as _;

#[path = "../common.rs"]
mod common;
#[path = "../site.rs"]
mod site;
#[path = "../t02.rs"]
mod t02;

const PORT: u16 = 80;

/// Close our side and drain the peer's, so a response still in flight is not
/// discarded. Writing a FIN with data pending would drop that data, and dropping
/// the socket outright sends a RST, which makes clients throw the response away.
async fn close_and_drain(sock: &mut TcpSocket<'_, '_>) {
    sock.close();
    let mut discard = [0u8; 64];
    let drain = async {
        loop {
            match sock.read(&mut discard).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    };
    let _ = with_timeout(Duration::from_secs(2), drain).await;
}

async fn serve(stack: Stack<'static>, ip: &str) -> ! {
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 2048];

    let mut listener = match TcpListener::new(stack) {
        Ok(l) => l,
        Err(_) => {
            warn!("no TCP listener slot");
            info!("RESULT: FAILED (listener)");
            common::halt().await
        }
    };
    if let Err(e) = listener.listen(PORT) {
        warn!("listen({}) failed: {:?}", PORT, e);
        info!("RESULT: FAILED (listen)");
        common::halt().await;
    }
    info!("RESULT: OK (serving)");
    info!("browse to  http://{}/", ip);

    let started = Instant::now();
    let mut served: u32 = 0;
    loop {
        let Ok(token) = listener.accept().await else {
            continue;
        };
        let Ok(mut sock) = TcpSocket::new(stack, &mut rx, &mut tx) else {
            continue;
        };
        if sock.accept(token).await.is_err() {
            continue;
        }

        let mut req = [0u8; 256];
        let n = match with_timeout(Duration::from_secs(5), sock.read(&mut req)).await {
            Ok(Ok(n)) => n,
            _ => {
                close_and_drain(&mut sock).await;
                continue;
            }
        };
        let text = core::str::from_utf8(&req[..n]).unwrap_or("");
        let Some(path) = site::request_path(text) else {
            close_and_drain(&mut sock).await;
            continue;
        };

        served += 1;
        info!("request #{}: GET {}", served, path.as_str());
        let page = site::route(
            path.as_str(),
            &site::Facts {
                profile: "T02",
                who: "This page is served by a TCP stack running on the STM32WBA65 \
                      (embassy-net); the module only carries raw Wi-Fi frames",
                ip,
                uptime_s: started.elapsed().as_secs(),
                requests: served,
            },
        );

        let mut head = heapless::String::<160>::new();
        let _ = core::fmt::write(
            &mut head,
            format_args!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                page.status,
                page.content_type,
                page.body.len()
            ),
        );
        let sent = async {
            sock.write_all(head.as_bytes()).await?;
            sock.write_all(page.body.as_bytes()).await
        };
        if with_timeout(Duration::from_secs(5), sent).await.is_err() {
            warn!("  could not answer request #{}", served);
        }
        close_and_drain(&mut sock).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("t02_web_server: hosting a website from the STM32 (embassy-net)");
    let started = common::start(spawner).await;
    let control = started.control;

    if common::profile(control).await != common::Profile::T02 {
        warn!("module is not on T02; run scripts/ncp_switch.sh t02 first");
        info!("RESULT: FAILED (not T02)");
        common::halt().await;
    }

    let Some((stack, ip)) = t02::bring_up(spawner, started).await else {
        warn!("no network: set WIFI_SSID / WIFI_PASS (scripts/poc.sh)");
        info!("RESULT: FAILED (no network)");
        common::halt().await;
    };
    serve(stack, ip.as_str()).await
}
