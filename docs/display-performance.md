# Display refresh performance

The native Bluetooth/audio firmware uses the CO5300 at **48MHz quad-SPI,
RGB565**. LilyGO's [official CO5300 driver](https://github.com/Xinyuan-LilyGO/SlFli-SDK-Lilygo/blob/master/sifli-sdk/customer/peripherals/co5300_t_sf32/co5300.c)
requests 50MHz and uses interrupt-driven pixel transfers. The pinned SiFli HAL
rounds its clock divider upward: with the observed 240MHz HCLK, requests for
48MHz and 50MHz both select divider 5, yielding 48MHz. The startup log now
reports the frequency calculated from HCLK and the LCDC divider register.

## Changes

- Compare frame signatures (regions with a hash of their inputs, computed from
  a complete UI snapshot) and redraw only changed regions. Damage includes
  both old and new key bounds, shadows, glows, and merged microphone layouts.
  A face without fine regions falls back to one full-screen region; a strip
  slide changes the frame mode and redraws everything.
- Split damage at 48-row boundaries and round coordinates outward to even pixels.
  The draw target clips to that rectangle, and the panel receives the same window.
- Use two aligned 45KB SRAM buffers. Draw into one while interrupt-driven LCDC
  DMA reads the other; preserve the 480-pixel source stride for cropped regions.
- Service input and Bluetooth queues between regions without the old 5ms delay.
  Wait for DMA completion only when needed, with an interrupt wake, 1ms fallback,
  and a one-second fault timeout. DMA errors restart the device rather than
  leaving the display queue permanently blocked.
- Retain the PM clock guard throughout each transfer. Wait for completion before
  brightness/sleep commands; force a complete redraw after waking the panel.
- Hold the SDK UI high-speed run vote while the panel is awake or resuming, so
  audio shutdown cannot lower the render/DMA clock. Release it after panel sleep
  to permit 48MHz run mode when audio is idle. Wake resets only the panel,
  restores its registers and QSPI divider, and redraws before Display On.
- Fill gradients in repeating four-pixel RGB565 blocks instead of converting and
  clipping each pixel separately. The ordered dither pattern remains unchanged.
- Cull circles, outlines, lines and whole controls against both axes before
  rasterization, including neighbors that touch the rounded damage boundary.

A single 96px key and its glow require 11,448 pixels, about 5% of a 230,400-pixel
frame. Multiple controls on the same band share a bounding rectangle. This
conservative union can include unchanged pixels, but bounds memory and DMA jobs.
The second buffer costs approximately 45KB of SRAM; it remains in `.bss`.

The 30s dim / 120s sleep policy and 80ms active / 200ms dim lighting cadence
remain in effect. Static states generate no repeated display transfers. The
Embassy target shares the renderer and signatures and keeps one band buffer.
It clips drawing to each damage rectangle, rotates that rectangle, then packs
its rows in place before DMA: the pinned HAL uses the window width as its
source pitch, unlike the native driver's 480-pixel stride. Panel window and
LCDC ROI use the same transformed rectangle. Transfer failures invalidate the
retained signature and force a complete repair on the next render.

### Flappy Bird refresh

Flappy Bird previously invalidated the whole screen on every simulation
revision. Embassy also expanded partial damage to whole 48-row bands and
waited another 33 ms after rendering. Together these increased transfer work
and made rendering delays accumulate into uneven motion.

Flappy now registers fixed signature slots for the bird, each pipe's upper
and lower sections, and the score HUD. Both old and new bounds are damaged,
including caps, highlights and the HUD shadow. Phase transitions still force
a complete frame. Embassy sleeps only for the remaining time until the next
20 ms simulation step, accounting for rendering time and early input events.

The regression reproducer went from 230,400 to 23,584 transferred pixels for
one moving pipe and bird (89.8% less). A 360-frame traversal with multiple
pipes, recycled pipes and skipped simulation steps averages 111,819 pixels
(51.5% less). These are exact transfer-area measurements from the shared
renderer, **not on-device FPS measurements**. Every partial frame is compared
byte for byte with a full redraw, including both the native strided transfer
and Embassy's packed transfer, on upright and rotated boards. Tests also cover
score digit changes, pause, game over and return to the menu. TE synchronization
remains disabled; visual smoothness and residual tearing require device review.

### Agent Command Deck faces

The deck faces register their own fixed regions (roster rows, the toast lane,
the LISTENING waveform box, single macro keys and the like), so a typical
update is one or two lanes rather than the screen. Ages repaint at most once a
minute and the waveform only while a PTT session is live. The strip position is
part of the frame mode, so a face slide redraws the whole panel on every step
of its ~220 ms ease.

Their render cost on the device is **unmeasured**: the text-heavy faces blit
4-bit glyph atlases, and no flashed build has reported their `display:`
timings yet. In a one-off host benchmark during review (optimized test build,
not kept in the repo), a full frame of any deck face took 0.03–0.15 ms, against
0.43 ms for the gradient-heavy Codex keyboard face the firmware had then (since
removed). Host timings do not transfer to the M33. Slide smoothness, text shimmer and toast
motion need the on-device trial in the
[first-flash checklist](deck-device-checklist.md).

