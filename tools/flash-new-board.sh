#!/usr/bin/env bash
# Provision a blank Rev A (BodyV6.13 r6) or Rev B main board over its UART
# pads: the flash table, the USB-recovery bootloader and the application in
# one sftool run, then read the boot log back and check the board came up.
#
#   tools/flash-new-board.sh PORT [ELF]
#
# Rev B's pads are the same three (the jig in hardware/rev-b/jig lands on
# them). The ELF names its board and the script says which it is writing;
# build the application for the board in front of you (hw-a or hw-b).
#
# PORT is the 3.3 V UART adapter (a DAPLink's serial side works) wired
# adapter TX -> TP6, adapter RX -> TP8, GND -> TP4. Leave TP5 alone: it is
# the active-high long reset. Power the board from its USB-C. No reset or
# power-cycle timing is involved: a blank chip waits in the boot ROM's
# download monitor, and a programmed one answers on its debug port while it
# runs (after a soft power-off, wake it with CANCEL or PTT first).
#
# Build the three images first:
#
#   source target/soc-build-env.sh
#   OWT_BOARD=hw-b OWT_BOOT_IMAGES=1 tools/build-native.sh   # flash table + bootloader (hw-a for Rev A)
#   cargo build --release --features hw-b                    # the application (hw-a for Rev A)
#
# After this the board updates over its own USB-C (tools/flash-usb.py); only
# the flash table and the bootloader stay UART-only. The boot log is kept in
# target/flash-logs/.
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
port=${1:-}
if [ -z "$port" ] || [ "${port#-}" != "$port" ]; then
    sed -n '2,23s/^# \{0,1\}//p' "$0"
    exit 2
fi
elf=${2:-$root/target/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw}
build=$root/native/project/build_openwalkietalkie_hcpu
ftab=$build/ftab.bin
boot=$build/bootloader/output/bootloader.bin
python=/usr/bin/python3

fail() {
    echo "flash-new-board: $*" >&2
    exit 1
}

[ -e "$port" ] || fail "no $port (ls /dev/cu.usbmodem* to find the adapter)"
command -v sftool >/dev/null || fail "sftool is not on PATH"
for image in "$ftab" "$boot" "$elf"; do
    [ -s "$image" ] || fail "missing $image (see the build lines in this script's header)"
done
# The images must be the ones this board needs: the flash table's magic, a
# bootloader that runs from SRAM, and an application built with
# `--features hw-a` or `hw-b` (the default build is the LilyGo's pin map: it
# leaves the panel rails off and drives the I2C data line as a power hold).
# Every board's ELF shares one path, so this is checked every time, and the
# board the application names is printed: a Rev A image on a Rev B board (or
# the reverse) boots but drives the wrong amplifier pin.
board=$("$python" - "$ftab" "$boot" "$elf" <<'EOF'
import sys
ftab, boot, elf = (open(p, "rb").read() for p in sys.argv[1:4])
problems = []
if ftab[:4] != b"FCES":
    problems.append("the flash table does not start with its FCES magic")
if not (0x20000000 <= int.from_bytes(boot[:4], "little") <= 0x20080000):
    problems.append("the bootloader's stack pointer is not in SRAM")
boards = [name for name in ("Rev A", "Rev B") if b"OpenWalkieTalkie " + name.encode() + b" (" in elf]
if len(boards) != 1:
    problems.append("the application is not a Rev A or Rev B build (cargo build --release --features hw-a|hw-b)")
for p in problems:
    print("flash-new-board: " + p, file=sys.stderr)
if problems:
    sys.exit(1)
print(boards[0])
EOF
) || exit 1
echo "flash-new-board: the application is a $board image"

echo "flash-new-board: writing the flash table, the bootloader and $(basename "$elf") over $port"
started=$(date +%s)
write() {
    "$python" "$root/tools/flash-uart.py" "$port" write_flash --verify \
        "$ftab@0x12000000" "$boot@0x12010000" "$elf"
}
# The first attempt on a fresh board can time out waiting for sftool's RAM
# stub to answer (seen on the second r6, 2026-10-04); the next one goes
# through, so one retry is part of the procedure rather than a reason to stop.
if ! write; then
    echo "flash-new-board: that attempt failed; trying once more"
    write || fail "the write failed twice (see sftool's output above)"
fi
echo "flash-new-board: written and verified in $(($(date +%s) - started)) s; reading the boot log"

logs=$root/target/flash-logs
mkdir -p "$logs"
log=$logs/boot-$(date +%Y%m%d-%H%M%S).log
"$python" "$root/tools/monitor.py" "$port" --seconds 12 >"$log" 2>&1

failures=0
check() { # required pattern, what it shows
    if grep -aq -- "$1" "$log"; then
        printf '  ok    %s\n' "$2"
    else
        printf '  FAIL  %s (no "%s")\n' "$2" "$1"
        failures=$((failures + 1))
    fi
}
# The ROM's own SFBL banner comes before the log is reopened and the
# adapter does not always keep it; the bootloader's banner proves the same.
check "OWTB: OpenWalkieTalkie bootloader" "the ROM started the USB-recovery bootloader"
check "OWTB: booting the application" "it accepted the application"
check "board: OpenWalkieTalkie $board" "the $board application runs"
check "display: CO5300 initialised" "the panel driver came up (it writes blind; look at the glass)"
check "usb: enabled" "the USB device is up"
check "link: advertising as OpenWalkieTalkie" "Bluetooth is advertising"
if grep -aq "levels 0xFF" "$log"; then
    echo "  ok    every button and the wheel read released"
elif grep -aq "inputs: six buttons" "$log"; then
    echo "  WARN  an input reads pressed at boot: $(grep -a 'inputs: six buttons' "$log" | head -1)"
fi
if grep -aq "touch: information mode failed" "$log"; then
    echo "  WARN  the touch controller does not answer (module missing or its 31-pin FPC not fully seated)"
fi

serial=$(grep -ao 'serial [0-9A-F]\{12\}' "$log" | head -1 | cut -d' ' -f2)
if [ -n "$serial" ]; then
    # The USB serial is the BLE address, byte-reversed.
    address=$(echo "$serial" | sed -E 's/(..)(..)(..)(..)(..)(..)/\6:\5:\4:\3:\2:\1/')
    echo "flash-new-board: USB serial $serial, Bluetooth address $address"
    mv "$log" "$logs/$serial-$(date +%Y%m%d-%H%M%S).log" && log=$(ls -t "$logs/$serial"-*.log | head -1)
fi
echo "flash-new-board: boot log in ${log#"$root"/}"
if [ "$failures" -gt 0 ]; then
    echo "flash-new-board: $failures check(s) failed"
    exit 1
fi
echo "flash-new-board: board is up; plug its USB-C into the Mac and allow the accessory once, then tools/flash-usb.py --status"
