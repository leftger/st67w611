#!/usr/bin/env bash
#
# Build, flash and run a proof-of-concept on the NUCLEO-WBA65RI, and stream its
# log. There is a T01 and a T02 version of each, and they need the module on the
# matching profile (scripts/ncp_switch.sh t01|t02).
#
# Usage:
#   WIFI_SSID=<network> WIFI_PASS=<password> scripts/poc.sh <bin> [--hold SECS]
#
#   t01_net_check   TCP/IP lives on the MODULE. Join Wi-Fi, then DNS / ping / SNTP /
#                   HTTP GET through AT commands; ends with a RESULT line.
#   t02_net_check   TCP/IP lives on the MCU (embassy-net). The same checks over raw
#                   frames, minus ping (no ICMP client); ends with a RESULT line.
#   t01_web_server  host a website from the module's own stack.
#   t02_web_server  host the same website from the MCU's stack (embassy-net).
#                   Both print the URL to browse to and keep serving until Ctrl-C
#                   (or --hold SECS elapses).
#
#   --hold SECS     web servers only: stop after SECS seconds instead of running
#                   until Ctrl-C.
#
# WIFI_SSID / WIFI_PASS are baked into the firmware at build time. If WIFI_SSID is
# unset, the bins rely on whatever network the module has stored. The password is
# never logged, but it does end up in the flash of the Nucleo.

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
board="$repo/boards/nucleo-wba65ri"
chip="STM32WBA65RI"
timeout_secs="${TIMEOUT_SECS:-180}"

bin="" hold=0
while [ $# -gt 0 ]; do
  case "$1" in
    --hold)    hold="${2:?--hold needs seconds}"; shift 2 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    t01_net_check|t01_web_server|t02_net_check|t02_web_server) bin="$1"; shift ;;
    *) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
  esac
done
[ -n "$bin" ] || { echo "usage: $0 <t01|t02>_<net_check|web_server> [--hold SECS]" >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
command -v probe-rs >/dev/null || die "probe-rs not found (https://probe.rs)"
command -v cargo    >/dev/null || die "cargo not found"
probe-rs list 2>&1 | grep -qi "no debug probes" && die "probe-rs sees no debug probe; plug in the Nucleo's ST-LINK USB port"

if [ -n "${WIFI_SSID:-}" ]; then
  echo "wifi:   building in network \"$WIFI_SSID\""
else
  echo "wifi:   WIFI_SSID not set; the module's stored network will be used"
fi

# DEFMT_LOG=info keeps the RTT log fast; override it for more detail.
echo "building $bin ..."
( cd "$board" && DEFMT_LOG="${DEFMT_LOG:-info}" cargo build --release --bin "$bin" )
elf="$board/target/thumbv8m.main-none-eabihf/release/$bin"
[ -f "$elf" ] || die "build produced no $elf"

log="$(mktemp -d)/probe-rs.log"
echo "flashing and running (log: $log) ..."
probe-rs run --chip "$chip" "$elf" >"$log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; sleep 1; kill -9 "$pid" 2>/dev/null || true' EXIT INT TERM

shown=0 waited=0 verdict="" held=0
while true; do
  total=$(wc -l <"$log" | tr -d ' ')
  if [ "$total" -gt "$shown" ]; then
    sed -n "$((shown + 1)),${total}p" "$log"
    shown=$total
  fi
  [ -z "$verdict" ] && verdict="$(grep -o 'RESULT: .*' "$log" | tail -1 || true)"

  if ! kill -0 "$pid" 2>/dev/null; then
    [ -n "$verdict" ] || { echo "probe-rs exited before a verdict (see $log)" >&2; exit 1; }
    break
  fi

  if [ -z "$verdict" ]; then
    waited=$((waited + 1))
    [ "$waited" -lt "$timeout_secs" ] || { echo "TIMEOUT: no verdict after ${timeout_secs}s. Log: $log" >&2; exit 1; }
  elif [ "${bin#*_}" = "net_check" ]; then
    break
  elif [ "$verdict" != "RESULT: OK (serving)" ]; then
    break
  elif [ "$hold" -gt 0 ]; then
    held=$((held + 1))
    [ "$held" -lt "$hold" ] || break
  fi
  sleep 1
done

echo
echo "$verdict"
case "$verdict" in
  "RESULT: OK"*) exit 0 ;;
  *) echo "Log: $log" >&2; exit 1 ;;
esac
