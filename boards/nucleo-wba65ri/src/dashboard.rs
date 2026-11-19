//! The T01 "mission control" page: a live UTC clock from the module's own SNTP
//! client, plus snippets the module's TCP/IP stack pulls from other websites.
//!
//! Everything here runs on the MCU as AT traffic: the **module** resolves the
//! names, opens the sockets and talks HTTP, and the MCU only issues commands and
//! parses what comes back.
//!
//! Included from the T01 web server as `#[path = "../dashboard.rs"] mod dashboard;`.

#![allow(dead_code)]

use core::fmt::Write as _;

use defmt::{info, warn};
use embassy_time::Instant;

use crate::t01::{self, FETCH_MAX};
use st67w611::net::xarxa::Control;

/// The outbound link. The server's accepted connections take the low link ids
/// (the module allows 5 in total), so the client uses the top one.
pub const CLIENT_LINK: u8 = 4;

/// The page: static HTML, CSS and a little JS that polls `/api` every 5 s and
/// ticks the clock locally in between.
pub const PAGE: &str = concat!(
    "<!doctype html><meta charset=utf-8>",
    "<meta name=viewport content='width=device-width,initial-scale=1'>",
    "<title>ST67W611 mission control</title>",
    "<style>",
    ":root{color-scheme:dark}",
    "body{margin:0;padding:1.2rem;font:15px system-ui;background:#0b1020;color:#e6ebff}",
    "h1{margin:0 0 .2rem;font-size:1.3rem}small{opacity:.65}",
    ".clock{font:600 2.6rem ui-monospace,monospace;margin:.6rem 0 0}",
    ".grid{display:grid;gap:.8rem;margin-top:1rem;grid-template-columns:repeat(auto-fit,minmax(15rem,1fr))}",
    ".c{background:#151c36;border-radius:12px;padding:.8rem 1rem}",
    ".c b{display:block;font-size:.72rem;letter-spacing:.08em;opacity:.6;text-transform:uppercase}",
    ".c p{margin:.3rem 0 0;font-size:1.05rem}",
    "</style>",
    "<h1>ST67W611 <small>mission control &middot; T01</small></h1>",
    "<small>served by the Wi-Fi module's own TCP/IP stack; the data below was fetched by it too</small>",
    "<div class=clock id=t>--:--:--</div><small id=l></small>",
    "<div class=grid>",
    "<div class=c><b>Weather</b><p id=w>&hellip;</p></div>",
    "<div class=c><b>ISS right now</b><p id=i>&hellip;</p></div>",
    "<div class=c><b>People in space</b><p id=p>&hellip;</p></div>",
    "<div class=c><b>Activity of the day</b><p id=a>&hellip;</p></div>",
    "<div class=c><b>Wi-Fi link</b><p id=r>&hellip;</p></div>",
    "<div class=c><b>Module</b><p id=m>&hellip;</p></div>",
    "</div>",
    "<script>",
    "let off=0;const $=i=>document.getElementById(i);",
    "function tick(){const d=new Date(Date.now()+off);",
    "$('t').textContent=d.toISOString().slice(11,19)+' UTC';",
    "$('l').textContent='your local time: '+d.toLocaleTimeString()+' ('+d.toLocaleDateString()+')'}",
    "async function go(){try{const j=await(await fetch('/api')).json();",
    "off=j.unix*1000-Date.now();",
    "$('w').textContent=j.weather||'\\u2026';$('i').textContent=j.iss||'\\u2026';",
    "$('p').textContent=j.people?j.people+' humans in orbit':'\\u2026';",
    "$('a').textContent=j.activity||'\\u2026';",
    "$('r').textContent=j.rssi?j.rssi+' dBm \\u00b7 channel '+j.channel+' \\u00b7 '+j.ip:'\\u2026';",
    "$('m').textContent='SDK '+j.sdk+' \\u00b7 up '+j.up+' s \\u00b7 '+j.reqs+' requests'}catch(e){}}",
    "go();setInterval(go,5000);setInterval(tick,250);",
    "</script>"
);

/// Everything the page shows, refreshed in the background and served from cache.
pub struct Dash {
    pub weather: heapless::String<64>,
    pub iss: heapless::String<48>,
    pub people: u32,
    pub activity: heapless::String<96>,
    pub rssi: i32,
    pub channel: u32,
    pub sdk: heapless::String<16>,
    /// The module's current IPv4 address (changes when the board joins another
    /// network), and whether it is associated with an access point at all.
    pub ip: heapless::String<16>,
    pub online: bool,
    /// Module time at the last SNTP read, and when we read it.
    sntp_unix: u64,
    sntp_at: Option<Instant>,
}

