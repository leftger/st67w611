# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/) (while `0.x`, a minor bump may
contain breaking changes).

## [Unreleased]

Everything since 0.1.0, the first crates.io release (2026-01-27). Not yet
published: see "Release status" at the end.

### Added

- **T01 and T02 behind one typed API.** The typed `WiFi`, `Net`, `Mqtt`, `Fwu`
  and BLE clients are generic over `AtTransport`, so the same code runs on both
  module firmwares. (The HTTP client and power management remain T01-only.)
  Select a firmware with the `mission-t01` (default) or `mission-t02` feature.
- **T02 support** (`net::xarxa`): a raw-L2 driver that plugs the module into the
  xarxa-based `embassy-net`, so the host runs the whole TCP/IP stack (IPv4/IPv6,
  DHCP, DNS, SLAAC). `tls` adds `embedded-tls` on top, on either firmware.
- **Full-duplex SPI engine** following ST's reference host (`bus::engine`,
  `bus::frame`), including the RDY flow-control handshake.
- **`Control` additions** (`net::xarxa`):
  - `ping()`: waits for the module's unsolicited `+PING:<ms>ms` result.
  - `recv()`: pulls buffered socket data (`AT+CIPRECVDATA`) as raw bytes.
  - `socket_event()`: delivers the module's own `+CIP:` / `+IPD:` events, which a
    TCP server needs to learn that a client connected.
  - `set_link()`: raises the link when the module re-joined a stored network.
- **Over-the-air profile switching** between T01 and T02 in either direction
  through `AT+OTA*` (`fwu::Fwu`), with the board tool `ncp_switch`.
- **Board crates** with working demos: `boards/nucleo-wba65ri` (matching
  `t01_*` / `t02_*` network self-checks and web servers, a T02 dashboard, a USB
  provisioning portal, and diagnostics) and `boards/stm32n6570-dk`.
- `scripts/ncp_switch.sh` and `scripts/poc.sh` to switch firmware and run the
  demos; `CHANGELOG.md`; documented AT-transport behaviour in `ARCHITECTURE.md`.

### Changed

- **Breaking: dependency versions.** `embassy-executor` 0.9 → 0.10,
  `embassy-sync` 0.7 → 0.8, `embedded-io` / `embedded-io-async` 0.6 → 0.7.
- **Breaking: the 0.1.0 `embassy-net-driver` (packet-token) integration is gone**,
  replaced by `net::xarxa`, because `embassy-net` 0.9 no longer uses that trait.
- **DNS lookups use IPv4-only resolution by default.** `Net::lookup` and
  `Net::resolve` now send `AT+CIPDOMAIN` with type `2`. The module answers `ERROR`
  to type `0`, which the previous default sent, so lookups failed outright.
- The board crate no longer needs a git-ignored `secrets.rs`: fallback Wi-Fi
  credentials come from the `WIFI_SSID` / `WIFI_PASS` build environment.
- **Linker arguments for the examples now come from a root `build.rs`**
  (`cargo:rustc-link-arg-examples`), not from a local `.cargo/config.toml`. The old
  setup leaked into `boards/*`, whose own `build.rs` adds the same arguments, so
  the linker reported `FLASH` defined twice and the scripts had to override
  `RUSTFLAGS`. They no longer do, and a fresh clone builds the examples with no
  local config.
- The whole tree is `rustfmt`-clean, and `clippy` is clean with `-D warnings` for
  every feature set, the examples and the tests. Fixes: `Default` impls for
  `ResponseSlot`, `ResponseSlotPool`, `AtProcessor` and `Socket`; inline `const`
  array repeats instead of named interior-mutable constants; `strip_prefix` in
  the URL parser; the HTTP client's "loop" that always exited on its first pass is
  now the single read it always was (behaviour unchanged, limitation documented).
- `boards/stm32n6570-dk/.cargo/config.toml`: the `[target.thumbv8m.main-none-eabihf]`
  key is now quoted. Unquoted, cargo parsed it as nested tables and silently
  ignored that board's `rustflags` and `runner`.
- Documentation cleanup: broken intra-doc links fixed (docs build with
  `-D warnings` for every feature set); the README's T02 quick start now shows
  the current API; the Cargo manifest gained `readme`, `rust-version` (1.90) and
  `exclude`.

### Fixed

- **`xarxa` AT transactions with a payload** (`AT+CIPSEND`, `AT+OTASEND`,
  `AT+BLEGATTSNTFY`): the module answers `OK` then `>`, often in one frame, and
  for `AT+CIPSEND` the `>` has no trailing newline. The runner treated the first
  `OK` as the end of the command, so the payload was never sent and the module
  swallowed the next command as data. Payload commands now stay open until the
  payload is sent and acknowledged (`Recv <n> bytes` / `SEND OK`), and an unsent
  payload reports a timeout instead of success.
- `AT+PING` results, which arrive after the command's `OK`, are now collected.
- `AT+CIPRECVDATA` replies are captured by their length prefix, so socket data is
  binary-safe and not limited by the 16-line response cap.
- Root examples `stm32wba55_t01_wifi_scan` / `_wifi_test` build again with the
  current `embassy-executor` `spawn` API, and the build is warning-free; a dead
  branch in the board's `thermal_probe` was removed.

### Removed

- The root example `stm32wba55_t02_embassy_net`, which targeted an API that no
  longer exists. Its replacements are the `t02_*` demos in `boards/nucleo-wba65ri`.

### Known issues

- `boards/stm32n6570-dk` does not build: it pins an older embassy revision than
  the root crate (so dependency resolution fails), and its code predates the
  current `embassy-stm32` EXTI API and still declares the removed `secrets` module.
  This predates the changes above (confirmed against a pristine `HEAD`). Porting it
  needs the board attached to verify.

### Release status

`cargo publish` is blocked until the xarxa-based `embassy-net` is released: the
`mission-t02` feature depends on it through a git revision, and the released
`embassy-net` 0.9.1 (smoltcp-based) lacks the features it uses. The T01 library
builds and passes its tests against released `embassy-*` crates only. See the
comments in `Cargo.toml`.

## [0.1.0] - 2026-01-27

First release: async Embassy driver for the ST67W611 over SPI with AT commands.

[Unreleased]: https://github.com/leftger/st67w611/compare/c0d5465...HEAD
[0.1.0]: https://github.com/leftger/st67w611/commit/c0d5465
