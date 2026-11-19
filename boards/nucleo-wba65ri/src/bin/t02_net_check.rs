#![no_std]
#![no_main]

//! T02 proof of concept #1: does the host-side network stack work?
//!
//! The twin of `t01_net_check`. With the module on the T02 profile it is only a
//! Wi-Fi MAC/PHY: everything below runs **on this MCU** in `embassy-net`, over raw
//! Ethernet frames the module carries:
//!
//! 1. firmware profile reads back as T02,
//! 2. join Wi-Fi and obtain an IPv4 address (DHCP client in `embassy-net`),
//! 3. DNS lookup (host-side resolver),
//! 4. SNTP time (a UDP socket on this MCU),
//! 5. a plain HTTP GET over a host-side TCP socket.
//!
//! T01 also runs an ICMP ping. `embassy-net` has no ICMP client, so there is no
//! T02 equivalent and the totals differ by one (5 here, 6 on T01).
//!
//! Each check logs `PASS` or `FAIL`, and the run ends with a `RESULT:` line. Run
//! it with `scripts/poc.sh t02_net_check`.

use defmt::{info, warn};
use embassy_executor::Spawner;

#[path = "../common.rs"]
mod common;
#[path = "../t02.rs"]
mod t02;

const HOST: &str = "example.com";

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

    fn finish(&self) {
        if self.passed == self.total {
            info!("RESULT: OK ({}/{} checks passed)", self.passed, self.total);
        } else {
            info!(
                "RESULT: FAILED ({}/{} checks passed)",
                self.passed, self.total
            );
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("t02_net_check: exercising the host-side embassy-net stack");
    let started = common::start(spawner).await;
    let control = started.control;
    let mut t = Tally {
        passed: 0,
        total: 0,
    };

    // 1. Profile.
    let profile = common::profile(control).await;
    t.check(
        "module runs the T02 profile (+CWNETMODE:0)",
        profile == common::Profile::T02,
    );
    if profile != common::Profile::T02 {
        warn!("this PoC needs T02; run scripts/ncp_switch.sh t02 first");
        t.finish();
        common::halt().await;
    }

    // 2. Wi-Fi + DHCP.
    let up = t02::bring_up(spawner, started).await;
    t.check("joined Wi-Fi and got a DHCP address", up.is_some());
    let Some((stack, ip)) = up else {
        warn!("no network: set WIFI_SSID / WIFI_PASS (scripts/poc.sh) or provision the module");
        t.finish();
        common::halt().await;
    };
    info!("host address: {}", ip.as_str());

    // 3. DNS.
    let resolved = t02::resolve(stack, HOST).await;
    if let Some(addr) = resolved {
        info!("  {} -> {}", HOST, addr);
    }
    t.check("DNS lookup", resolved.is_some());

    // 4. SNTP.
    let unix = t02::sntp(stack, "pool.ntp.org").await;
    if let Some(secs) = unix {
        info!("  unix time: {}", secs);
    }
    t.check("SNTP time sync", unix.is_some());

    // 5. TCP + HTTP.
    let mut status = heapless::String::<96>::new();
    let http_ok = match t02::http_get(stack, HOST, "/", &mut status).await {
        Ok(()) => {
            info!("  {}", status.as_str());
            true
        }
        Err(why) => {
            warn!("  http: {}", why);
            false
        }
    };
    t.check("HTTP GET over a host-side TCP socket", http_ok);

    t.finish();
    common::halt().await;
}
