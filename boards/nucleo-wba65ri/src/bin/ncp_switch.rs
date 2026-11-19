#![no_std]
#![no_main]

//! Switch the ST67W611 between its two mission profiles, in either direction:
//! **T01** (TCP/IP on the module) and **T02** (TCP/IP on the host).
//!
//! The module is updated over its own `AT+OTA*` commands, driven from this board
//! over SPI. Run it through `scripts/ncp_switch.sh <t01|t02>`, which checks the
//! image, builds this with `ST67_OTA_IMAGE` pointing at ST's
//! `st67w611m_mission_tXX_vX.Y.Z.bin.ota`, flashes it and watches for the
//! `RESULT:` line below. The image decides the direction: a `mission_t01` image
//! installs T01, a `mission_t02` image installs T02.
//!
//! The sequence mirrors ST's reference host (`fota.c` / `w6x_sys.c`):
//!
//! 1. `AT+OTASTART=0` clears any session left over from an earlier attempt,
//! 2. `AT+OTASTART=1` opens a new one,
//! 3. the 512-byte FWU header goes out as an `AT+OTASEND` of its own, then the
//!    body in 1024-byte chunks,
//! 4. `AT+OTAFIN` makes the module verify the image and reboot into it.
//!
//! Nothing is retried mid-stream: after a failed chunk the module's view of the
//! stream is unknown, so the only safe moves are to abort the session and start
//! again from the top. If the module already runs the target profile, nothing is
//! sent.
//!
//! The AT link is the same SPI link in both profiles, so this talks to the module
//! through the xarxa transport whichever profile it starts in.

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

#[path = "../common.rs"]
mod common;

mod image {
    include!(concat!(env!("OUT_DIR"), "/ota_image.rs"));
}

use common::{halt, log_lines, profile, wait_responsive, Profile};
use st67w611::fwu::Fwu;

/// Size of the FWU header at the start of every `.ota` file.
const FWU_HEADER_SIZE: usize = 512;

/// Bytes per body `AT+OTASEND`: a multiple of ST's 256-byte
/// `FWU_SECTOR_ALIGNMENT`, and well under the SPI frame limit.
const CHUNK: usize = 1024;

/// How long the module may take to come back after `AT+OTAFIN` (ST waits 30 s).
const REBOOT_TIMEOUT: Duration = Duration::from_secs(45);

async fn log_version(control: &st67w611::net::xarxa::Control<'_>) {
    match control.at("AT+GMR").await {
        Ok(out) => log_lines("AT+GMR", &out),
        Err(e) => warn!("AT+GMR failed: {:?}", e),
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("ncp_switch: ST67W611 over-the-air profile switch");

    let want = match image::TARGET {
        "t01" => Profile::T01,
        "t02" => Profile::T02,
        _ => {
            error!("no image embedded: build with ST67_OTA_IMAGE=<…mission_t01|t02….bin.ota> (use scripts/ncp_switch.sh)");
            info!("RESULT: FAILED (no image)");
            halt().await;
        }
    };
    info!(
        "image: {} ({} bytes) -> {}",
        image::IMAGE_NAME,
        image::IMAGE.len(),
        want
    );

    let control = common::start(spawner).await.control;

    info!("step: current firmware");
    log_version(control).await;
    let before = profile(control).await;
    if before == want {
        info!("module already reports {}; nothing to do", want);
        info!("RESULT: OK (already {})", want);
        halt().await;
    }
    if before == Profile::Unknown {
        // Not fatal: the image itself is what decides the result. But say so.
        warn!("could not tell which profile the module runs; continuing anyway");
    } else {
        info!("module reports {}, switching to {}", before, want);
    }

    let fwu = Fwu::new(control);

    // Clear any half-finished session; an error here just means there was none.
    let _ = fwu.abort().await;
    Timer::after_millis(200).await;

    info!("step: AT+OTASTART=1");
    if let Err(e) = fwu.start().await {
        error!("module refused to start an update session: {:?}", e);
        info!("RESULT: FAILED (OTASTART)");
        halt().await;
    }

    // ST's host sends the 512-byte FWU header as an OTASEND of its own and only
    // then the body (`fota.c`, FWU_HEADER_SIZE); the module parses the header on
    // that first transfer, so header and body must not share a chunk.
    let total = image::IMAGE.len();
    if total <= FWU_HEADER_SIZE {
        error!(
            "image is smaller than the {}-byte FWU header",
            FWU_HEADER_SIZE
        );
        info!("RESULT: FAILED (image too small)");
        halt().await;
    }
    let (header, body) = image::IMAGE.split_at(FWU_HEADER_SIZE);

    info!("step: sending the {}-byte FWU header", header.len());
    if let Err(e) = fwu.send(header).await {
        error!(
            "module rejected the FWU header: {:?}; aborting the session",
            e
        );
        let _ = fwu.abort().await;
        info!("RESULT: FAILED (FWU header)");
        halt().await;
    }

    info!(
        "step: streaming {} body bytes in {}-byte chunks",
        body.len(),
        CHUNK
    );
    let mut sent = FWU_HEADER_SIZE;
    let mut next_report = 0usize;
    for chunk in body.chunks(CHUNK) {
        if let Err(e) = fwu.send(chunk).await {
            error!(
                "chunk at offset {} failed: {:?}; aborting the session",
                sent, e
            );
            let _ = fwu.abort().await;
            info!("RESULT: FAILED (OTASEND at {})", sent);
            halt().await;
        }
        sent += chunk.len();
        if sent >= next_report {
            info!("  {}% ({}/{} bytes)", sent * 100 / total, sent, total);
            next_report = sent + total / 10;
        }
    }
    info!("  100% ({}/{} bytes)", sent, total);

    info!("step: AT+OTAFIN (module verifies the image, then reboots)");
    if let Err(e) = fwu.finish().await {
        // The module may reset before it manages to answer, so a timeout here is
        // not proof of failure. Fall through and check what comes back.
        warn!(
            "AT+OTAFIN did not report OK ({:?}); checking the module anyway",
            e
        );
    }

    info!("step: waiting for the module to reboot ...");
    if !wait_responsive(control, REBOOT_TIMEOUT).await {
        error!("module did not come back after the update");
        info!("RESULT: FAILED (no reboot)");
        halt().await;
    }

    info!("step: firmware after update");
    log_version(control).await;
    let after = profile(control).await;
    if after == want {
        info!("RESULT: OK (module now reports {})", want);
    } else if after == Profile::Unknown {
        info!("RESULT: UNVERIFIED (module rebooted but its profile is unreadable; check with a {} example)", want);
    } else {
        info!("RESULT: FAILED (module still reports {})", after);
    }
    halt().await;
}
