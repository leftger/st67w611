//! The little website both web-server demos serve, so the T01 and T02 versions
//! return identical pages and differ only in who runs the TCP/IP stack.
//!
//! Included as `#[path = "../site.rs"] mod site;`.

#![allow(dead_code)]

use core::fmt::Write as _;

/// What the pages report.
pub struct Facts<'a> {
    /// `"T01"` or `"T02"`.
    pub profile: &'a str,
    /// One sentence on who is doing the TCP/IP work.
    pub who: &'a str,
    pub ip: &'a str,
    pub uptime_s: u64,
    pub requests: u32,
}

/// A rendered response.
pub struct Page {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: heapless::String<900>,
}

/// Pick the page for `path`: `/` (HTML status page), `/json`, or a 404.
pub fn route(path: &str, f: &Facts) -> Page {
    let mut body = heapless::String::new();
    match path {
        "/" | "/index.html" => {
            let _ = write!(
                body,
                "<!doctype html><meta charset=utf-8>\
                 <meta name=viewport content='width=device-width,initial-scale=1'>\
                 <title>ST67W611 ({p})</title>\
                 <style>body{{font:16px system-ui;margin:2rem auto;max-width:34rem;padding:0 1rem}}\
                 td{{padding:.25rem 1rem .25rem 0}}code{{background:#eee;padding:.1rem .3rem}}</style>\
                 <h1>Hello from the ST67W611</h1>\
                 <p>{who} (<b>{p}</b> firmware).</p>\
                 <table><tr><td>Address</td><td><code>{ip}</code></td></tr>\
                 <tr><td>Uptime</td><td>{up} s</td></tr>\
                 <tr><td>Requests served</td><td>{n}</td></tr></table>\
                 <p><a href=/json>/json</a></p>",
                p = f.profile,
                who = f.who,
                ip = f.ip,
                up = f.uptime_s,
                n = f.requests
            );
            Page {
                status: "200 OK",
                content_type: "text/html; charset=utf-8",
                body,
            }
        }
        "/json" => {
            let _ = write!(
                body,
                "{{\"firmware\":\"{}\",\"ip\":\"{}\",\"uptime_s\":{},\"requests\":{}}}",
                f.profile, f.ip, f.uptime_s, f.requests
            );
            Page {
                status: "200 OK",
                content_type: "application/json",
                body,
            }
        }
        _ => {
            let _ = body.push_str("not found\n");
            Page {
                status: "404 Not Found",
                content_type: "text/plain",
                body,
            }
        }
    }
}

/// The path from the first `GET <path> HTTP/1.x` found anywhere in `text`.
pub fn request_path(text: &str) -> Option<heapless::String<64>> {
    let i = text.find("GET ")?;
    let path = text[i + 4..].split(' ').next().unwrap_or("/");
    let mut s = heapless::String::new();
    let _ = s.push_str(&path[..path.len().min(63)]);
    Some(s)
}
