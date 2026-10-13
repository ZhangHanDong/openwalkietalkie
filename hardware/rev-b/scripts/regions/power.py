# SPDX-License-Identifier: MIT
"""Rev B placement region module. apply(b, P, P2) places this region's parts.
P(ref, side, x, y, rot); P2(ref, side, x, y, toward[, rots]) -- see placelib.py.

Routing intent this placement leaves room for:
  U3 north  row-1 caps' supply pads sit over pins 6 / 10; pins 6+8 joined on the pad row.
            GND vias above each cap GND pad (v ~32.1-32.6). Feed vias from F: IO_3V3 in the
            C54|C19 slot (u ~34.1), PANEL_4V above C18's 4V pad (u ~35.6, v ~32.6).
            Pins 12-22 get vias in the 0.8 mm channel between cap pads and U3 pads (v ~34.65).
  corridor  u 30.9-31.95 between the pull-up column and C20 is kept free for touch/I2C from
            the MCU: pins 2/4 enter through the channel under C20, pins 3/5 pass U3's west
            end into the lane. PANEL_LOGIC leaves C21 below those (v ~39.6) to pins 9/11.
  regulators  CIN/COUT GND pads connect straight into the exposed pad (no pins on the IC's
            west/east edges); VIN/VOUT leave the corner pins sideways; FB sense runs up the
            gap between the two output caps; outputs + SYSTEM_SUPPLY change to F via vias.
"""


def apply(b, P, P2):
    # ================================================================ display connector + its parts (B)

    P('U3', 'B', 36.6, 36.75, 0)
    # supply decoupling: one row tight against U3's north courtyard, supply pad down at the pin
    P('C20', 'B', 32.45, 33.35, 90)    # IO_3V3 (pins 6/8)
    P('C19', 'B', 34.75, 33.35, 90)    # PANEL_4V pin 10
    P('C18', 'B', 36.75, 33.55, 0)     # PANEL_4V bulk, 4V pad west
    P('R30', 'B', 45.3, 22.95, 90)   # PANEL_RESET pull-down next to the reset escape via (Rev B routing)
    # touch / I2C pull-ups: column west of U3, signal pads east toward the U3 west-end corridor
    P('R41', 'B', 29.95, 34.35, 0)     # TOUCH_IRQ (pin 2)
    P('R28', 'B', 29.95, 35.40, 0)     # I2C_SDA (pin 4)
    P('R29', 'B', 29.95, 36.45, 0)     # I2C_SCL (pin 5)
    P('R45', 'B', 29.95, 37.50, 180)   # TOUCH_RESET (pin 3) pull-down
    P('C21', 'B', 29.40, 38.70, 180)   # PANEL_LOGIC (pins 9/11)

    # ================================================================ power block (B, left of U3, under the bare panel)

    # U8 PANEL_4V buck-boost: power row south (VIN W, VOUT E), L5 below, CIN west, COUT east;
    # the CIN/COUT GND pads sit level with the exposed pad; FB divider + EN pull-down north
    P('U8', 'B', 22.0, 34.5, 90)
    P('L5', 'B', 22.0, 37.5, 0)
    P('C45', 'B', 19.05, 35.5, 90)
    P('C46', 'B', 24.95, 35.5, 90)
    P('C47', 'B', 27.40, 35.5, 90)
    P('R34', 'B', 20.60, 31.6, 180)
    P('R31', 'B', 23.45, 31.6, 180)
    P('R32', 'B', 25.40, 31.6, 180)

    # U7 IO_3V3 buck-boost (same arrangement), offset west so its FB divider is clear of L5
    P('U7', 'B', 15.0, 42.5, 90)
    P('L4', 'B', 15.0, 45.5, 0)
    P('C42', 'B', 12.05, 43.5, 90)
    P('C43', 'B', 17.95, 43.5, 90)
    P('C44', 'B', 20.40, 43.5, 90)
    P('R43', 'B', 16.45, 39.6, 180)
    P('R42', 'B', 18.40, 39.6, 180)

    # U10 TPS22918 IO_3V3 -> PANEL_LOGIC: output row north (CT W, QOD, VOUT E), input row south
    P('U10', 'B', 26.0, 43.0, 90)
    P('R35', 'B', 26.50, 40.35, 180)
    P('C50', 'B', 24.45, 40.35, 180)
    P('C49', 'B', 26.50, 46.00, 180)   # IO pad under VIN, GND pad under GND
    P('R36', 'B', 23.45, 44.30, 180)
    # R48: fitted 0R between IO_3V3 and U3 pin 6 (TP_VCC) so the tie can be undone (review)
    P2('R48', 'B', 33.5, 33.35, 'U3.6', (90, 270))
