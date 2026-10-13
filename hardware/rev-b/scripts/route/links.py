# SPDX-License-Identifier: MIT
"""Supply links the F/B escapes cannot make (hand-routed, locked).

MCU_VSYS (pins 19 and 52) and MCU_LDO1 (pins 22 and 38) sit at opposite corners of U5, and
any F/B path between them would cut through the escape fields. Both links run on In1 under
U5 (r6 ran MCU_VSYS on In2): VSYS from the north A-row via down the u 37.335 channel between
the EP via columns and east along v 28.0 to a via on the pin-52 stub; LDO1 from the A-row
via east along v 21.2 to a via beside C14. The In1 GND plane keeps flowing around both.
CELL (U5 pin 20, VBAT sense) runs on B north of the via rows and U4 to a via under J3's
body, then up into J3 pin 1.
"""


def apply(R):
    P = R.pad

    # MCU_VSYS: A-row via (36.45, 20.45) -> In1 -> via on the pin-52 stub
    R.via('MCU_VSYS', (42.75, 28.0))
    R.path('MCU_VSYS', 'In1', [(36.45, 20.45), (37.335, 21.335), (37.335, 28.0), (42.75, 28.0)], 0.25)

    # MCU_LDO1: C14.1 -> via NE of C14 -> In1 -> A-row via (37.25, 20.45)
    c14 = P('C14', 1)
    R.via('MCU_LDO1', (44.65, 21.95))
    R.path('MCU_LDO1', 'B', [c14, (c14[0], 22.51), (43.9, 21.95), (44.65, 21.95)], 0.2)
    R.path('MCU_LDO1', 'In1', [(44.65, 21.95), (43.9, 21.2), (38.0, 21.2), (37.25, 20.45)], 0.25)

    # CELL sense: B0 via (36.85, 19.6) -> B north/west -> via under J3 -> J3.1
    j31 = P('J3', 1)
    R.via('CELL', (15.75, 15.6))
    R.path('CELL', 'F', [(15.75, 15.6), j31], 0.25)
    R.path('CELL', 'B', [(36.85, 19.6), (37.05, 19.4), (37.05, 16.95), (17.1, 16.95), (15.75, 15.6)], 0.15)

    # C14.2 (B) and C15.2 (F) sit back to back with no room for a GND via beside them: one via
    # in both pads (the order specifies epoxy-filled, capped vias)
    R.via('GND', (44.4, 23.05))
