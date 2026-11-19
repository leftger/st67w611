#![no_std]
#![no_main]

//! T01 proof of concept #1: does the module's own TCP/IP stack work?
//!
//! With the module on the T01 profile, everything below happens **inside the
//! module**; this MCU only sends AT commands:
//!
//! 1. firmware profile reads back as T01,
//! 2. join Wi-Fi and obtain an IPv4 address (DHCP, on the module),
//! 3. DNS lookup (`AT+CIPDOMAIN`),
//! 4. ICMP ping (`AT+PING`),
//! 5. SNTP time (`AT+CIPSNTPCFG` / `AT+CIPSNTPTIME?`),
//! 6. a plain HTTP GET over a module-side TCP socket (`AT+CIPSTART`/`CIPSEND`).
//!
//! Each check logs `PASS` or `FAIL`, and the run ends with a
//! `RESULT: ... n/m checks passed` line. Run it with `scripts/poc.sh
//! t01_net_check`.

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_time::Timer;

#[path = "../common.rs"]
mod common;
#[path = "../t01.rs"]
mod t01;

use st67w611::net::client::Net;

const HOST: &str = "example.com";
const PING_TARGET: &str = "1.1.1.1";

struct Tally {
    passed: u32,
    total: u32,
}

impl Tally {
    fn check(&mut self, name: &str, ok: bool) {
        self.total += 1;
        if ok {
            self.passed += 1;
            info!("PASS  {}", name);
        } else {
            warn!("FAIL  {}", name);
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("t01_net_check: exercising the module's TCP/IP stack");
    let control = common::start(spawner).await.control;
    let mut t = Tally {
        passed: 0,
        total: 0,
    };

    // 1. Profile.
    let profile = common::profile(control).await;
    t.check(
        "module runs the T01 profile (+CWNETMODE:1)",
        profile == common::Profile::T01,
    );
    if profile != common::Profile::T01 {
        warn!("this PoC needs T01; run scripts/ncp_switch.sh t01 first");
        info!("RESULT: FAILED ({}/{} checks passed)", t.passed, t.total);
        common::halt().await;
    }

    // 2. Wi-Fi + address.
    let ip = t01::join(control).await;
    t.check("joined Wi-Fi and got an IPv4 address", ip.is_some());
    let Some(ip) = ip else {
        warn!("no network: set WIFI_SSID / WIFI_PASS (scripts/poc.sh) or provision the module");
        info!("RESULT: FAILED ({}/{} checks passed)", t.passed, t.total);
        common::halt().await;
    };
    info!("module address: {}", ip.as_str());

    let net = Net::new(control);

    // 3. DNS.
    let dns_ok = match net.lookup(HOST).await {
        Ok(out) => {
            common::log_lines("AT+CIPDOMAIN", &out);
            out.value_after("+CIPDOMAIN:").is_some()
        }
        Err(e) => {
            warn!("  lookup failed: {:?}", e);
            false
        }
    };
    t.check("DNS lookup", dns_ok);

    // 4. Ping.
    let ping_ok = match control.ping(PING_TARGET).await {
        Ok(out) => {
            common::log_lines("AT+PING", &out);
            out.value_after("+PING:").is_some()
        }
        Err(e) => {
            warn!("  ping failed: {:?}", e);
            false
        }
    };
    t.check("ICMP ping", ping_ok);

    // 5. SNTP.
    let mut sntp_ok = false;
    match net.configure_sntp(true, 0, "pool.ntp.org").await {
        Ok(()) => {
            // The module reports 1970 until its first sync completes.
            for _ in 0..10 {
                Timer::after_secs(2).await;
                if let Ok(s) = net.sntp_time_string().await {
                    if !s.as_str().contains("1970") {
                        info!("  time: {}", s.as_str());
                        sntp_ok = true;
                        break;
                    }
                }
            }
            if !sntp_ok {
                warn!("  still at the 1970 epoch after 20 s");
            }
        }
        Err(e) => warn!("  sntp config failed: {:?}", e),
    }
    t.check("SNTP time sync", sntp_ok);

    // 6. TCP + HTTP.
    let mut status = heapless::String::<96>::new();
    let http_ok = if t01::setup_sockets(control).await {
        match t01::http_get(control, HOST, "/", &mut status).await {
            Ok(()) => {
                info!("  {}", status.as_str());
                true
            }
            Err(why) => {
                warn!("  http: {}", why);
                false
            }
        }
    } else {
        false
    };
    t.check("HTTP GET over a module-side TCP socket", http_ok);

    if t.passed == t.total {
        info!("RESULT: OK ({}/{} checks passed)", t.passed, t.total);
    } else {
        info!("RESULT: FAILED ({}/{} checks passed)", t.passed, t.total);
    }
    common::halt().await;
}
