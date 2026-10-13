# Approved charger package migration: SGM41562S to SGM41562SA

Status: the package migration is implemented in the enclosing BodyV5 r2 routed PCB release, which has a matching checked source snapshot and native KiCad DRC report. Earlier migration audit hashes describe the source-migration stage only. Firmware was not modified and still requires the compatibility change below before charging can operate.

## Firmware release gate

This is not a binary drop-in substitution. SGM41562SA reports `REG0x0B = 0x00`; SGM41562S reports `0x09`. The current `src/battery/charger.rs::initialize` accepts only `0x09`, and its unknown/legacy-device test explicitly rejects `0x00`. It will therefore refuse to configure or enable charging on the replacement. Implement board-selected SA support and its tests in the firmware workstream; do not simply accept `0x00` globally because that weakens the existing legacy-device guard. Firmware is unchanged in this handoff.

SG Micro's July 2026 S and SA register tables, pp. 20-26, have identical writable fields, reset values, and encodings. Their only register-table difference is that read-only identity byte. I2C remains address `0x03`. Configuration must still set the battery-specific charge limits and enable charging last.

## Preserved electrical and recovery behavior

- The same IN/SYS/BAT power-path topology, 500 mA input-limit reset, 4.55 V SYS target, and 2 A battery-discharge-limit reset are retained. Charging remains disabled at reset (`CEB=1`); firmware must enable it.
- Existing input, battery and SYS capacitors are 4.7 uF nominal; vendor minimums are 2.2 uF on IN/BAT and greater than 2.2 uF on SYS. VDD retains 100 nF. Effective capacitance at bias and close, low-inductance placement must still be checked.
- Battery NTC input, its remote cell sensor/divider, and independent I2C supply are unchanged. Existing provisional cell-temperature limits still need qualification against the selected battery.
- `r_nint` remains 100 kohm to BAT, with `tp_nint` on the same net. The fixture must only pull this BAT-domain signal low; do not drive it from a 3.3 V push-pull output. It is not wired to the MCU. Status remains I2C-polled.
- Hardware nINT reset remains a default 16-second low followed by a 4-second power interruption. Shipping-mode exit takes 2 seconds via nINT-low or 100 ms after valid IN detection. nINT wake requires the specified default EN_SHIP_DGL setting. If INT_DIS is set because nINT was grounded or floating before shipping mode, valid IN is the only wake method (pp. 18-19). No nINT feature was removed to ease routing.
- Valid USB input events can force the charger watchdog to 64 seconds until the host writes the device (p. 13). This existing behavior survives the substitution: blank/wrong-firmware flashing must tolerate or explicitly manage a power recycle. This source change alone does not establish unlimited uninterrupted flashing time or validate the complete UART/SWD recovery sequence.

The SA has slightly higher typical switch resistances: IN-to-SYS 175 mOhm versus 155 mOhm; BAT-to-SYS 73 mOhm versus 70 mOhm. Published package theta-JA is 74.7 C/W versus 94.4 C/W, but these test-board figures are not a thermal qualification of this enclosure. The applicable current/voltage limits remain the same. High-current layout and thermal testing remain required.

## Exact part and land

Order `SGM41562SAXTYH10G/TR`, TQFN-2x2-10DL. Package body maximum is 2.1 x 2.1 mm and height maximum 0.8 mm. The former WLCSP body was at most 1.58 x 1.60 mm and 0.6 mm high. Recheck placement, component clearance, battery envelope and assembly supply; availability was not verified.

The footprint follows the component-side top view on datasheet p. 3 and land drawing TX00663.000 on p. 33. Copper envelope is 2.6 x 2.6 mm. The added 3.1 x 3.1 mm courtyard is this board's 0.25 mm assembly allowance, not a vendor copper dimension.

| Pin | Function / board net | Local center mm | Land size mm | Rotation |
| --- | --- | --- | --- | --- |
| 1 | IN / USB_VBUS | -0.925, -0.75 | 0.75 x 0.25 | 0 |
| 2 | NTC / power::CELL_TEMPERATURE | -0.95, -0.25 | 0.70 x 0.25 | 0 |
| 3 | NINT / power::CHG_NINT_BAT_DOMAIN | -0.95, 0.25 | 0.70 x 0.25 | 0 |
| 4 | VDD / power::CHG_VDD | -0.95, 0.75 | 0.70 x 0.25 | 0 |
| 5 | GND | 0, 0.95 | 0.70 x 0.25 | 90 |
| 6 | SCL / I2C_SCL | 0.95, 0.75 | 0.70 x 0.25 | 0 |
| 7 | SDA / I2C_SDA | 0.95, 0.25 | 0.70 x 0.25 | 0 |
| 8 | BAT / CELL | 0.95, -0.25 | 0.70 x 0.25 | 0 |
| 9 | BAT / CELL | 0.95, -0.75 | 0.70 x 0.25 | 0 |
| 10 | SYS / SYSTEM_SUPPLY | 0, -0.95 | 0.70 x 0.25 | 90 |

Pin 1 is longer toward the body: its outer edge aligns with the other left-side lands. Therefore its center is 0.025 mm inward of their -0.95 mm center line. There is no center exposed pad or via-in-pad requirement. Both BAT pins are explicitly bound and must be physically connected.

The 0.50 mm row pitch minus 0.25 mm land width leaves a 0.25 mm adjacent copper gap, exceeding either nominal JLC 0.10 mm or literal 4 mil (0.1016 mm). Every pad escapes outward; no trace must pass between lands to reach a buried center ball. Stagger 0.45 mm vias: putting different-net vias directly on every 0.50 mm pitch would leave only 0.05 mm clearance. Signal escape geometry does not authorize narrow power trunks; widen IN/SYS/BAT immediately beyond the pad launch and keep all four bypass loops short. Final placement/routing must check surrounding obstacles, holes, all copper layers, and the manufacturer's fabrication rules.

## Source and verification

Input: fresh GPT-6 `outline-revised/source-ready/main`; no older board artifacts were used. Only `src/parts.cohdl`, `src/power_bindings.cohdl`, `src/power.cohdl`, and the charger comments in `src/main.cohdl` changed. U6's identity and placement are retained. Four capacitor placements were subsequently updated in source and the board for the larger courtyard: C41 at (12.25, 7.8), 90 degrees; C48 at (15, 6.6), 90 degrees; C40 at (18.4, 8.1), 0 degrees; C51 at (17.8, 10.8), 0 degrees. All are on F.Cu. Compiler output is 133 instances / 100 nets; `design.lock` and `cohdl.lock` are unchanged. All non-U6 netlist nodes are unchanged.

Primary source: [SG Micro SGM41562SA](https://www.sg-micro.com/product/SGM41562SA), July 2026 Rev. A.1, archived here as [sgm41562sa.pdf](sgm41562sa.pdf). Comparison source: [SG Micro SGM41562S](https://www.sg-micro.com/product/SGM41562S), July 2026, from the project's existing manufacturer document. Reviewed pages: 2-6, 13-19, 20-26, 31-33. The package/pin pages were visually inspected, not inferred from similar part names.
