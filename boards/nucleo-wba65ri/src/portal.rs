//! USB provisioning portal.
//!
//! The board enumerates on the **user USB-C connector (CN9)** as a CDC-NCM
//! Ethernet gadget. The host picks up 192.168.7.10..20 over DHCP and can open
//! <http://192.168.7.1/> to:
//!
//! * see the access points the module can hear (`AT+CWLAP`, scanned on every
//!   page load),
//! * pick one, type the password, and join it,
//! * have the module remember it (`CWJAP` already stores credentials, plus an
//!   explicit auto-connect flag) so the board reconnects on the next power-up,
//! * read the addresses the board currently has, both on Wi-Fi (the DHCP lease
//!   of the host's embassy-net stack) and on this USB link.
//!
//! This link is independent of Wi-Fi, so it works at a new location where the
//! stored network is absent — exactly the "take it somewhere else" case.

use core::fmt::Write as _;

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_net::iface::dhcpv4_server::DhcpServerConfig;
use embassy_net::iface::Iface;
use embassy_net::tcp::{TcpListener, TcpSocket};
use embassy_net::wire::{IpAddr, IpCidr, Ipv4Addr};
use embassy_net::{Stack, StackStorage, StaticPool};
use embassy_stm32::interrupt::typelevel::Binding;
use embassy_stm32::peripherals;
use embassy_stm32::usb::{self, Driver};
use embassy_usb::class::cdc_ncm::device::embassy_net::{Device, Runner, State as NetState};
use embassy_usb::class::cdc_ncm::device::{CdcNcmClass, Config as NcmConfig, State as NcmState};
use embassy_usb::{Builder as UsbBuilder, UsbDevice, UsbDeviceSpeed};
use embedded_io_async::Write;
use st67w611::net::xarxa::Control;
use static_cell::StaticCell;

/// USB peripheral driver, named so the task signatures stay readable.
pub type UsbDriver = Driver<'static, peripherals::USB_OTG_HS>;
type NcmRunner = Runner<'static, UsbDriver>;

/// Where the board's Wi-Fi address comes from.
///
/// * **T02**: the module is only a MAC/PHY, and the address is the DHCP lease of
///   the host's own `embassy-net` interface, so the portal reads it from there.
/// * **T01**: the module owns TCP/IP, so the portal asks it (`AT+CIPSTA?`).
///
/// The portal itself is identical either way: it only needs the module's AT link
/// to scan and join, and its own USB stack to serve the page.
// Each firmware that includes this file constructs only one of the two variants
// (the T02 dashboard uses `Host`, the T01 web server `Module`), so the other is
// "never constructed" from that binary's point of view.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum WifiLink {
    /// T02: read the address from the host stack's Wi-Fi interface.
    Host(Iface<'static>),
    /// T01: ask the module.
    Module,
}

impl WifiLink {
    /// The IPv4 address(es) on Wi-Fi, comma-separated; empty if none yet.
    async fn ip(self, control: &Control<'static>) -> heapless::String<160> {
        match self {
            WifiLink::Host(iface) => format_ip_addrs(iface),
            WifiLink::Module => module_ip(control).await,
        }
    }
}

/// The station address the module reports (`+CIPSTA:ip:"a.b.c.d"`), if it has one.
async fn module_ip(control: &Control<'static>) -> heapless::String<160> {
    let mut out = heapless::String::<160>::new();
    let Ok(reply) = control.at("AT+CIPSTA?").await else {
        return out;
    };
    for line in reply.lines.iter() {
        let l = line.as_str();
        if !l.starts_with("+CIPSTA:ip") {
            continue;
        }
        let Some(start) = l.find('"') else { continue };
        let Some(len) = l[start + 1..].find('"') else {
            continue;
        };
        let ip = &l[start + 1..start + 1 + len];
        if ip != "0.0.0.0" && !ip.is_empty() {
            let _ = out.push_str(ip);
        }
    }
    out
}

/// Runs the USB device state machine.
#[embassy_executor::task]
pub async fn usb_task(mut device: UsbDevice<'static, UsbDriver>) -> ! {
    device.run().await
}

/// Shuttles CDC-NCM packets between USB and the embassy-net device.
#[embassy_executor::task]
pub async fn ncm_task(runner: NcmRunner) -> ! {
    runner.run().await
}