impl Dash {
    pub fn new() -> Self {
        Self {
            weather: heapless::String::new(),
            iss: heapless::String::new(),
            people: 0,
            activity: heapless::String::new(),
            rssi: 0,
            channel: 0,
            sdk: heapless::String::new(),
            ip: heapless::String::new(),
            online: false,
            sntp_unix: 0,
            sntp_at: None,
        }
    }

    /// Current UTC as Unix seconds, extrapolated from the last SNTP read.
    pub fn unix_now(&self) -> u64 {
        match self.sntp_at {
            Some(at) => self.sntp_unix + at.elapsed().as_secs(),
            None => 0,
        }
    }

    /// Read the module's SNTP clock. The module reports 1970 until it has synced.
    pub async fn refresh_time(&mut self, control: &Control<'_>) {
        let net = st67w611::net::client::Net::new(control);
        if let Ok(s) = net.sntp_time_string().await {
            if let Some(unix) = parse_sntp(s.as_str()) {
                if unix > 1_000_000_000 {
                    self.sntp_unix = unix;
                    self.sntp_at = Some(Instant::now());
                }
            }
        }
    }

    /// Wi-Fi signal strength and channel (`AT+CWJAP?`). The SSID and BSSID are
    /// deliberately not kept: this page is readable by everyone on the network.
    ///
    /// Returns whether the module is associated with an access point, or `None`
    /// if it could not be asked (a transport hiccup is not evidence either way).
    pub async fn refresh_link(&mut self, control: &Control<'_>) -> Option<bool> {
        let out = control.at("AT+CWJAP?").await.ok()?;
        let mut associated = false;
        for line in out.lines.iter() {
            let l = line.as_str();
            // `+CWJAP:No AP` has no quoted SSID; a real association does.
            if l.starts_with("+CWJAP:") && l.contains('"') {
                associated = true;
            }
            if let Some((channel, rssi)) = parse_cwjap(l) {
                self.channel = channel;
                self.rssi = rssi;
            }
        }
        Some(associated)
    }

    /// SDK version from `AT+GMR` (`component_version_sdk_2.0.97`).
    pub async fn refresh_version(&mut self, control: &Control<'_>) {
        let Ok(out) = control.at("AT+GMR").await else {
            return;
        };
        for line in out.lines.iter() {
            if let Some(v) = line.as_str().strip_prefix("component_version_sdk_") {
                self.sdk.clear();
                let _ = self.sdk.push_str(&v[..v.len().min(15)]);
            }
        }
    }

    /// Pull one of the four web sources, round-robin by `idx`. A failure leaves
    /// the previous value in place and returns `false`.
    pub async fn refresh_source(&mut self, control: &Control<'_>, idx: usize) -> bool {
        let mut body = heapless::Vec::<u8, FETCH_MAX>::new();
        let (host, path, what) = match idx % 4 {
            0 => ("wttr.in", "/?format=3", "weather"),
            1 => ("api.open-notify.org", "/iss-now.json", "iss"),
            2 => ("api.open-notify.org", "/astros.json", "people"),
            _ => ("www.boredapi.com", "/api/activity", "activity"),
        };
        if let Err(why) = t01::http_fetch(control, CLIENT_LINK, host, path, &mut body).await {
            warn!("refresh {} ({}{}) failed: {}", what, host, path, why);
            return false;
        }
        match idx % 4 {
            0 => {
                if let Ok(text) = core::str::from_utf8(&body) {
                    set(&mut self.weather, text.trim());
                }
            }
            1 => {
                let lat = json_string(&body, b"latitude");
                let lon = json_string(&body, b"longitude");
                if let (Some(lat), Some(lon)) = (lat, lon) {
                    self.iss.clear();
                    let _ = write!(self.iss, "{}\u{b0}, {}\u{b0}", lat, lon);
                }
            }
            2 => {
                if let Some(n) = json_number(&body, b"number") {
                    self.people = n;
                }
            }
            _ => {
                if let Some(a) = json_string(&body, b"activity") {
                    set(&mut self.activity, a);
                }
            }
        }
        info!("refreshed {}", what);
        true
    }

