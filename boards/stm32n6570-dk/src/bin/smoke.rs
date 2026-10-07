#![no_std]
#![no_main]

//! Milestone A0 — smoke test.
//!
//! Deliberately does *nothing*: no clock init, no embassy, no peripherals. All
//! it proves is that the image runs from RAM and that defmt/RTT works.
//!
//! * If this prints → the CPU/RAM/RTT path is fine and the hang is in
//!   `embassy_stm32::init` (N6 clock tree).
//! * If this is silent → the problem is below us: how probe-rs releases the N6
//!   from reset, or SRAM not being usable without the boot ROM/FSBL step.

use cortex_m_rt::entry;
use defmt::info;
// Pulls in the device crate, which emits the interrupt vector table that
// cortex-m-rt requires (otherwise linking fails with "interrupt vectors are
// missing").
use embassy_stm32 as _;
use {defmt_rtt as _, panic_probe as _};

/// panic-probe 1.0 no longer registers `_defmt_panic` for you.
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf()
}

#[entry]
fn main() -> ! {
    info!("ALIVE: CPU running from RAM, RTT works, no clock init done");

    let mut n: u32 = 0;
    loop {
        n = n.wrapping_add(1);
        if n % 1_000_000 == 0 {
            info!("tick {}", n);
        }
    }
}