/// Drives the USB-side network stack (ARP, DHCP server, TCP).
#[embassy_executor::task]
pub async fn usb_net_task(mut runner: embassy_net::Runner<'static>) -> ! {
    runner.run().await
}

/// Page shown at the root: scan results, credentials form, current status.
const INDEX_HEAD: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>st67w611 Wi-Fi setup</title>
<style>
:root{color-scheme:dark;--bg:#0d1117;--card:#161b22;--bd:#30363d;--ac:#58a6ff;--ok:#3fb950}
*{box-sizing:border-box}
body{margin:0;font-family:system-ui,-apple-system,sans-serif;background:var(--bg);color:#e6edf3;padding:20px}
.wrap{max-width:520px;margin:0 auto;display:grid;gap:14px}
h1{margin:0;font-size:1.1rem;color:var(--ac)}
.card{background:var(--card);border:1px solid var(--bd);border-radius:10px;padding:16px}
.k{font-size:.72rem;letter-spacing:.08em;text-transform:uppercase;color:#8b949e;margin-bottom:8px}
label{display:block;font-size:.85rem;margin:12px 0 4px;color:#8b949e}
select,input{width:100%;background:#0d1117;color:#e6edf3;border:1px solid var(--bd);border-radius:7px;padding:9px;font-size:.9rem}
button{width:100%;margin-top:14px;background:#1f6feb;border:1px solid #388bfd;color:#fff;border-radius:7px;padding:10px;font-size:.9rem;cursor:pointer}
button.alt{background:#21262d;border-color:var(--bd)}
.msg{border-left:3px solid var(--ok);padding:8px 10px;background:#0f2916;border-radius:6px;font-size:.88rem}
.ip{margin-top:8px;font-size:.9rem}
.ip code{background:#0d1117;border:1px solid var(--bd);border-radius:5px;padding:1px 6px;color:var(--ok)}
.f{color:#8b949e;font-size:.78rem}
</style></head><body><div class="wrap">
<h1>st67w611 &middot; Wi-Fi setup</h1>
"#;

/// Renders the page body: status, message, scan list and the form.
///
/// Scanning is done on demand (each load) because the module's scan takes a
/// couple of seconds and the list is only useful when freshly taken.
async fn render_page(
    control: &Control<'static>,
    wifi: WifiLink,
    usb_iface: Iface<'static>,
    message: Option<&str>,
) -> heapless::String<6144> {
    let mut out = heapless::String::<6144>::new();
    let _ = out.push_str(INDEX_HEAD);

    if let Some(msg) = message {
        let _ = write!(out, "<div class=\"card\"><div class=\"msg\">");
        push_escaped(&mut out, msg);
        let _ = out.push_str("</div></div>\n");
    }

    // Current association, if any.
    let mut status = heapless::String::<160>::new();
    let mut associated = false;
    match control.at("AT+CWJAP?").await {
        Ok(o) if o.is_ok() => {
            associated = true;
            if let Some(line) = o.lines.first() {
                let _ = write!(status, "{}", line.as_str());
            }
        }
        _ => {
            let _ = status.push_str("not associated");
        }
    }

    // The Wi-Fi address lives in the host's embassy-net stack on T02: the
    // module is only a MAC/PHY there, so `AT+CIPSTA?` would not have it. The
    // USB address comes from this portal's own stack.
    let wifi_ip = wifi.ip(control).await;
    let mut usb_ip = format_ip_addrs(usb_iface);
    if usb_ip.is_empty() {
        let _ = usb_ip.push_str("no address");
    }

    let _ = out.push_str("<div class=\"card\"><div class=\"k\">current network</div><div>");
    push_escaped(&mut out, status.as_str());
    let _ = out.push_str("</div>\n");
    // Show the wireless address only while the station is associated. DHCP
    // may still be in flight right after joining, so say so explicitly.
    if associated {
        let _ = out.push_str("<div class=\"ip\">Wi-Fi IP: <code>");
        if wifi_ip.is_empty() {
            let _ = out.push_str("waiting for DHCP");
        } else if wifi_ip.contains(',') {
            push_escaped(&mut out, wifi_ip.as_str());
        } else {
            // One address: make it a link. It opens only from a machine on that
            // same Wi-Fi network, which is exactly the point of showing it.
            let _ = out.push_str("<a href=\"http://");
            push_escaped(&mut out, wifi_ip.as_str());
            let _ = out.push_str("/\">");
            push_escaped(&mut out, wifi_ip.as_str());
            let _ = out.push_str("</a>");
        }
        let _ = out.push_str("</code></div>\n");
    }
    let _ = out.push_str("<div class=\"ip\">USB IP: <code>");
    push_escaped(&mut out, usb_ip.as_str());
    let _ = out.push_str("</code></div></div>\n");

    // Fresh scan.
    let scan = control.scan().await;
    let mut options = heapless::String::<4096>::new();
    let mut seen = 0usize;
    match scan {
        Ok(o) => {
            for line in &o.lines {
                if add_option(&mut options, line.as_str()) {
                    seen += 1;
                }
            }
        }
        Err(e) => warn!("portal: scan failed: {:?}", e),
    }

    let _ = out.push_str("<div class=\"card\"><div class=\"k\">available networks");
    let _ = write!(out, " ({})", seen);
    let _ = out.push_str("</div>\n<form method=\"get\" action=\"/connect\">\n");
    let _ =
        out.push_str("<label for=\"ssid\">network</label>\n<select id=\"ssid\" name=\"ssid\">\n");
    if seen == 0 {
        let _ = out.push_str("<option value=\"\">(no networks found - press rescan)</option>\n");
    }
    let _ = out.push_str(options.as_str());
    let _ = out.push_str("</select>\n<label for=\"pass\">password</label>\n");
    let _ = out.push_str("<input id=\"pass\" name=\"pass\" type=\"password\" autocomplete=\"off\" placeholder=\"leave empty for open networks\">\n");
    let _ = out.push_str("<button type=\"submit\">Connect and remember</button>\n</form>\n");
    let _ = out.push_str("<form method=\"get\" action=\"/\"><button class=\"alt\" type=\"submit\">Rescan</button></form>\n");
    let _ = out.push_str("</div>\n<p class=\"f\">Served over USB at 192.168.7.1 &middot; credentials are stored in the module and re-used on boot.</p>\n</div></body></html>\n");
    out
}

/// Format the IPv4 addresses assigned to an interface, comma-separated.
///
/// The prefix length is intentionally left off: the value is meant to be
/// pasted into a browser's address bar, and the page labels which link each
/// one belongs to. IPv6 is skipped because an Ethernet interface always gets
/// an automatic (link-local) address, which is not useful here.
fn format_ip_addrs(iface: Iface<'static>) -> heapless::String<160> {
    let mut out = heapless::String::<160>::new();
    for addr in iface.ip_addrs() {
        if !addr.cidr.is_ipv4() {
            continue;
        }
        if !out.is_empty() {
            let _ = out.push_str(", ");
        }
        if write!(out, "{}", addr.cidr.address()).is_err() {
            break;
        }
    }
    out
}

/// Append one `<option>` for a `+CWLAP` line; returns `false` if the line did
/// not look like a scan result (or the SSID was already listed).
fn add_option(out: &mut heapless::String<4096>, line: &str) -> bool {
    let Some(lp) = line.find('(') else {
        return false;
    };
    let inner = &line[lp + 1..];

    // Security code is the first field, before the SSID.
    let sec = inner.split(',').next().unwrap_or("");
    // The module uses the Espressif-style extended enum (0 open .. 7 mixed
    // WPA2/WPA3, 8+ WAPI/enterprise variants).
    let sec_label = match sec.trim() {
        "0" => "open",
        "1" => "WEP",
        "2" => "WPA",
        "3" => "WPA2",
        "4" => "WPA/WPA2",
        "5" => "WPA2-ENT",
        "6" => "WPA3",
        "7" => "WPA2/WPA3",
        _ => "?",
    };

    // SSID is the first quoted string.
    let Some(q1) = inner.find('"') else {
        return false;
    };
    let rest = &inner[q1 + 1..];
    let Some(q2) = rest.find('"') else {
        return false;
    };
    let ssid = &rest[..q2];
    if ssid.is_empty() {
        return false;
    }

    // RSSI is the field right after the closing quote.
    let after = &rest[q2 + 1..];
    let rssi = after
        .trim_start_matches(',')
        .split(',')
        .next()
        .unwrap_or("?");

    // Skip duplicates: the same SSID is usually heard on several BSSIDs.
    let mut needle = heapless::String::<120>::new();
    let _ = needle.push_str("value=\"");
    let _ = needle.push_str(ssid);
    if out.as_str().contains(needle.as_str()) {
        return false;
    }

    let _ = out.push_str("<option value=\"");
    push_escaped(out, ssid);
    let _ = out.push_str("\">");
    push_escaped(out, ssid);
    let _ = write!(out, " [{} dBm, {}]", rssi.trim(), sec_label);
    let _ = out.push_str("</option>\n");
    true
}

/// HTML-escape into a fixed buffer, dropping what does not fit.
fn push_escaped<const N: usize>(out: &mut heapless::String<N>, s: &str) {
    for c in s.chars() {
        let _ = match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\r' | '\n' => out.push(' '),
            _ => out.push(c),
        };
    }
}

/// Decode `%XX` and `+` from a query-string component into a fixed buffer.
fn pct_decode(src: &str, out: &mut heapless::String<96>) {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                let _ = out.push((h * 16 + l) as char);
                i += 3;
                continue;
            }
        }
        let _ = out.push(if b == b'+' { ' ' } else { b as char });
        i += 1;
    }
}

/// Pull one `key=value` pair out of the URL's query string.
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(v);
        }
    }
    None
}

/// The provisioning HTTP server, on the USB stack, port 80.
#[embassy_executor::task]
pub async fn portal_task(
    stack: Stack<'static>,
    control: &'static Control<'static>,
    wifi: WifiLink,
    usb_iface: Iface<'static>,
) -> ! {
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 2048];

    let mut listener = loop {
        match TcpListener::new(stack) {
            Ok(l) => break l,
            Err(_) => {
                error!("portal: no TCP listener slot");
                embassy_time::Timer::after_secs(1).await;
            }
        }
    };
    if let Err(e) = listener.listen(80u16) {
        error!("portal: listen(80) failed: {:?}", e);
    }
    info!("portal: Wi-Fi setup page on http://192.168.7.1/");

    loop {
        let Ok(token) = listener.accept().await else {
            continue;
        };
        let mut sock = match TcpSocket::new(stack, &mut rx, &mut tx) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if sock.accept(token).await.is_err() {
            continue;
        }

        let mut req = [0u8; 512];
        let n = sock.read(&mut req).await.unwrap_or(0);
        let req = &req[..n];

        let (page, message, is_connect) = if req.starts_with(b"GET /connect?") {
            (None, Some(handle_connect(req, control, wifi).await), true)
        } else {
            (None, None, false)
        };
        let _ = is_connect;

        // The page is rendered after the request line is parsed, so a failed
        // join still shows the (fresh) network list with the error on top.
        let body: heapless::String<6144> = match page {
            Some(p) => p,
            None => render_page(control, wifi, usb_iface, message.as_deref()).await,
        };

        const HEADER: &str = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Cache-Control: no-store\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        let _ = sock.write_all(HEADER.as_bytes()).await;
        let _ = sock.write_all(body.as_bytes()).await;

        // Graceful close: FIN after the data, then drain until the peer goes.
        sock.close();
        let mut discard = [0u8; 64];
        while let Ok(k) = sock.read(&mut discard).await {
            if k == 0 {
                break;
            }
        }
    }
}

/// The SSID the module is associated with right now (`+CWJAP:"<ssid>",...`).
async fn current_ssid(control: &Control<'static>) -> Option<heapless::String<96>> {
    let reply = control.at("AT+CWJAP?").await.ok()?;
    for line in reply.lines.iter() {
        let l = line.as_str();
        if !l.starts_with("+CWJAP:") {
            continue;
        }
        // `+CWJAP:No AP` has no quoted SSID.
        let start = l.find('"')? + 1;
        let len = l[start..].find('"')?;
        let mut ssid = heapless::String::new();
        ssid.push_str(&l[start..start + len]).ok()?;
        return Some(ssid);
    }
    None
}

/// Join the network named in the request and remember it.
///
/// Joining means leaving the current network first, so a mistyped password or a
/// network out of range would strand the board offline. The previous network is
/// therefore put back (from the module's stored credentials) if the new join fails.
async fn handle_connect(
    req: &[u8],
    control: &Control<'static>,
    wifi: WifiLink,
) -> heapless::String<256> {
    let mut msg = heapless::String::<256>::new();

    // Request line: `GET /connect?<query> HTTP/1.1`
    let text = core::str::from_utf8(req).unwrap_or("");
    let Some(start) = text.find('/') else {
        let _ = msg.push_str("bad request");
        return msg;
    };
    let rest = &text[start..];
    let Some(qmark) = rest.find('?') else {
        let _ = msg.push_str("no parameters");
        return msg;
    };
    let query = &rest[qmark + 1..];
    let query = query.split_whitespace().next().unwrap_or("");

    let mut ssid = heapless::String::<96>::new();
    let mut pass = heapless::String::<96>::new();
    if let Some(v) = query_param(query, "ssid") {
        pct_decode(v, &mut ssid);
    }
    if let Some(v) = query_param(query, "pass") {
        pct_decode(v, &mut pass);
    }
    if ssid.is_empty() {
        let _ = msg.push_str("Pick a network first.");
        return msg;
    }

    info!("portal: joining '{}'", ssid.as_str());
    let previous = current_ssid(control).await;
    // A failed disconnect is fine (we may not be associated).
    let _ = control.disconnect().await;

    // `AT+CWJAP` answering OK only means the module accepted the request. The
    // association (and, on T01, DHCP) finishes afterwards, and a wrong password or
    // a network out of range answers OK too. So the reply is not trusted: wait
    // until the module really reports being on the network.
    let accepted = control.connect(ssid.as_str(), pass.as_str()).await;
    let joined = match &accepted {
        Ok(_) => wait_joined(control, wifi, ssid.as_str(), 25).await,
        Err(_) => false,
    };

    if joined {
        // Belt and braces: the module stores a network once it has associated,
        // and the explicit flag makes it re-join by itself after a power cycle.
        let mut cmd = heapless::String::<160>::new();
        let _ = write!(
            cmd,
            "AT+CWCREDADD=\"{}\",\"{}\"",
            ssid.as_str(),
            pass.as_str()
        );
        if let Err(e) = control.at(cmd.as_str()).await {
            warn!("portal: could not store credentials: {:?}", e);
        }
        if let Err(e) = control.at("AT+CWAUTOCONN=1").await {
            warn!("portal: could not enable auto-connect: {:?}", e);
        }
        let _ = write!(
            msg,
            "Connected to '{}' and saved it. This page will stop working once the board leaves USB; find it on the Wi-Fi network instead.",
            ssid.as_str()
        );
        return msg;
    }

    match accepted {
        Err(e) => {
            let _ = write!(msg, "Could not join '{}': {:?}.", ssid.as_str(), e);
        }
        Ok(_) => {
            let _ = write!(
                msg,
                "Could not join '{}': no association (wrong password, or out of range?).",
                ssid.as_str()
            );
        }
    }
    warn!("portal: join of '{}' failed", ssid.as_str());

    // Forget the attempt, then get back to where the board was. Leaving it on a
    // network that never worked would strand it offline.
    let mut cmd = heapless::String::<160>::new();
    let _ = write!(cmd, "AT+CWCREDDEL=\"{}\"", ssid.as_str());
    let _ = control.at(cmd.as_str()).await;
    if let Some(prev) = previous {
        if go_back(control, wifi, prev.as_str()).await {
            info!("portal: back on '{}'", prev.as_str());
            let _ = write!(msg, " Went back to '{}'.", prev.as_str());
        } else {
            warn!("portal: could not get back to '{}'", prev.as_str());
            let _ = write!(
                msg,
                " Could not get back to '{}'; power-cycle the board.",
                prev.as_str()
            );
        }
    }
    msg
}

/// Wait up to `secs` seconds for the module to report that it is associated with
/// `ssid` and, when it owns TCP/IP (T01), that it has an address.
///
/// On T02 the address is the host stack's DHCP lease, which this function cannot
/// see, so association alone counts.
async fn wait_joined(control: &Control<'static>, wifi: WifiLink, ssid: &str, secs: u32) -> bool {
    for _ in 0..secs {
        if current_ssid(control).await.as_deref() == Some(ssid) {
            match wifi {
                WifiLink::Host(_) => return true,
                WifiLink::Module => {
                    if !wifi.ip(control).await.is_empty() {
                        return true;
                    }
                }
            }
        }
        embassy_time::Timer::after_secs(1).await;
    }
    false
}

/// Return to the network the board was on before a failed join.
///
/// The module only persists a network once it has associated with it, so the one it
/// had saved is still the old one; a reset makes it re-join that. (`AT+RST`: not
/// `AT+RESTORE`, which would erase everything.)
async fn go_back(control: &Control<'static>, wifi: WifiLink, prev: &str) -> bool {
    let _ = control.at("AT+RST").await;
    // It announces itself again when it has rebooted.
    let _ = control
        .wait_ready(embassy_time::Duration::from_secs(15))
        .await;
    wait_joined(control, wifi, prev, 30).await
}

/// Bring the portal up: USB device, NCM gadget, its own net stack, DHCP
/// server, and the config page.
pub fn spawn<B>(
    spawner: Spawner,
    usb: embassy_stm32::Peri<'static, peripherals::USB_OTG_HS>,
    dp: embassy_stm32::Peri<'static, peripherals::PD6>,
    dm: embassy_stm32::Peri<'static, peripherals::PD7>,
    irq: B,
    control: &'static Control<'static>,
    wifi: WifiLink,
) where
    B: Binding<
            <peripherals::USB_OTG_HS as usb::Instance>::Interrupt,
            usb::InterruptHandler<peripherals::USB_OTG_HS>,
        > + 'static,
{
    let mut usb_drv_cfg = embassy_stm32::usb::Config::default();
    // CN9 supplies VBUS whenever a cable is present; the sense pin is not
    // wired on this board, so do not wait for it.
    usb_drv_cfg.vbus_detection = false;

    static EP_OUT: StaticCell<[u8; 1024]> = StaticCell::new();
    let driver = Driver::new_hs(usb, dp, dm, irq, EP_OUT.init([0u8; 1024]), usb_drv_cfg);

    let mut usb_cfg = embassy_usb::Config::new(0xc0de, 0x6711);
    usb_cfg.manufacturer = Some("st67w611");
    usb_cfg.product = Some("WBA65 Wi-Fi setup");
    usb_cfg.serial_number = Some("st67w611-portal");
    usb_cfg.max_power = 500;
    usb_cfg.max_packet_size_0 = 64;
    usb_cfg.max_speed = UsbDeviceSpeed::High;

    static CONFIG_DESC: StaticCell<[u8; 512]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 512]> = StaticCell::new();

    let mut builder = UsbBuilder::new(
        driver,
        usb_cfg,
        &mut CONFIG_DESC.init([0; 512])[..],
        &mut BOS_DESC.init([0; 256])[..],
        &mut [],
        &mut CONTROL_BUF.init([0; 512])[..],
    );

    let our_mac = [0x02, 0xBA, 0x65, 0x00, 0x00, 0x01];
    let host_mac = [0x02, 0xBA, 0x65, 0x00, 0x00, 0x02];

    static NCM_STATE: StaticCell<NcmState> = StaticCell::new();
    let ncm = CdcNcmClass::new(
        &mut builder,
        NCM_STATE.init(NcmState::new()),
        NcmConfig::new(host_mac),
    );
    spawner.spawn(defmt::unwrap!(usb_task(builder.build())));

    static NET_STATE: StaticCell<NetState<4, 4>> = StaticCell::new();
    let (ncm_runner, device) =
        ncm.into_embassy_net_device::<4, 4>(NET_STATE.init(NetState::new()), our_mac, 1514);
    spawner.spawn(defmt::unwrap!(ncm_task(ncm_runner)));

    static STACK_STORAGE: StaticCell<StackStorage<'static>> = StaticCell::new();
    static POOL: StaticPool<1514, 8> = StaticPool::new();
    let (stack, runner) = Stack::new(
        STACK_STORAGE.init(StackStorage::new()),
        &POOL,
        0x5eed_0f67_11aa_1234,
    );
    spawner.spawn(defmt::unwrap!(usb_net_task(runner)));

    static DEVICE: StaticCell<Device<'static>> = StaticCell::new();
    let iface = defmt::unwrap!(stack.add_iface_borrowed(DEVICE.init(device)));
    defmt::unwrap!(iface.add_ip_addr(IpCidr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 7, 1)), 24)));
    // Hand the host 192.168.7.10..20 — the same range the sand-portal uses.
    let dhcp = DhcpServerConfig::new(
        Ipv4Addr::new(192, 168, 7, 10),
        Ipv4Addr::new(192, 168, 7, 20),
    );
    defmt::unwrap!(iface.set_dhcpv4_server(Some(dhcp)));

    spawner.spawn(defmt::unwrap!(portal_task(stack, control, wifi, iface)));
}
