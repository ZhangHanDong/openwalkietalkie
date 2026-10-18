# Native Bluetooth microphone and speaker firmware

This target uses SiFli's dual-mode Bluetooth host and RT-Thread audio runtime.
The Rust static library in `core/` reuses the Agent Command Deck UI and model,
control state, power policy, charger, touch/keypad protocols, the deck's
channel-5 protocol and the HID system RPC (`src/link/wire.rs`) from `src/`, mounted with
`#[path]` under the same module names the Embassy firmware uses.

Audio travels from the display board's analog MEMS microphone through AUDCODEC,
AUDPRC and the SDK HFP voice path to the Mac over SCO/eSCO. HFP negotiates CVSD
(8 kHz) or mSBC (16 kHz). macOS opens the voice link when an application uses
this input. BLE HID (advertised as `OpenWalkieTalkie`, VID 1209 PID 5754 in
its PnP ID; the Classic name is the same) runs alongside HFP and carries the
same channel-2 system RPC and channel-5 deck lines as the Embassy build.

Playback uses the SDK's A2DP SBC decoder, AUDPRC and AUDCODEC DAC, then the
board's NS4150B amplifier and mono speaker. Normal Mac output uses A2DP;
opening the microphone switches output to the HFP voice link (8/16 kHz).
Music output returns to A2DP when microphone use ends. The speaker combines
both stereo channels, so sounds panned entirely left or right remain audible.

The USB-C connection remains the CH343 serial/debug connection.

### Diagnosing sleep without opening USB serial

Serial attachment can reboot this board and erase the failure state. Use the
Bluetooth vendor interface when investigating unplug/wake failures:

```sh
cc -Wall -Wextra -Werror tools/diagnose-device.c \
    -framework IOKit -framework CoreFoundation -o /tmp/owt-diagnose
/tmp/owt-diagnose --watch 900
```

For charge-completion investigations, use `/tmp/owt-diagnose --battery --watch 180`.
It reports all charger registers, SOC, cached voltage, and the pending completion
event's age. Register read failures are explicit; REG09 reads can clear latched
fault indications. Near-full status polling and fresh-voltage confirmation are
described in [battery estimation](../docs/battery-estimation.md).

The default single request and `--watch` read status without waking the screen,
resetting peripherals, draining keypad events, or changing charging settings.
`--wake` explicitly requests a display wake without a key action. Status includes
uptime, run clock, screen state, GPIO levels, cached touch report, touch mode,
keypad/expander registers, charger registers, I2C failure bits, lifetime input
IRQ counts, battery millivolts, display/audio activity, pre-initialization
charger faults, SDK boot cause/mode, and the deep-WFI build setting. Watch
reacquires the HID interface after a disconnect and records host timestamps.
The GPIO mask's bits
0–5 are PA1, PA11, PA0, PA9, PA41, PA44 on the dev board (touch interrupt,
keypad interrupt, touch reset, panel reset, system enable, panel supply); the
Rev A build fills the same slots with PA43, PA39 (encoder A), PA44, PA9, PA11
(panel logic), PA10 (panel 4 V). Do not use an active diagnostic watch
as a battery-current benchmark.

