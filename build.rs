//! Linker arguments for the examples.
//!
//! The examples are Cortex-M firmware and need `cortex-m-rt`'s `link.x` and
//! defmt's `defmt.x`. Emitting them here (for examples only) keeps the repo
//! self-contained: no local `.cargo/config.toml` has to carry them, and none can
//! leak into the board crates under `boards/`, which add the same arguments from
//! their own `build.rs` (a duplicate makes the linker report `FLASH` defined
//! twice).
//!
//! The library itself, and `cargo test` on the host, are unaffected.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg-examples=--nmagic");
    println!("cargo:rustc-link-arg-examples=-Tlink.x");
    println!("cargo:rustc-link-arg-examples=-Tdefmt.x");
}