    /// The `/api` document the page polls.
    pub fn api_json(&self, uptime_s: u64, requests: u32) -> heapless::String<900> {
        let mut j = heapless::String::new();
        let _ = write!(j, "{{\"unix\":{},\"weather\":", self.unix_now());
        json_str(&mut j, &self.weather);
        let _ = j.push_str(",\"iss\":");
        json_str(&mut j, &self.iss);
        let _ = write!(j, ",\"people\":{},\"activity\":", self.people);
        json_str(&mut j, &self.activity);
        let _ = write!(
            j,
            ",\"rssi\":{},\"channel\":{},\"online\":{},\"ip\":",
            self.rssi, self.channel, self.online
        );
        json_str(&mut j, &self.ip);
        let _ = j.push_str(",\"sdk\":");
        json_str(&mut j, &self.sdk);
        let _ = write!(j, ",\"up\":{},\"reqs\":{}}}", uptime_s, requests);
        j
    }
}

/// Replace `dst` with `src`, cut at a character boundary if it does not fit.
fn set<const N: usize>(dst: &mut heapless::String<N>, src: &str) {
    dst.clear();
    for ch in src.chars() {
        if dst.push(ch).is_err() {
            break;
        }
    }
}

/// Append `s` as a JSON string literal.
fn json_str<const N: usize, const M: usize>(
    out: &mut heapless::String<N>,
    s: &heapless::String<M>,
) {
    let _ = out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => {
                let _ = out.push_str("\\\"");
            }
            '\\' => {
                let _ = out.push_str("\\\\");
            }
            c if (c as u32) < 0x20 => {
                let _ = out.push(' ');
            }
            c => {
                let _ = out.push(c);
            }
        }
    }
    let _ = out.push('"');
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The string stored under `key` in a JSON body. Tolerates spaces after the
/// colon, which open-notify has (`"latitude": "50.15"`) and boredapi does not.
fn json_string<'a>(raw: &'a [u8], key: &[u8]) -> Option<&'a str> {
    let mut pat = heapless::Vec::<u8, 40>::new();
    pat.push(b'"').ok()?;
    pat.extend_from_slice(key).ok()?;
    pat.push(b'"').ok()?;
    let mut i = find_sub(raw, &pat)? + pat.len();
    while i < raw.len() && (raw[i] == b' ' || raw[i] == b':') {
        i += 1;
    }
    if raw.get(i) != Some(&b'"') {
        return None;
    }
    let rest = &raw[i + 1..];
    let end = rest.iter().position(|&b| b == b'"')?;
    core::str::from_utf8(&rest[..end]).ok()
}

/// The unsigned integer stored under `key` in a JSON body.
fn json_number(raw: &[u8], key: &[u8]) -> Option<u32> {
    let mut pat = heapless::Vec::<u8, 40>::new();
    pat.push(b'"').ok()?;
    pat.extend_from_slice(key).ok()?;
    pat.push(b'"').ok()?;
    let mut i = find_sub(raw, &pat)? + pat.len();
    while i < raw.len() && (raw[i] == b' ' || raw[i] == b':') {
        i += 1;
    }
    let digits = raw[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    core::str::from_utf8(&raw[i..i + digits]).ok()?.parse().ok()
}

/// `Thu Nov 19 01:44:01 2026` (the module's `+CIPSNTPTIME` text, UTC) to Unix
/// seconds.
pub fn parse_sntp(s: &str) -> Option<u64> {
    let mut parts = s.split_whitespace();
    let _weekday = parts.next()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let day: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':');
    let h: i64 = hms.next()?.parse().ok()?;
    let m: i64 = hms.next()?.parse().ok()?;
    let sec: i64 = hms.next()?.parse().ok()?;
    let year: i64 = parts.next()?.parse().ok()?;

    // Days since 1970-01-01 (Howard Hinnant's civil-from-days, inverted).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let unix = days * 86_400 + h * 3_600 + m * 60 + sec;
    u64::try_from(unix).ok()
}

/// `+CWJAP:"<ssid>","<bssid>",<channel>,<rssi>,...` to `(channel, rssi)`.
fn parse_cwjap(line: &str) -> Option<(u32, i32)> {
    let rest = line.strip_prefix("+CWJAP:")?;
    // Skip the two quoted fields (an SSID may itself contain commas).
    let mut quotes = 0;
    let mut after = None;
    for (i, ch) in rest.char_indices() {
        if ch == '"' {
            quotes += 1;
            if quotes == 4 {
                after = Some(i + 1);
                break;
            }
        }
    }
    let mut fields = rest[after?..].split(',').filter(|f| !f.is_empty());
    let channel = fields.next()?.trim().parse().ok()?;
    let rssi = fields.next()?.trim().parse().ok()?;
    Some((channel, rssi))
}