This private protocol uses vendor report 6, channel 4, version 2. Request body
bytes 2/3 are operation (0=status, 1=wake) and sequence; replies set operation
bit 7 and echo the sequence. Only the encrypted HID connection can invoke it.
Channel 2 is the system RPC (`sys.version`, `sys.reboot`, `device.status`); the
deck uses channel 5 (see [Agent Command Deck](#agent-command-deck)).

## Build

Battery SOC uses the [shared stateful estimator](../docs/battery-estimation.md).
The UI and host battery reports retain their estimate across USB transitions;
UART battery lines include both the raw voltage lookup and reported SOC.
Charging limits and watchdog behavior are unchanged by this estimation fix.

Use SiFli-SDK commit `006d03293e0621d694335cd4031773279fa2f19a`, with its
Bluetooth submodule at `c3f204157265c275b5a9af8b20ef40663d9feab2` and
SiliconSchema submodule at `304658c6edd5497211b7d152932327ce6e4450ff`.
The tested compiler is SiFli's Arm GNU Toolchain 14.2.Rel1 (GCC 14.2.1).

```sh
git -C /path/to/SiFli-SDK checkout 006d03293e0621d694335cd4031773279fa2f19a
git -C /path/to/SiFli-SDK submodule update --init --recursive
cd /path/to/SiFli-SDK
./install.sh
source ./export.sh
cd /path/to/openwalkietalkie
tools/build-native.sh                  # LilyGo T-Display SF32 (upright panel)
OWT_BOARD=hw-a tools/build-native.sh   # custom Rev A hardware (panel turned 180°)
OWT_BOARD=hw-b tools/build-native.sh   # Rev B main board (hardware/rev-b)
```

Every board links to the same `main.elf` path, so the script prints the
selected board, and the firmware's boot log repeats it (`board: lilygo
image, rotation=0deg`, `board: hw-a image, rotation=180deg` or `board: hw-b
image, rotation=180deg`); Settings → About names the board too. Check it
before judging a picture to be upside down.
`OWT_BOARD=hw-a` also selects the Rev A pin map on the C side
(`OWT_BOARD_HW_A`, defined in `native/boards/openwalkietalkie/SConscript` so
that the bootloader child project compiles the board files for Rev A too):
panel rails on PA11 and PA10, touch on PA42/PA41 with reset PA44 and
interrupt PA43, the six
buttons and the wheel on GPIOs (`owt_inputs_read`, decoded by the shared
`src/input/direct.rs`), no keypad and no I2C charger. `OWT_BOARD=hw-b` is
that map plus `OWT_BOARD_HW_B` for the three pins Rev B moved: the speaker
amplifier's enable on PA01 (`OWT_AMP_PIN` in `audio.c`), the CAL pad parked
on PA00, and the BQ24073's CHG output on PA33 (`owt_charger_charging`, which
the core turns into a charging state; this image has no USB device, so the
end of a charge reads as no input). A C side and Rust core built for
different boards is reported at boot as `board: MISMATCH`.

The SDK checkout needs `example/boot_loader`, `example/ble/lcpu_general`,
`example/rom_bin`, the board/peripheral files, drivers, RTOS, middleware,
external libraries, and build tools. A full checkout includes these.

`tools/build-native.sh` checks both flash partition bounds and SRAM placement
of clock-transition helpers. Keep the board wrapper named `sdk_bsp_init.c`:
the SDK's linker matches `*bsp_init.o` to place it in RAM while external NOR
clocks change. `ram_memory.c` supplies a RAM `memset` because Rust's weak
implementation does not match the SDK's newlib object-name placement rule.
These requirements are covered by checks on the final ELF, not just source
annotations. See the [HardFault investigation](../docs/idle-power-audit.md).

## Flash and connect

The default image is upright on the LilyGo development board; use it there
without turning the board over. An `OWT_BOARD=hw-a` image turns the splash, UI
and touch mapping 180° for the Rev A hardware, which mounts its panel upside
down; flashed onto the development board by mistake it shows upside down. See
[screen mounting and animation](../docs/display-performance.md#screen-mounting-and-animation)
for the shared orientation setting and visual checks.

```sh
python3 tools/check-native-image.py native/project/build_openwalkietalkie_hcpu/main.elf
SIFLI_SFTOOL_PORT=/dev/cu.usbmodem5B910815801 \
    tools/flash.sh native/project/build_openwalkietalkie_hcpu/main.elf
```

The Rev A board is flashed over UART1 on test pads TP6 (board RX), TP8
(board TX) and TP4 (GND); nothing there can pulse its reset, so `tools/flash.sh`
does not apply and `sftool` is driven by hand. A blank board first needs the
flash table and the SiFli bootloader, which `OWT_BOOT_IMAGES=1` builds as
child projects of the app from the board's `ptab.yaml`. A chip with no flash
table waits in the boot ROM's UART download monitor, so the first flash needs
no reset timing:

```sh
OWT_BOARD=hw-a OWT_BOOT_IMAGES=1 tools/build-native.sh   # hw-b for a Rev B board
sftool -c SF32LB52 -p /dev/cu.usbmodem3101 --before no_reset --compat true \
    write_flash --verify \
    native/project/build_openwalkietalkie_hcpu/ftab.bin@0x12000000 \
    native/project/build_openwalkietalkie_hcpu/bootloader/output/bootloader.bin@0x12010000 \
    native/project/build_openwalkietalkie_hcpu/main.elf
```

On Rev A the microphone is the MP34DT05 PDM part on PA22 (clock) / PA23
(data), powered through a load switch on PA28, so the `OWT_BOARD=hw-a` build
switches the audio server's receive path to PDM1: `tools/build-native.sh`
installs `native/boards/openwalkietalkie/hcpu/hw-a.conf` as the SDK's per-board
overlay (`native/project/openwalkietalkie_hcpu/proj.conf`, merged after
`proj.conf` for the main project only) and `board.c` wraps `HAL_PDM_Init` /
`HAL_PDM_DeInit` to switch the microphone's supply on for exactly as long as
the PDM block runs. The dev board keeps the codec's analog microphone
(`board.conf`). Verified 2026-10-03: recording from the Mac's deck input
(then named "Codex Micro") opens the voice link (`HFP: voice stream result=0 codec=255 air=3
rate=16000`, mSBC), the board logs `microphone: MP34DT05 powered (PA28), PDM1
on PA22/PA23` and per-second `microphone: frames=100 … peak=…` lines (about
350-430 at rest, 1300-2400 for speech), and the Mac's recording shows the
speech at peaks of 10000-16000 against a noise floor under 60.

The native image has no soft power-off yet. The shared UI shows the
`Power off?` dialog when CANCEL is held (it is the same `ui` tree as the
Embassy image) but nothing follows at 3 s; the Embassy `hw-a` image enters
the PMU's hibernate state there (`src/pmu.rs`). The native equivalent is the
SDK's `HAL_PMU_SelectWakeupPin` / `HAL_PMU_EnablePinWakeup` /
`HAL_PMU_EnterShutdown` from `native/src/idle_power.c`, with the audio
server stopped first.

## Bootloader with USB recovery (`native/bootloader`)

`OWT_BOARD=hw-a OWT_BOOT_IMAGES=1 tools/build-native.sh` builds the
second-stage bootloader from `native/bootloader` instead of the SDK's stock
project: the same SDK sources (`example/boot_loader/project/butterflmicro`)
plus `owt/recovery.c`, which wraps the last call before the jump to the
application (`board_init_psram`, linked with `--wrap`) and turns the board
into its recovery device (CherryUSB on the chip's musb controller, bare
metal) when

- the application asked for it (`sys.reboot` with `mode: recovery` over the
  vendor HID channel writes the request word into RTC backup register 7 and
  reboots through the PMU),
- SHIFT and CANCEL (ESC) are both held while the chip comes up, or
- the application image is not valid: its vector table at `0x12020000` is
  checked before every boot, so erased flash or an interrupted download never
  gets jumped into.

Recovery keeps the application's vendor HID collection as interface 0:
"OpenWalkieTalkie" (`1209:5754`, manufacturer "Cohdl", the same strings,
chip-derived serial, report descriptor and HID endpoints). Current recovery
also exposes vendor bulk interface 1 (`ff/57/01`, OUT `0x03`, IN `0x85`, 64-byte
full-speed packets). macOS accessory approval applies to the composite device.
The compatible update path travels on channel 9 of the same 64-byte reports
(`[6][9][len][payload]`): HELLO, BEGIN (erases the vector-table sector),
DATA pieces into a 4 KB sector buffer, COMMIT with address, length and CRC-32
(erase, program, read back), FINISH (writes the held vector-table sector,
checks it, reboots), BOOT and READ. Protocol 2 writes only the two application
banks (`0x12020000..0x126FFFFF` and `0x12720000..0x12FFFFFF`), preserving both
Bluetooth stores between them. Clients check the recovery version before BEGIN;
large images are refused on protocol-1 recovery before any erase. A download that
stops half way leaves an image the bootloader refuses and comes back to
recovery by itself. The deck host's own traffic on the other channels is
ignored. Application recovery never writes the flash table at `0x12000000`
or the bootloader's slot. UART (`tools/flash-uart.py ... bootloader.bin@0x12010000`)
can provision both through the chip's always-on debug port regardless of the
contents of flash. For an acknowledged Rev A/B bootloader upgrade, a temporary
Embassy app built with `--features hw-a,bootloader-update`
rewrites the slot itself (`tools/flash-usb.py --bootloader bootloader.bin`,
deliberately left out of `--help`; `src/link/boot_update.rs`): a power loss
during that second leaves the UART pads as the only way back. If nobody
speaks the protocol within two minutes and the image is valid, recovery mode
boots the application. It logs `OWTB: ...` lines on UART1. The RSA image
verifier the SDK project carries is left out (`CONFIG_PKG_SIFLI_MBEDTLS_BOOT`
off: images are unsigned here), which makes room for the USB stack in the
64 KB slot (51,888 bytes used, including bulk and the recovery screen). `owt/usb_dc_musb.c` is the SDK's musb device
port with one fix (its EP0 OUT data stage only ended on a short packet, so a
control transfer that is an exact multiple of 64 bytes never completed);
`proj.conf` selects CherryUSB's "custom" port so the SDK's copy is not
compiled too, and `owt/usb_glue_sifli.c` is the SDK's glue unchanged.

Rev A/B recovery initializes the CO5300 AMOLED itself, with the same panel
rails, reset sequence, QSPI framing and 180-degree orientation as the app.
It shows recovery instructions, missing firmware, update progress, completion,
and CRC/flash/verification or USB-reset errors with retry instructions. Normal
boots do not initialize the display here. No audio or touch driver is started.
The renderer carries a small built-in font and uses a 15 KiB RGB565 band in
unused bootloader SRAM. Two 4,160-byte USB receive buffers share this NOLOAD
region, starting at `0x20030000` (23,680 bytes total); neither uses application
assets or PSRAM. `display_scratch.ld` keeps these buffers out of the load image
and asserts that the image fits below it in the existing 64 KiB flash slot.
LCDC command/DMA waits are bounded; a display fault stops drawing and leaves
the USB updater running. Rev A display orientation has been verified; Rev B
physical display validation remains pending.

HELLO still reports protocol 2, with an optional capability byte after the
image-valid byte: bit 0 enables sparse ERASE (op 8, COMMIT fields with implicit
FF data), bit 1 enables bulk v1. Missing capabilities mean the legacy protocol.
Both hosts prefer bulk and fall back to HID only before a transfer begins.
HID also uses sparse ERASE when advertised. Empty sectors are checked, erased
only if necessary, and verified; skipping their USB payload never preserves
stale firmware. Both Bluetooth stores remain excluded. An older protocol-2
bootloader continues to work at its previous speed; this optional acceleration
does not silently trigger another bootloader replacement.

After checking an incoming sector's CRC, recovery compares its bytes against
flash. Identical sectors are already verified and need no erase/program cycle;
the unused tail of a partial sector must also be FF before a rewrite can be
skipped. Changed sectors still get erase/program/readback verification. BEGIN
always invalidates the vector sector, which remains held until verified FINISH,
including when all other sectors match. This reduces update time and flash wear
when large font assets or code have not changed.

Bulk uses 32-byte headers and up to 4 KiB data, padded to a 64-byte boundary
after the header so every frame terminates with a short packet. Header fields
are `OWTF`, op:u8, version=1:u8, length:u16, address:u32, sequence:u32,
session-token:u32, data-CRC32:u32, header-CRC32(first 24 bytes):u32, reserved=0:u32.
All integers are little-endian. BEGIN (2) has sequence/length/data-CRC zero,
address=image span and a fresh nonzero token. COMMIT (4) includes data; ERASE
(8) includes only the header and CRC of the implicit FF bytes. FINISH (5) has
zero address/length/data-CRC. Sequence advances from 1 after BEGIN.
The 24-byte acknowledgment is `OWTA`, op, status, version=1, reserved=0,
sequence:u32, token:u32, address:u32, CRC32(first 20 bytes):u32.
Two receive buffers and two host requests overlap reception and flash work.
Every sector is CRC-checked and verified before acknowledgment; FINISH checks
complete coverage and commits vectors last. Errors require a fresh BEGIN;
hosts never switch transport or send FINISH after a failed/uncertain transfer.
Before any sector data is submitted, hosts may repeat BEGIN once to flush an
old partial USB frame. CRC-valid acknowledgments from another session are
drained within a deadline; matching-token replies must still pass sequence,
address and status checks. This does not retry uncertain sector writes.

The desktop uses native USB through `nusb`, with no external libusb dependency.
`flash-usb.py` optionally uses PyUSB/libusb; `--transport auto|hid|bulk` selects
automatic negotiation, compatible HID, or required bulk. Missing PyUSB/libusb
falls back to HID in auto mode before any erase. Bulk mode fails before erase
when unavailable. The script binds recovery and reconnection to the same serial.

The desktop and `flash-usb.py` send an optional four-byte little-endian image
span after BEGIN. Old bootloaders ignore it. New recovery validates it before
erasing and counts unique CRC-checked sectors, excluding the two Bluetooth
stores. It holds progress at 99% until FINISH writes and validates the vector
sector; incomplete declared transfers cannot FINISH. Old clients' empty BEGIN
still works and shows "RECEIVING DATA" without an invented percentage. This is
an optional protocol-2 extension, not a protocol version change. Already-installed
protocol-2 bootloaders continue to work; an application-only update does not
retrofit their screen. The screen accompanies the new recovery image installed
by the acknowledged protocol-1 migration or explicit bootloader provisioning.
During the temporary app's replacement of the bootloader, progress and the
power-loss warning remain in the desktop app.

Host side: `tools/flash-usb.py <ELF or .bin>` (needs the `hid` Python
module, nothing else) asks a running deck to reboot into recovery, waits for
it to re-enumerate, downloads sector by sector (about 32 s for a 1 MB image)
and waits for the application to come back and answer `sys.version`;
`--no-request` for a deck already in recovery (buttons, or no valid image),
`--status` to see what is attached and in which mode. Verified 2026-10-04:
a full download from a running deck, and a deck whose vector-table sector had
been erased recovering over USB-C alone.

To exercise both audio paths from the Mac: `blueutil --connect
fb-a6-f6-4c-57-4e` (the deck's address) attaches the Classic profiles, which
macOS does not do by itself after pairing the BLE HID side; then
`tools/test-speaker.c --tone` / `--loud` and `tools/test-microphone.c
--seconds 10` (build both with the `cc` lines in their headers). **Full-scale
output at maximum volume while the board runs from USB with no cell fitted
resets the board about a second into the tone**: the BQ24073's USB500 input
limit cannot carry the amplifier's peak draw and SYSTEM_SUPPLY browns the MCU
out (no fault dump, a clean reboot). Loud tests belong on a fitted cell, and
the firmware's volume ceiling without a cell is an open item.

The Rev A USB-C data pair is served by the Embassy firmware only
(`src/link/usb.rs`, the deck's HID device over the cable); the native
target does not enable the SDK's USB device stack yet, so over the cable the
native image only charges. The SDK's HID class has never been exercised on
the 52x, its endpoint table lacks the interrupt-OUT entry the deck needs and
`BSP_USING_USBD` moves DLL2 to 240 MHz, which the run-mode switcher then
turns off at screen-off; that is the work a native USB port would take.

`--compat true` is for USB CDC bridges such as a DAPLink's UART side, which
drop sftool's 64 KiB stub frame. Later flashes write only the ELF and need
no reset: the debug port answers while the firmware runs, so
`tools/flash-uart.py /dev/cu.usbmodem3101 write_flash --verify
native/project/build_openwalkietalkie_hcpu/main.elf` (it clears a stale debug
session, then runs sftool with `--before no_reset --compat true`).

Switching from the Embassy Bluetooth host requires forgetting the old
**OpenWalkieTalkie** pairing and pairing the new firmware (a deck still paired
under its former name, **Codex Micro**, must be forgotten too). Select
**OpenWalkieTalkie** in macOS Sound → Input; PTT talks to the deck host from
any face. Advertising flags identify
simultaneous LE/BR-EDR support, and Classic discovery includes HFP, Audio Sink
and AVRCP service UUIDs. LE-to-Classic cross-transport key derivation is enabled. The pinned BT
library lacks the reverse-direction callback, so that option stays disabled.

The serial log identifies the device address, HID connections and disconnect
reasons, HFP profile connections, and the negotiated microphone codec/rate.
While the voice link is capturing, a `microphone: frames=... samples=...
peak=... muted=...` line reports microphone PCM activity once per second.
Pairing material and audio payloads are not logged. A successful build or HFP
advertisement alone does not validate microphone capture; verify an input
recording and simultaneous PTT operation on the Mac.

If audio works but the HID link (deck host, system RPC) disappears, check the
BLE pairing separately.
macOS can retain the Classic audio bond while its BLE HID bond is missing or
stale; an audio-connected entry does not establish that HID is connected.
Reconnect the disconnected OpenWalkieTalkie entry in Bluetooth settings and
accept the renewed **Pair** prompt promptly. Verify `BLE: link encrypted`, HID
subscription/report writes and a successful `device.status` reply.
Do not erase firmware bond storage merely because audio reconnects without HID.

The records below date from the Codex Micro emulation (removed since), when the
deck paired as **Codex Micro** and answered the Codex app on channel 2.

Diagnosis on 2026-09-09: macOS reported `classicPaired 1, lePaired 0`. A direct
BLE reconnection succeeded and the board served its HID map and both report
references, but the Mac's replacement Just Works pairing alert timed out after
30 seconds. No encrypted link or macOS IOHID interface appeared. The same
pairing failure preceded the battery-saving update; completing the Mac pairing
is required before a HID regression can be assessed.

After the user accepted pairing at 18:04, macOS enabled BLE encryption and
published the Codex Micro IOHID interface; icon/layout synchronization resumed.
HFP, A2DP and BLE then appeared connected simultaneously. A silent speaker
open/close succeeded with continuing HID writes, and the amplifier shut down
on suspension. No firmware change was needed to restore this HID connection.

Validation on the connected SF32LB525 board (2026-09-09): the native image builds,
passes the application flash-boundary check, flashes with verification, and
boots with display, I2C, battery ADC, charger, keypad, touch, BLE HID advertising,
and HFP discoverability initialized. The native Rust core's 42 host tests pass,
including cancellation of a stale touch press across clock wrap and HID status
requests with and without the report-ID byte. The original Embassy target and
its 42 host tests also pass.

After re-pairing, macOS exposes Codex Micro as a 16 kHz Bluetooth microphone.
A five-second Core Audio capture received 81,920 samples, including nonzero
data, without saving audio. BLE HID and HFP reconnect after flashing. While
audio is active, a hardware HID probe receives battery/charging status, and
Codex's own log confirms successful `v.oai.rgbcfg`, `v.oai.thstatus`, and
`device.status` replies. Listening/speech quality, physical PTT operation,
display appearance, and longer-term connection stability still need user
validation.

## Speaker playback

Select **OpenWalkieTalkie** in macOS Sound → Output, or select it directly in an
application that supports choosing its output. The built-in speaker plays
computer audio over Bluetooth; USB remains the serial/debug connection.
Mac volume controls apply to playback. AVRCP absolute-volume messages map
0–127 to the SDK's music volume range, including mute; hosts that scale PCM
locally also work. HFP speaker gain is handled separately by the SDK.

The native target uses the Bluetooth
[Speakerphone appearance (`0x0845`)](https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Assigned_Numbers/out/en/index-en.html)
in both advertising and the GAP characteristic. The old Keyboard appearance
made macOS omit this composite device from Sound's Output list even though
Core Audio reported a live, visible, default-eligible output and played audio
through it. Restarting System Settings did not repair the list. After updating
the appearance, **Codex Micro — Bluetooth** (the deck's name then) appeared selected in Output, and
the Mac sent AVRCP volume (`63` mapped to local level `7`). HID kept the same
service UUID, report descriptor and VID/PID. Upgrading from the old
appearance may prompt macOS to pair the HID connection again; accept the
pairing prompt before testing the controls.
The appearance revision passed the flash-boundary check and was flashed with
verification. Core Audio still selected Codex Micro for input, output and
system sounds; a five-second microphone check received 80,320 samples with
nonzero data, and a silent playback open/close succeeded. HID verification for
this revision awaits acceptance of the Mac's renewed pairing prompt.

Follow-up after successful HID re-pairing on 2026-09-09: the user again reported
Codex Micro missing from System Settings → Sound → Output, while Control Center
showed it selected. Core Audio reported its output alive, not hidden, and selected
for both default output and system sounds; stereo playback and HID remained
available. The Speakerphone appearance change therefore does not fully resolve
the System Settings listing issue. Control Center can select the working output;
the cause of the differing device lists remains unresolved.

PA20 enables the NS4150B in mode 1, following the
[manufacturer's pin map](https://github.com/Xinyuan-LilyGO/T-Display-SF32) and
[amplifier driver](https://github.com/Xinyuan-LilyGO/SlFli-SDK-Lilygo/blob/master/sifli-sdk/t_sf32_display_hw_device/pa/ns4150b/ns4150b.c).
The amplifier stays off at boot and between playback sessions. The SDK starts
the DAC muted; the board hook resets the amplifier low for 550 µs, enables it,
and allows 100 ms to settle before unmuting. Silence filtering may power it
down during gaps in music. The mono conversion preserves the SDK's stereo DMA
format and leaves mono HFP data unchanged.

`speaker: enabled ... rate=... channels=...` and once-per-second
`speaker: frames=... samples=... peak=... enabled=...` logs report the output
path without recording audio. A2DP packet notifications bypass the UI queue;
the SDK playback worker already owns and decodes those buffers.

The pinned SDK opens an A2DP audio client before its prebuffer reaches the
playing state. Its normal stop path skips that client if the host suspends
immediately, leaving silent DAC output running and able to resume after HFP.
The board's suspend/close/abort/disconnect hooks ensure the SDK drains its
decoder worker and closes the client even during prebuffering. No SDK checkout
modifications are required.

Run the host PCM/volume checks and an explicit-device Mac playback probe:

```sh
cc -std=c11 -Wall -Wextra -Werror tests/speaker_pcm.c -o /tmp/owt-speaker-pcm-test
/tmp/owt-speaker-pcm-test
cc -std=c11 -Wall -Wextra -Werror tools/test-speaker.c \
    -framework AudioToolbox -framework CoreAudio -framework CoreFoundation \
    -o /tmp/owt-speaker-test
/tmp/owt-speaker-test --list
/tmp/owt-speaker-test --tone
/tmp/owt-speaker-test --silence
```

The probe plays quiet left-only, right-only and both-channel tones, without
changing the Mac's default output or volume. Raise OpenWalkieTalkie's output volume
if needed: these deliberately attenuated tones were inaudible at 50% in the
bring-up test and audible at 75%; the original volume was restored afterward.
`--silence` opens a one-second silent stream to exercise early-suspend cleanup:
after the host suspends it, the log should show the amplifier disabled with no
continuing speaker frames.

Speaker validation on 2026-09-09: the verified image flashed while preserving
bonds; the Mac reconnected without re-pairing and exposed a 44.1 kHz, two-channel
output. The user confirmed hearing the tones. Simultaneous playback and capture
switched to 16 kHz HFP; a five-second capture received 80,640 samples with
nonzero data, and three concurrent HID status requests completed in 40–79 ms.
The Mac returned to its 44.1 kHz output after capture, and another tone test
confirmed playback resumed with concurrent HID replies. PCM/volume checks pass.
The subsequent cleanup revision also flashed and passed two silent-session
checks plus simultaneous playback/capture (82,560 captured samples). After
HFP closed and each silent A2DP session suspended, the amplifier shut down
without continuing DMA frames. All HID probes replied; one request during the
repeat test took 1.63 seconds, so these checks do not establish a latency bound.
The initial Mac test used local PCM volume scaling; the appearance revision
also exercised AVRCP volume as described above. Extended listening and echo
quality have not been evaluated.

The LCD layer must be reset explicitly after controller initialization: the
SDK's zero-initialized layer has zero alpha and stride. The driver sets opaque
RGB565 and a 480-pixel row stride for every band. HID Output allows 64 bytes
because macOS can include report ID 6 in its GATT write; the shared decoder
also accepts the usual 63-byte BLE body. A 63-byte maximum rejects those Mac
writes with ATT error 0x0d before the application callback runs.

## Touch interface and preview

Both targets show the website's yellow pixel-radio icon and stacked
“OPEN / WALKIE / TALKIE” hero title at boot, in cream and gold on black.
A small symbol from `assets/CoHDL-透明白2k.svg` sits beside “A COHDL PROJECT”
at the bottom.
The panel stays off until all ten splash bands have replaced its startup
contents. The logo is visible for 1.2 seconds before the Agents face appears.
On the native target, peripheral initialization and Bluetooth processing run
during that interval; touch interpretation starts with the first UI frame.
The splash reuses the existing 48-row framebuffer and stores about 34 KB of
bitmaps in flash, without allocating a full-screen buffer or parsing SVG or
fonts on-device.

Regenerate the assets with `python3 tools/generate-splash.py` (CairoSVG, Pillow
and `fontTools[woff]` required). Sources are the website's `logo-64.png`, its
OFL-licensed Press Start 2P font (license in `site/public/fonts/PressStart2P-OFL.txt`),
and the supplied COHDL SVG. The radio keeps its original pixel grid; the COHDL
symbol keeps its proportions. Source hashes are recorded in
`src/ui/splash_asset.rs`. On macOS
with Homebrew Cairo, set `DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/opt/cairo/lib`
if CairoSVG cannot locate the library. Normal firmware builds use the generated
assets directly and do not require Python graphics packages. Preview the exact
firmware rendering with `python3 tools/preview-all.py splash`.

Both firmware targets draw the same Agent Command Deck (see the
[top-level README](../README.md#faces)).

Lucide glyphs (`assets/lucide/`) come from the actual icon-font subset, with
its source hash, glyph codepoints, license and generated alpha masks.
Regenerate the masks with `python3 tools/generate-ui-glyphs.py` (Pillow
required). No hand-traced paths are used for these icons.

The missed-interrupt regression injects raw CST9220 frames with INT held high
and verifies that polling still produces a complete press/release.

The native driver latches PA1 falling edges and uses 50ms idle / 20ms active
fallback polling. Previously an entire interrupt pulse could
be missed while rendering, leaving the UI without a Down event. Startup logs
include the first four raw report headers; touch Down/Up/Cancel logs include
coordinates and whether HID is ready. Coordinate logs contain no audio or
pairing material.

The Codex Micro replica image of 2026-09-09 (since removed) was flashed with
verification: the board reported its first complete frame, reconnected
encrypted BLE HID and HFP, and answered a hardware `device.status` probe; a
five-second Mac microphone check received 80,320 samples at 16 kHz with
nonzero data. A later revision's first four raw controller reads showed an
idle buffer followed by acknowledged `0xAB` buffers, confirming normal-mode
I2C read/write access. No physical Down/Up events were captured during those
checks, so the missed-interrupt repair is covered by the raw-report regression
but still awaits a physical tap/swipe confirmation.

### Tests and previews

```sh
cargo test --manifest-path native/core/Cargo.toml --lib \
    --target "$(rustc -vV | sed -n 's/^host: //p')" \
    --target-dir target/native-core-tests --locked --offline
cargo run --manifest-path native/core/Cargo.toml --example ui-preview \
    --target "$(rustc -vV | sed -n 's/^host: //p')" \
    --target-dir target/native-core-preview --locked --offline -- /tmp/ui.ppm
```

The native core crate runs 451 tests (2026-09-18, the same count with
`--features hw-a`): the shared `ui`/`deck`/`power` inline tests,
`core/src/native_tests/` (the runtime glue driven through its C entry points:
HID input and the system RPC, power and battery, the deck's channel-5 session from boot through the
demo to a real host and its loss), and `core/src/render_tests/` (per-face render checks,
rotated partial updates, the slide, and the partial == full fuzz walk).
`tools/test-all.sh` runs them for both boards together with the shared host
tests (398), and `--native-link` also links this firmware for both boards; see
the [top-level README](../README.md#building-and-flashing).

The preview renders the actual band framebuffer to a PPM image; append a scene
name after the output path. `ui-preview --list` names every scene:

- Boot: `splash`.
- Agents face: `roster`, `roster-offline`, `channel`, `channel-question2`,
  `channel-ready`, `channel-long-reply`, `channel-gone`, `card-question`,
  `card-question-sent`, `card-question-long`, `card-question-offline`,
  `card-ready`, `card-ready-step`, `card-none`.
- LISTENING overlay: `ptt-roster`, `ptt-omarchy`, `ptt-no-host`, `ptt-naming`,
  `ptt-answer`, `ptt-long-transcript`.
- Other faces: `omarchy`, `omarchy-scrolled`, `macropad`, `macropad-pressed`,
  `macropad-empty`, `usage`, `usage-updating`, `settings-volume`,
  `settings-speech`, `settings-alerts`, `settings-display`, `settings-host`,
  `settings-about`, `battery-low-header`.
- Manager and notifications: `manager`, `manager-grabbed`, `manager-confirm`,
  `gallery`, `naming`, `toast-question`, `toast-ready`, `toast-message`,
  `toast-not-delivered`, `drawer`, `indicator`.
- Face slide: `slide-25`, `slide-50`, `slide-75`.

`tools/preview-all.py` renders them all to PNG, with comparison sheets against
`docs/mockups/`. The native serial log prints
`UI: first frame ready` after the first complete frame has been sent to the
display.

## Agent Command Deck

The deck's UI state, model and protocol are the shared `src/ui` and `src/deck`
code; this runtime adds the glue in `core/src/lib.rs` and `src/main.c`.

- **Inbound.** The GATT write callback copies each HID output report into the
  application message queue (64 entries, shared with input and diagnostics;
  32 before the deck). A full queue answers the write with ATT error `0x11`
  and disconnects HID, so a lost release cannot stay held. The main loop
  drains at most 16 messages per pass. `owt_core_hid_report` sends a channel-5
  body (with or without report id 6) to `deck::proto::LineAssembler`; each
  completed line is parsed into a static `HostMsg`, applied (or rejected
  through the UI's rate-limited `err`), and its consequences are emitted
  before the next line. HID connect and disconnect clear the assembler.
- **Gating.** Channel 2 only answers system RPC requests
  (`wire::handle_request`); the device never speaks first there. Deck events go out on channel 5 only while
  the deck host is present and has sent a channel-5 line on this link. Until
  then, while demo data is active, events are answered by the built-in demo
  host (`demo_loopback`, also on tick updates, so its follow-ups arrive).
  An `Update` overflow sends `up` (when online) and disconnects HID.
- **Outbound.** Each event is encoded into a static 640-byte line buffer and
  framed into 61-byte channel-5 reports, 4 ms apart like the channel-2 replies.
  An event that does not fit is dropped whole; a cut line never reaches the
  host.
- **Screen.** A new card notice calls `Policy::notify` (lit for 15 s without
  counting as input). `owt_core_brightness()` scales the power state by the
  Settings brightness; `main.c` re-applies it whenever the value changes,
  which is how the Display rows take effect immediately.
- **Stack.** `UiState`, the frame snapshot, both signatures, the line
  assembler, `HostMsg` and the TX line are statics filled in place. The main
  thread stack is 24576 bytes (`CONFIG_RT_MAIN_THREAD_STACK_SIZE`, was 12288),
  and `tools/test-all.sh --native-link` fails on any Rust frame over 2 KB.

The channel-5 contract is [docs/deck-protocol.md](../docs/deck-protocol.md);
`tools/deck-sim.py` runs the mockup session against a flashed device. None of
this has been exercised on hardware yet; see the
[first-flash checklist](../docs/deck-device-checklist.md).

## Battery saving

The shared policy dims at 30 seconds and sleeps the CO5300 at 120 seconds of
local inactivity. The lit brightness follows Settings → Display brightness 1–5
(panel levels 40/80/130/190/255, default 5); dim is 32, or the lit level if
that is lower. Screen-off sends Display Off and Sleep In. Wake waits 120ms after
Sleep Out, renders all ten bands while hidden, then enables the panel. These
waits use state transitions so the native application continues servicing
touch, HID and audio. The complete wake touch is consumed. Hardware PTT wakes
and acts; every other bound key and wheel detent pressed after the panel slept
and before the wake frame is shown only wakes, and its release is consumed too
(unbound matrix keys also wake). A new deck card lights the screen for 15
seconds without resetting the inactivity clock; the demo's own notices never
wake it. Idle-home uses the same inactivity clock.

PA1 touch and PA11 keypad interrupts wake an RT-Thread event wait, as do queued
host messages. Idle fallback is 50ms while visible and 500ms while dark with no
held input. Interrupts bypass that timeout immediately. Held inputs use a 5ms
fallback; active touch reports retain a 20ms fallback and the 120ms stale-contact
cancellation. Pending redraws draw into the free buffer immediately. LCDC completion
wakes the application, with a 1ms fallback and bounded DMA timeout.
The TCA8418 interrupt is acknowledged after draining its FIFO, then the pending
count is rechecked so events arriving during acknowledgement trigger another
immediate wake. The manufacturer's keyboard schematic confirms PA11 and the
shared Wi-Fi/keypad supply. Only the
unused keypad LED enable is removed; other output/reset states are preserved.

Battery/charger service runs immediately and every 15 seconds. The SDK already
disables the VBAT monitor at the end of each ADC read. Native BLE uses the SDK's
30-second fast advertising phase (60ms interval), then indefinite background
advertising (1 second); disconnect restores fast discovery. Classic sniff and
audio parameters remain managed by the SDK.

HCPU enables SiFli's PM framework and guarded deep-WFI clock scaling. A strong
board policy and mode-selector override cap sleep at `PM_SLEEP_MODE_IDLE` from
startup, preserving ordinary PA1/PA11 interrupt wake. The SDK's AON-only
LIGHT/DEEP/STANDBY paths remain blocked; the selected `PM_LIGHT_ENABLE` build
option does not authorize entry into LIGHT. See
[HW-001](../docs/hardware-design-constraints.md#hw-001--reserve-wake-capable-pins-before-allocating-other-peripherals).
The SDK checks hardware activity and its radio IPC queue before scaling idle
clocks. Direct LCD transfers, complete battery conversions and bond-store writes
hold hardware activity guards, alongside the SDK's I2C and audio guards. Active
CPU performance and the 1kHz OS tick remain unchanged.

Every 30 seconds, `power:` diagnostics report state (0 active, 1 dim, 2 off),
I2C transactions, display bands, successful watchdog feeds, failures, and the
VBAT monitor enable bit. A second line counts PM calls and hardware/IPC-busy
conditions sampled on entry, with `sleep_ceiling=IDLE`. These are activity
diagnostics, not exact deep-WFI counts, sleep residency or measured current.
BLE connection/update logs include the negotiated interval, latency and timeout.
The [battery-life plan](../docs/battery-life-plan.md) records implementation,
verification and the remaining hardware measurements needed to estimate runtime.

The [2026-09-10 idle audit](../docs/idle-power-audit.md) tracks the follow-up
implementation and its device validation separately from the initial results.

Battery-saving validation on 2026-09-09: all 62 native core and 58 shared host
tests pass, and both targets build. The verified native image was flashed;
logs show dim at 30,017ms and off at 120,022ms. Idle 30-second windows while off
show zero display bands, two successful watchdog feeds with no feed errors,
and the VBAT monitor disabled. BLE advertising changes from 60ms to 1 second.
A five-second microphone probe while dark received 81,920 samples with nonzero
data. Silent A2DP sessions suspend and disable the amplifier, including after
HFP closes and the Mac returns to 44.1kHz stereo. SDK voice drop/buffer warnings
occurred, so these checks do not establish audio quality or connection stability.
Physical wake/PTT verification and battery-current measurements remain pending;
the Mac's HID re-pairing prompt from the appearance revision also remains pending.

## Storage and scheduling

- Application: `0x12020000..0x126fffff`.
- Application assets: `0x12720000..0x12ffffff` (8.875 MiB), linked by the
  shared `flash-assets.x`. Together the application banks hold 15.75 MiB.
- Native FlashDB/NVDS (Classic bonds, BLE bonds and per-peer HID subscriptions):
  `0x12700000..0x1270ffff`.
- Preserved Embassy bond journal: `0x12710000..0x1271ffff`.
- Flash table at `0x12000000` and SiFli bootloader at `0x12010000` (it runs
  from SRAM at `0x20020000`): the LilyGo keeps its factory copies, Rev A gets
  the ones `OWT_BOOT_IMAGES=1` builds.

By default the build generates no bootloader or flash-table images and only
`main.elf` is flashed; `OWT_BOOT_IMAGES=1` adds both for a blank Rev A board
(see Flash and connect). The radio-core image is embedded in `main.elf` and
loaded by the SDK.
`check-native-image.py` validates all nonempty ELF load segments before flashing.
The original Embassy target still builds with `cargo build --release`.

Existing main boards need a one-time update of the recovery bootloader
before USB can install images containing upper-bank assets. With the SDK
environment loaded, build `OWT_BOARD=hw-a OWT_BOOT_IMAGES=1 tools/build-native.sh`
(use `hw-b` for Rev B). The Flash and connect procedure above writes `ftab.bin`,
the protocol-2 `bootloader.bin` and the application's **ELF**, preserving both
bond stores because no ELF load segment targets them. The same procedure is
available through `tools/flash-new-board.sh PORT ELF`. Never flash a padded
release `.bin` directly over UART: its gap contains placeholders, not bytes
that should replace Bluetooth data. `tools/flash-usb.py` and the desktop
updater skip that gap explicitly. The bootloader enforces the exclusion too.

Bluetooth callbacks queue bounded messages to the single application thread.
Only that thread enters Rust. UI rendering compares frame snapshots and draws only
damaged rectangles, split at 48-row boundaries. Two aligned SRAM buffers let LCDC
DMA transfer one region while Rust renders the next. Each transfer retains a
power-management clock guard until completion, and display sleep/brightness
commands wait for DMA. Input and host queues are serviced between regions; the
SDK audio threads handle voice and music playback. See
[display performance and validation](../docs/display-performance.md). Queue overflow or
a notification timeout disconnects HID so a lost PTT release cannot remain
held on the host. Subscription state is saved by peer identity after encryption.
Hardware RNG supplies the SDK's `rand()` calls used by legacy SMP, as well as
the persistent local IRK. HCPU deep sleep is disabled during initial bring-up.

### Upgrading Rev A recovery over USB-C

The desktop's automated migration now supports Rev A and Rev B with separately
identified images and explicit acknowledgement of the bootloader power-loss
warning. See [RELEASING.md](../RELEASING.md#device-firmware-updates), including the
pending shipped-Rev-B hardware validation. The commands below are the manual
maintenance path; use `hw-b` for both builds when working on Rev B.

A working Rev A/B with protocol-1 recovery can install protocol 2 without a UART
adapter using the temporary `bootloader-update` firmware. Its fonts remain in
the lower bank so the old recovery can install it. The feature must not be
included in normal firmware; distribute it only as the separate bootstrap image.
Its maintenance commands are USB-only. Keep USB-C power connected throughout:
power loss while replacing the bootloader requires UART recovery.

First build the new recovery with the SDK environment loaded:

```sh
OWT_BOARD=hw-a OWT_BOOT_IMAGES=1 tools/build-native.sh
cargo build --offline --locked --release --features hw-a,bootloader-update --target-dir target/firmware-bootstrap
python3 tools/check-xip-image.py --bootstrap target/firmware-bootstrap/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
python3 tools/flash-usb.py --serial SERIAL target/firmware-bootstrap/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
python3 tools/flash-usb.py --serial SERIAL --bootloader native/project/build_openmicrodial_hcpu/bootloader/output/bootloader.bin
```

Replace `SERIAL` with the connected deck's USB serial. Before erasing the
bootloader, the maintenance app checks its staged image's CRC and SRAM vectors
and the installed flash-table mapping: storage `0x12010000`, execution
`0x20020000`, flags `2` (automatic boot, no encryption), and sufficient load
length (at most 64 KiB).
An incompatible table is refused and must be updated over UART. `boot.info`
reports these fields; `boot.read` can back up the existing 64 KiB bootloader
slot in chunks of up to 64 bytes. The table and Bluetooth stores are unchanged.
The replacement is read back and CRC checked before reboot.

Finally install a normal build **without** `bootloader-update`, using the same
USB flasher. This places fonts in the upper bank and removes the maintenance
RPCs. Protocol 2 is checked before any application erase, and both Bluetooth
stores are skipped. Keep the maintenance build in its separate target directory
so it cannot be mistaken for a release image.

Verified on Rev A on 2026-10-17 using USB-C only: the temporary lower-bank app
installed through protocol 1, recovery v2 was written and read back, and the
expanded production app installed with per-sector verification. The existing
flash table was retained and the Bluetooth sectors were skipped. The application
answered `sys.version`, `boot.info` returned Method not found, and the desktop
service reconnected with zero device errors. No audio tests were run.

The display-enabled recovery was subsequently installed on the same Rev A over
USB-C and its full 50,564 bytes read back before reboot. The user confirmed that
the recovery progress screen was visible and upright during the production
0.3.1 transfer. The final application reported recovery protocol 2, rejected
`boot.info`, and reconnected to the desktop service. The temporary and final
application downloads took 210.4 s and 394.9 s respectively (about 31 KB/s).
Rev B hardware validation and interrupted-transfer display checks remain pending;
no audio tests were run.

USB bulk acceleration was verified on the same Rev A on 2026-10-18. The
51,888-byte recovery image was read back in full before reboot. A regular
reinstall of the 6,566,500 load-byte production image took 9.00 s through the
desktop transport and 9.9 s through Python (transfer/verification, excluding
application reboot), versus 394.9 s with the earlier HID uploader. Uploads
contain 6,665,056 bytes including bulk framing instead of 12,364,180 firmware
bytes over HID. This benchmark reuses matching flash contents: changed sectors
still require erase/program time. Restoring production after the temporary
updater and an interrupted packet took 89.86 s, including the bounded BEGIN
retry and cleanup of the temporary layout. A full stale-padding sector was
read back as FF; incomplete transfers stayed unbootable, both across reboot
and on a direct retry without disconnecting USB. Both clients returned to
production 0.3.1. No audio tests ran; Rev B hardware validation is pending.
