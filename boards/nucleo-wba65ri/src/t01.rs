//! T01 helpers: the **module** owns TCP/IP, and this MCU drives it with AT
//! commands (`AT+CIPSTART`, `AT+CIPSEND`, `AT+CIPSERVER`, ...).
//!
//! Included from the T01 bins as `#[path = "../t01.rs"] mod t01;` next to
//! `common.rs`.

#![allow(dead_code)]

use core::fmt::Write as _;

use defmt::error;
use embassy_time::{Duration, Timer};

use crate::common::{self, log_lines, quoted};
use st67w611::net::xarxa::Control;
use st67w611::wifi::WiFi;

/// Join Wi-Fi and wait for the module's own DHCP client to hand it an address.
/// Returns that address.
pub async fn join(control: &Control<'_>) -> Option<heapless::String<16>> {
    if !common::join_ap(control).await {
        return None;
    }
    let wifi = WiFi::new(control);
    for _ in 0..30 {
        if let Ok(out) = wifi.station_ip().await {
            for line in out.lines.iter() {
                let l = line.as_str();
                if l.starts_with("+CIPSTA:ip") {
                    if let Some(ip) = quoted(l) {
                        if ip != "0.0.0.0" && !ip.is_empty() {
                            let mut s = heapless::String::new();
                            let _ = s.push_str(ip);
                            return Some(s);
                        }
                    }
                }
            }
        }
        Timer::after_secs(1).await;
    }
    error!("no IPv4 address after 30 s");
    None
}

/// Put the module in the socket mode the T01 demos use: multiple connections,
/// passive receive (data is pulled with `AT+CIPRECVDATA`, never pushed as
/// `+IPD`, which the AT transport would mangle). Same init ST's driver does.
pub async fn setup_sockets(control: &Control<'_>) -> bool {
    for cmd in ["AT+CIPMUX=1", "AT+CIPRECVMODE=1"] {
        match control.at(cmd).await {
            Ok(out) if out.is_ok() => {}
            other => {
                error!("{} failed (ok={})", cmd, other.is_ok());
                return false;
            }
        }
    }
    true
}

/// `AT+CIPCLOSE=<link>`.
pub async fn close(control: &Control<'_>, link: u8) -> bool {
    let mut cmd = heapless::String::<32>::new();
    let _ = write!(cmd, "AT+CIPCLOSE={}", link);
    matches!(control.at(&cmd).await, Ok(out) if out.is_ok())
}

/// Plain HTTP GET over a module-side TCP socket (link 0).
///
/// On success, `status` holds the response's status line. Returns a short reason
/// on failure.
pub async fn http_get(
    control: &Control<'_>,
    host: &str,
    path: &str,
    status: &mut heapless::String<96>,
) -> Result<(), &'static str> {
    const LINK: u8 = 0;

    let mut cmd = heapless::String::<160>::new();
    // ST's own form: <link>,"TCP","<host>",<port>,<keepalive>,,<timeout ms>
    let _ = write!(cmd, "AT+CIPSTART={},\"TCP\",\"{}\",80,0,,5000", LINK, host);
    match control.at(&cmd).await {
        Ok(out) if out.is_ok() => {}
        Ok(out) => {
            log_lines("CIPSTART", &out);
            return Err("CIPSTART refused");
        }
        Err(_) => return Err("CIPSTART timed out"),
    }

    let mut req = heapless::String::<256>::new();
    let _ = write!(
        req,
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: st67w611-t01\r\nConnection: close\r\n\r\n",
        path, host
    );
    cmd.clear();
    let _ = write!(cmd, "AT+CIPSEND={},{}", LINK, req.len());
    match control.at_with_payload(&cmd, req.as_bytes()).await {
        Ok(out) if out.is_ok() => {}
        Ok(out) => {
            log_lines("CIPSEND", &out);
            let _ = close(control, LINK).await;
            return Err("CIPSEND refused");
        }
        Err(_) => {
            let _ = close(control, LINK).await;
            return Err("CIPSEND timed out");
        }
    }

    // Passive mode: pull whatever the module has buffered until we see a status
    // line (about 5 s at most).
    let mut result = Err("no HTTP response");
    'poll: for _ in 0..50 {
        cmd.clear();
        let _ = write!(cmd, "AT+CIPRECVDATA={},512", LINK);
        if let Ok(out) = control.at(&cmd).await {
            for line in out.lines.iter() {
                let l = line.as_str();
                if let Some(i) = l.find("HTTP/1.") {
                    let end = l[i..].len().min(95);
                    let _ = status.push_str(&l[i..i + end]);
                    result = Ok(());
                    break 'poll;
                }
            }
        }
        Timer::after(Duration::from_millis(100)).await;
    }
    let _ = close(control, LINK).await;
    result
}

/// Largest response (headers included) that [`http_fetch`] keeps.
pub const FETCH_MAX: usize = 2048;

/// First occurrence of `needle` in `haystack`.
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Plain HTTP GET over a module-side TCP socket on `link`; the response **body**
/// is left in `body` (headers stripped).
///
/// Uses HTTP/1.0 so servers answer without chunked transfer encoding, and pulls
/// the reply with `Control::recv`, which is binary-safe. Only a `200` counts.
pub async fn http_fetch(
    control: &Control<'_>,
    link: u8,
    host: &str,
    path: &str,
    body: &mut heapless::Vec<u8, FETCH_MAX>,
) -> Result<(), &'static str> {
    body.clear();
    let mut cmd = heapless::String::<160>::new();
    let _ = write!(cmd, "AT+CIPSTART={},\"TCP\",\"{}\",80,0,,5000", link, host);
    match control.at(&cmd).await {
        Ok(out) if out.is_ok() => {}
        Ok(_) => return Err("connect refused"),
        Err(_) => return Err("connect timed out"),
    }

    let mut req = heapless::String::<256>::new();
    let _ = write!(
        req,
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: st67w611-t01\r\n\r\n",
        path, host
    );
    cmd.clear();
    let _ = write!(cmd, "AT+CIPSEND={},{}", link, req.len());
    if !matches!(control.at_with_payload(&cmd, req.as_bytes()).await, Ok(out) if out.is_ok()) {
        let _ = close(control, link).await;
        return Err("send failed");
    }

    // Pull until the reply goes quiet (the server closes after an HTTP/1.0 answer).
    let mut acc = heapless::Vec::<u8, FETCH_MAX>::new();
    let mut idle = 0u8;
    for _ in 0..60 {
        match control.recv(link, 1024).await {
            Ok(chunk) if !chunk.is_empty() => {
                idle = 0;
                if acc.extend_from_slice(&chunk).is_err() {
                    break;
                }
            }
            Ok(_) => {
                idle += 1;
                if find_sub(&acc, b"\r\n\r\n").is_some() && idle >= 3 {
                    break;
                }
            }
            Err(_) => break,
        }
        Timer::after(Duration::from_millis(100)).await;
    }
    let _ = close(control, link).await;

    let split = find_sub(&acc, b"\r\n\r\n").ok_or("no HTTP response")?;
    if !acc.starts_with(b"HTTP/1.") || find_sub(&acc[..split], b" 200").is_none() {
        return Err("not a 200");
    }
    let _ = body.extend_from_slice(&acc[split + 4..]);
    Ok(())
}
