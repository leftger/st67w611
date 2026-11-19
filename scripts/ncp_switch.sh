#!/usr/bin/env bash
#
# Switch the ST67W611's mission profile between T01 (lwIP on the module) and T02
# (lwIP on the host), using the NUCLEO-WBA65RI that is wired to it.
#
# How: the module is updated through its own AT+OTA* commands. This script builds
# boards/nucleo-wba65ri/src/bin/ncp_switch.rs with ST's `.ota` image for the target
# profile embedded, flashes it to the Nucleo with probe-rs, and watches the board's
# log for the verdict. No UART bootloader, no QConn_Flash, and no eFuse locking,
# which ST's own NCP_update_mission_profile_t0X.sh does.
#
# Usage:
#   scripts/ncp_switch.sh <t01|t02> [--image PATH | --version 2.0.97|2.0.89] [--yes] [--dry-run]
#
#   t01 | t02        the profile to install
#   --image PATH     the st67w611m_mission_tXX_vX.Y.Z.bin.ota file to install
#   --version V      pick that version out of $X_CUBE_ST67W61 (default 2.0.97)
#   --yes            skip the confirmation prompt
#   --dry-run        run every check and build, but do not flash
#
# Environment:
#   X_CUBE_ST67W61   path to the unpacked ST X-CUBE-ST67W61 package (needed
#                    unless --image is given)
#   TIMEOUT_SECS     how long to wait for the verdict (default 300)
#
# If the module already runs the target profile, nothing is sent. The board
# firmware must match the module afterwards: the examples are named t01_* / t02_*
# for the profile they need (see scripts/poc.sh).

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
board="$repo/boards/nucleo-wba65ri"
chip="STM32WBA65RI"
bin="ncp_switch"
elf="$board/target/thumbv8m.main-none-eabihf/release/$bin"
timeout_secs="${TIMEOUT_SECS:-300}"

target="" image="" version="2.0.97" assume_yes=0 dry_run=0
while [ $# -gt 0 ]; do
  case "$1" in
    t01|t02)   target="$1"; shift ;;
    --image)   image="${2:?--image needs a path}"; shift 2 ;;
    --version) version="${2:?--version needs a value}"; shift 2 ;;
    --yes|-y)  assume_yes=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }
[ -n "$target" ] || die "say which profile to install: $0 <t01|t02> (try --help)"
other="t02"; [ "$target" = "t02" ] && other="t01"
# (macOS ships bash 3.2, which has no ${var^^}.)
TARGET="$(echo "$target" | tr '[:lower:]' '[:upper:]')"
OTHER="$(echo "$other" | tr '[:lower:]' '[:upper:]')"

# --- 1. the image ---------------------------------------------------------
if [ -z "$image" ]; then
  [ -n "${X_CUBE_ST67W61:-}" ] || die "set X_CUBE_ST67W61 to the ST package, or pass --image PATH"
  image="$X_CUBE_ST67W61/Projects/ST67W6X_Scripts/Binaries/NCP_Binaries/st67w611m_mission_${target}_v${version}.bin.ota"
fi
[ -f "$image" ] || die "image not found: $image"
case "$(basename "$image")" in
  *mission_"$target"*.bin.ota) ;;
  *mission_"$other"*) die "that is the $OTHER image; you asked for $TARGET" ;;
  *) die "expected a st67w611m_mission_${target}_*.bin.ota file, got $(basename "$image")" ;;
esac
size=$(wc -c <"$image" | tr -d ' ')
[ "$size" -gt 500000 ] || die "image is only $size bytes; that does not look like a full NCP image"
# The Nucleo has 2 MB of flash and the image rides along inside the firmware.
[ "$size" -lt 1800000 ] || die "image is $size bytes; it will not fit next to the tool in 2 MB of flash"
image="$(cd "$(dirname "$image")" && pwd)/$(basename "$image")"
echo "target: $TARGET"
echo "image:  $image"
echo "size:   $size bytes"
echo "sha256: $(shasum -a 256 "$image" | cut -d' ' -f1)"

# --- 2. the tools and the board ------------------------------------------
command -v probe-rs >/dev/null || die "probe-rs not found (https://probe.rs)"
command -v cargo    >/dev/null || die "cargo not found"
if [ "$dry_run" -eq 0 ]; then
  probes="$(probe-rs list 2>&1 || true)"
  if echo "$probes" | grep -qi "no debug probes"; then
    die "probe-rs sees no debug probe. Plug in the Nucleo's ST-LINK USB port, then retry."
  fi
  echo "probe:  $(echo "$probes" | sed -n 's/^\[0\]: //p' | head -1)"
fi

# --- 3. confirm ----------------------------------------------------------
if [ "$dry_run" -eq 0 ] && [ "$assume_yes" -eq 0 ]; then
  cat <<EOF

This will reprogram the ST67W611 module's firmware to $TARGET.
  - The Nucleo's own flash is overwritten with the update tool.
  - The module reboots into the new image when the transfer finishes.
  - Do not unplug the board or the shield until the verdict appears.
EOF
  read -r -p "Proceed? [y/N] " reply
  case "$reply" in y|Y|yes|YES) ;; *) echo "aborted."; exit 1 ;; esac
fi

# --- 4. build ------------------------------------------------------------
# DEFMT_LOG=info keeps the log fast over RTT during the ~1400-chunk transfer.
echo
echo "building $bin ..."
( cd "$board" && DEFMT_LOG="${DEFMT_LOG:-info}" ST67_OTA_IMAGE="$image" \
    cargo build --release --bin "$bin" )
[ -f "$elf" ] || die "build produced no $elf"

if [ "$dry_run" -eq 1 ]; then
  echo "dry run: built OK, not flashing."
  exit 0
fi

# --- 5. flash, run, and watch for the verdict ----------------------------
logdir="$(mktemp -d)"
log="$logdir/probe-rs.log"
echo
echo "flashing and running (log: $log) ..."
probe-rs run --chip "$chip" "$elf" >"$log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; sleep 1; kill -9 "$pid" 2>/dev/null || true' EXIT

shown=0 waited=0 verdict=""
while [ "$waited" -lt "$timeout_secs" ]; do
  # Stream new log lines to the terminal.
  total=$(wc -l <"$log" | tr -d ' ')
  if [ "$total" -gt "$shown" ]; then
    sed -n "$((shown + 1)),${total}p" "$log"
    shown=$total
  fi
  verdict="$(grep -o 'RESULT: .*' "$log" | tail -1 || true)"
  [ -n "$verdict" ] && break
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "probe-rs exited before a verdict was reported (see $log)" >&2
    exit 1
  fi
  sleep 1
  waited=$((waited + 1))
done

echo
if [ -z "$verdict" ]; then
  echo "TIMEOUT: no verdict after ${timeout_secs}s. Full log: $log" >&2
  exit 1
fi
echo "$verdict"
case "$verdict" in
  "RESULT: OK"*)
    echo "The module now runs the $TARGET profile."
    echo "Next: flash a $TARGET example (scripts/poc.sh ${target}_net_check); the $OTHER examples will not work." ;;
  "RESULT: UNVERIFIED"*)
    echo "The module rebooted but its profile could not be read back."
    echo "Confirm with: scripts/poc.sh ${target}_net_check"
    exit 3 ;;
  *)
    echo "Update failed. Log: $log" >&2
    echo "If the module no longer responds, recover it with ST's UART bootloader route" >&2
    echo "(examples/stm32wba55_firmware_programmer.rs + QConn_Flash)." >&2
    exit 1 ;;
esac