## Validation

### Screen mounting and animation

The mounting is a per-board build setting in `src/display/orientation.rs`: the
default LilyGo development board is upright, and the Rev A and Rev B hardware
(`--features hw-a` or `hw-b`, or `OWT_BOARD=hw-a|hw-b tools/build-native.sh`)
turn 180°. The
rest of this section describes the 180° build; the upright build skips the
reversal and sends logical rectangles unchanged. `tools/test-all.sh` runs every
host regression for both mountings.

The UI keeps its logical coordinates. After drawing,
the firmware reverses RGB565 pixel pairs within each damaged rectangle, maps its
destination to the opposite panel corner, and retains the source offset and
480-pixel stride. Logical bands run bottom first so physical panel writes still
visit native rows in ascending order. The splash and full wake redraw use the
same orientation, and raw touch reports are transformed before gesture tracking.
No additional framebuffer is allocated; the native transfer still owns its
buffer until DMA completes.

This keeps the panel's scan configuration unchanged and avoids the SDK's LCDC
layer rotation: the pinned HAL accepts 180° but its horizontal mirror register
code covers SF32LB56X/58X, not this SF32LB52X. Software rotation prevents partial
updates from mixing logical and physical coordinates. It does not synchronize
the writes to panel scanout: LCDC TE synchronization remains disabled, so visible
tearing still needs an on-device animation trial.

Host regressions compare rotated partial updates with an independently rotated
complete image across key/glow, lighting, icon/layout, battery, D-pad and dial
changes. They also exercise cropped rectangles within a band, pixel byte order,
physical transfer order, splash, wake redraw, touch corners and raw-report HID
press/release mapping. On 2026-09-16, all 87 native core and 68 shared host tests
passed, as did the C panel wake check and both release builds. The native ELF
passed application partition and SRAM clock-helper checks. It was then flashed
with verification to the connected board. Startup reached the splash and UI at
48MHz QSPI, with charger/keypad/touch initialized and encrypted HID, HFP and A2DP
reconnected. The first full redraw reported 72ms total and 69ms drawing; a
75,472-pixel partial redraw reported 27ms total and 25ms drawing. These are single
startup observations, not sustained animation measurements. The 20-second boot
log contained no display DMA error, timeout, assertion or fatal error.

No board needs turning over for its own image: the default image is upright on
the LilyGo board, and an `hw-a` image is upright on the Rev A hardware. Only an
`hw-a` image run on the LilyGo board, to trial the rotated path before Rev A
exists, needs the dev board held upside down. On whichever board is under test,
check corner keys and swipes, run animated lighting while turning the dial, then
let the display sleep and check the first wake frame. Inspect for stale blocks,
reversed touch directions and visible tear lines. Sustained animation performance
and visual results for both mountings still require a hardware trial; the
[first-flash checklist](deck-device-checklist.md) lists the steps.

### Earlier hardware measurements

Measured on the connected T-Display SF32 on 2026-09-10:

| Startup UI redraw | Total elapsed | Drawing calls |
| --- | ---: | ---: |
| Baseline | 116ms | 47ms |
| Partial/DMA/block-fill/bounds-check implementation | 48ms | 43ms |

The baseline additionally spent 20ms in synchronous transfer and approximately
49ms in scheduling/input overhead. The optimized full redraw took 59% less time
(2.42× faster), with transfers overlapping drawing. These are wall-clock
measurements including interrupts, rather than a sustained animation FPS
benchmark. Single-key partial redraws of 11,448 pixels measured 4–5ms total,
including 4ms in drawing calls. This measures rendering/transfer after a snapshot
is taken, not end-to-end touch-controller-to-host latency.

After flashing, the log confirmed 48MHz QSPI, successful DMA completion, an
encrypted HID connection, and connected A2DP playback. No display timeout or
assertion appeared during startup.
An idle screen-off window reported zero updates, zero transferred pixels and
zero DMA time while charger watchdog feeds and host messages continued. Physical
touch press/release pairs were logged successfully; a physical wake gesture still
requires user confirmation.

Host regression tests compare partial updates byte-for-byte with complete renders
through key presses/releases, glow expiration, lighting, all icon IDs, microphone
layout changes, battery states, D-pad directions, and knob rotation/presses. They
also check coordinate alignment, full-panel wake damage, distinct DMA buffers,
and stable frame snapshots while new input arrives. The optimized row-fill
preview is byte-identical to the earlier renderer.

The native startup log reports full-frame drawing time and total elapsed time.
Every 30 seconds, `display:` diagnostics report update count, accumulated frame
and drawing time, transferred pixel count, and DMA time. DMA and CPU intervals
overlap; adding them does not give frame latency.

The nonblocking panel wake sequence has an independent host check:

```sh
cc -std=c11 -Wall -Wextra -Werror tests/panel.c -o /tmp/openwalkietalkie-panel-test
/tmp/openwalkietalkie-panel-test
```

`owt_wake_display` on the debug UART tests panel recovery without sending any
input to the host. Follow it with physical touch/keypad wake testing; a successful
UART wake does not prove that input hardware can wake the display.
