#!/usr/bin/env bash
# Flash an ELF to the SF32LB52 application partition and tail the log UART.
#
# Used as the cargo runner, so `cargo run --release --bin openwalkietalkie` both
# programs the board and shows its output. Pass --no-monitor to just flash.
set -euo pipefail

elf="${1:-}"
[ -n "$elf" ] && shift || { echo "usage: $0 <ELF> [--no-monitor]" >&2; exit 64; }
[ -f "$elf" ] || { echo "ELF not found: $elf" >&2; exit 66; }

monitor=1
for a in "$@"; do [ "$a" = "--no-monitor" ] && monitor=0; done

port="${SIFLI_SFTOOL_PORT:-}"
if [ -z "$port" ]; then
    # Prefer the WCH vendor node; macOS exposes the CH343 twice (wchusbserial
    # via WCH's DEXT and usbmodem via Apple's CDC driver) and they are the same
    # physical chip, so taking the first match after sorting is fine.
    port=$(ls /dev/cu.*wchusbserial* /dev/cu.*usbserial* 2>/dev/null | head -1 || true)
    [ -n "$port" ] || port=$(ls /dev/cu.*usbmodem* 2>/dev/null | head -1 || true)
    [ -n "$port" ] || { echo "No USB serial device found; set SIFLI_SFTOOL_PORT." >&2; exit 65; }
    echo "[flash] port: $port"
fi

# There is one serial port and it carries both the log and the bootloader
# handshake, so a monitor left attached in another window silently eats the
# bytes sftool is waiting for. That failure is indistinguishable from a wedged
# board, so say so up front rather than let it look like bad hardware.
if command -v lsof >/dev/null && lsof "$port" >/dev/null 2>&1; then
    echo "[flash] WARNING: another process has $port open (a monitor?)." >&2
    echo "[flash]          Close it -- flashing will otherwise fail to connect." >&2
fi

# A custom baud left behind by tools/monitor.py makes sftool refuse the port
# with "Invalid argument", so put it back to something standard first.
stty -f "$port" 115200 2>/dev/null || true

# The firmware logs on the same UART the bootloader handshakes over, so a
# running image can talk over sftool's connect attempt and the flash fails with
# "timeout while waiting for RAM command response". It is not a wedged board --
# retrying reliably wins, because the reset lands in a quieter moment. (Holding
# the board in reset while flashing would avoid it entirely; upstream's UART
# examples carry the same warning.)
#
# write_flash takes the ELF directly and derives the load address from it, so
# there is no objcopy step. Only the app partition is touched -- the flash
# table and bootloader are never written, which is what keeps the board
# recoverable.
flashed=0
for attempt in 1 2 3 4 5; do
    if sftool -c SF32LB52 --port "$port" --connect-attempts 10 \
            write_flash --verify "$elf"; then
        flashed=1
        break
    fi
    echo "[flash] attempt $attempt failed, retrying" >&2
    sleep 1
done
[ "$flashed" = 1 ] || { echo "[flash] giving up after 5 attempts" >&2; exit 1; }

if [ "$monitor" = 1 ]; then
    # sftool has already reset the board into the new image, so the monitor
    # just attaches -- no reset pulse of its own, which would only reboot it a
    # second time and lose the banner.
    #
    # Not `stty` + `cat`: macOS termios cannot express 1000000 baud, so the
    # monitor sets the rate with an ioctl instead. See tools/monitor.py.
    exec "$(dirname "$0")/monitor.py" "$port"
fi
