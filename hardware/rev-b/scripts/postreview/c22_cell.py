# SPDX-License-Identifier: MIT
# KiCad python. C22 straight to U4 pin 8; old C22 stub/via to the MCU_LDO1 star removed; CELL re-routed.
import pcbnew, routelib
b = pcbnew.LoadBoard('IN.kicad_pcb')
# deleted (MCU_LDO1): B (36.45,17.69)-(35.48,18.66)-(35.48,19.47)-(35.98,19.97), via (35.98,19.97),
#   F (35.98,19.97)-(35.98,20.67)-(36.23,20.92)-(36.78,20.92)-(37.25,20.45)
# deleted (CELL): the B run from the B0 via (36.85,19.6) to the J3 via (15.75,15.6)
R = routelib.Router(b)
c22 = R.pad('C22', '1'); u48 = R.pad('U4', '8')
R.path('MCU_LDO1', 'B', [c22, (c22[0], u48[1])], 0.25)
pcbnew.SaveBoard('OUT.kicad_pcb', b)
# then: maze.py OUT.kicad_pcb OUT2.kicad_pcb --nets CELL --via-cost 120 --layers F,B
