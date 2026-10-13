# Audio crosstalk check: hardware/rev-b/main/main-revb.kicad_pcb

Copper stack: F.Cu / In1.Cu / In2.Cu / B.Cu; GND fills on: B.Cu, F.Cu, In1.Cu, In2.Cu
Dielectric between layers: F.Cu-In1.Cu 0.0994 mm (er 4.1), In1.Cu-In2.Cu 0.7000 mm (er 4.6), In2.Cu-B.Cu 0.0994 mm (er 4.1)

**0 ERROR, 4 WARN, 15 INFO**

| sev | rule | at (u,v) | finding |
|---|---|---|---|
| WARN | PAIR |  | AC_P/AC_N not routed together: P-to-N separation p95 1.13 mm, max 1.13 mm (limit p95 1.0) |
| WARN | PAIR |  | AMP_VOP/AMP_VON not routed together: P-to-N separation p95 2.81 mm, max 2.85 mm (limit p95 1.0) |
| WARN | PAIR |  | SPEAKER_PLUS/SPEAKER_MINUS not routed together: P-to-N separation p95 2.27 mm, max 3.11 mm (limit p95 1.0) |
| WARN | SUPPLY |  | AVDD33_AUD (U5 pin 38) shares net MCU_LDO1 with flash U4 VCC (pin 8): flash read current ripples the audio supply |
| INFO | BUDGET |  | output-referred coupling sum(C_est x Zt) to fast/switching nets = 0.01 fF*Mohm (limit 10.0; r6 = 307.8, i.e. 89.8 dB vs r6) |
| INFO | XT-SHLD | (45.75, 30.49) | DAC_FILTER_N (S1_analog) vs BT_RF_LINE (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.30 |
| INFO | XT-SHLD | (45.75, 35.70) | DAC_FILTER_N (S1_analog) vs PANEL_D1 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.37 |
| INFO | XT-SHLD | (45.75, 35.15) | DAC_FILTER_N (S1_analog) vs PANEL_D3 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.37 |
| INFO | XT-SHLD | (45.75, 26.96) | DAC_FILTER_N (S1_analog) vs PANEL_D1 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.37 |
| INFO | XT-SHLD | (45.75, 25.95) | DAC_FILTER_N (S1_analog) vs PANEL_D3 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.40 |
| INFO | XT-SHLD | (40.26, 64.02) | DAC_FILTER_N (S1_analog) vs MIC_CLOCK (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.39 |
| INFO | XT-SHLD | (42.30, 61.30) | DAC_FILTER_N (S1_analog) vs MIC_PDM_DATA (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.27 |
| INFO | XT-SHLD | (46.10, 30.50) | DAC_FILTER_P (S1_analog) vs BT_RF_LINE (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.30 |
| INFO | XT-SHLD | (42.55, 61.55) | DAC_FILTER_P (S1_analog) vs MIC_PDM_DATA (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.27 |
| INFO | XT-SHLD | (41.70, 64.30) | DAC_FILTER_P (S1_analog) vs MIC_CLOCK (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.68 |
| INFO | XT-SHLD | (46.10, 27.30) | DAC_FILTER_P (S1_analog) vs PANEL_D1 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.40 |
| INFO | XT-SHLD | (46.10, 35.35) | DAC_FILTER_P (S1_analog) vs PANEL_D1 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.40 |
| INFO | XT-SHLD | (46.10, 26.30) | DAC_FILTER_P (S1_analog) vs PANEL_D3 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.40 |
| INFO | XT-SHLD | (46.10, 34.80) | DAC_FILTER_P (S1_analog) vs PANEL_D3 (A_fast): crosses F.Cu/B.Cu through GND fill; overlap length 0.40 |

## Summing node
- AMP_INP: {'len_mm': 1.51, 'vias': 0, 'layers': ['F.Cu'], 'pads': ['R4.2', 'U1.3'], 'rin_to_pin_mm': 1.43}
- AMP_INN: {'len_mm': 1.51, 'vias': 0, 'layers': ['F.Cu'], 'pads': ['U1.4', 'R5.2'], 'rin_to_pin_mm': 1.43}

## Estimated coupling (2D-table estimate; C x Zt = output-referred weight)
weighted total to fast/switching nets: 0.01 fF*Mohm; improvement vs r6: 89.8 dB

| victim | aggressor | relation | min gap | len<0.2 | len<0.5 | len<1.0 | pad ovl mm2 | C_est fF | Zt kohm | C*Zt fF*Mohm | where |
|---|---|---|---|---|---|---|---|---|---|---|---|
| DAC_FILTER_N | PANEL_RESET | same:F.Cu | 0.500 | 0.00 | 0.00 | 2.05 | 0.0000 | 0.7 | 10.4 | 0.007 | u45.8-45.8 v23.1-25.2 |
| MCU_LDO1 | DEBUG_TX_SWCLK | unshielded:In1.Cu/F.Cu | -0.176 | 0.97 | 1.69 | 2.84 | 0.0000 | 81.6 | 0.0 | 0.000 | u37.3-39.8 v20.5-21.2 |
| MCU_LDO1 | DEBUG_RX_SWDIO | unshielded:In1.Cu/F.Cu | -0.172 | 0.75 | 1.35 | 2.35 | 0.0000 | 68.6 | 0.0 | 0.000 | u38.1-40.4 v21.2-21.2 |
| MCU_LDO1 | ENC_A | unshielded:In1.Cu/F.Cu | -0.172 | 0.75 | 1.35 | 2.42 | 0.0000 | 68.4 | 0.0 | 0.000 | u39.0-41.4 v21.2-21.2 |
| MCU_LDO1 | DEBUG_TX_SWCLK | same:In1.Cu | 0.400 | 0.00 | 0.80 | 2.64 | 0.0000 | 22.1 | 0.0 | 0.000 | u37.5-40.0 v20.7-21.2 |
| MCU_LDO1 | FLASH_IO0 | unshielded:In1.Cu/F.Cu | 0.565 | 0.00 | 0.00 | 2.02 | 0.0000 | 21.5 | 0.0 | 0.000 | u39.5-41.5 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO1 | unshielded:In1.Cu/F.Cu | 0.565 | 0.00 | 0.00 | 1.95 | 0.0000 | 20.7 | 0.0 | 0.000 | u40.3-42.3 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO0 | same:In1.Cu | 0.400 | 0.00 | 0.80 | 2.25 | 0.0000 | 16.1 | 0.0 | 0.000 | u39.3-41.6 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO1 | same:In1.Cu | 0.400 | 0.00 | 0.80 | 2.25 | 0.0000 | 10.9 | 0.0 | 0.000 | u40.1-42.4 v21.2-21.2 |
| ADC_REF | USB_MCU_P | unshielded:B.Cu/In2.Cu | 0.570 | 0.00 | 0.00 | 0.22 | 0.0000 | 6.1 | 0.0 | 0.000 | u34.7-35.3 v28.7-29.2 |
| MCU_LDO1 | FLASH_IO2 | unshielded:In1.Cu/F.Cu | 1.415 | 0.00 | 0.00 | 0.00 | 0.0000 | 4.3 | 0.0 | 0.000 |  |
| MCU_LDO1 | FLASH_CLK | unshielded:In1.Cu/F.Cu | 1.415 | 0.00 | 0.00 | 0.00 | 0.0000 | 4.3 | 0.0 | 0.000 |  |
| MCU_LDO1 | DEBUG_RX_SWDIO | same:In1.Cu | 1.250 | 0.00 | 0.00 | 0.00 | 0.0000 | 4.3 | 0.0 | 0.000 |  |
| ADC_REF | USB_MCU_N | unshielded:B.Cu/In2.Cu | 1.120 | 0.00 | 0.00 | 0.00 | 0.0000 | 3.7 | 0.0 | 0.000 |  |
| MCU_LDO1 | FLASH_CLK | same:In1.Cu | 1.250 | 0.00 | 0.00 | 0.00 | 0.0000 | 3.0 | 0.0 | 0.000 |  |
| MCU_LDO1 | FLASH_IO2 | same:In1.Cu | 1.250 | 0.00 | 0.00 | 0.00 | 0.0000 | 2.0 | 0.0 | 0.000 |  |
| MCU_LDO1 | FLASH_IO3 | same:B.Cu | 0.284 | 0.00 | 0.84 | 1.50 | 0.0000 | 1.8 | 0.0 | 0.000 | u35.4-37.3 v15.6-16.4 |
| MCU_LDO1 | ENC_A | same:F.Cu | 0.103 | 0.00 | 0.39 | 1.03 | 0.0000 | 1.2 | 0.0 | 0.000 | u36.7-37.3 v15.6-16.4 |
| AUDIO_REF | PANEL_RESET | same:B.Cu | 0.265 | 0.00 | 0.57 | 1.50 | 0.0000 | 1.1 | 0.0 | 0.000 | u42.0-43.5 v23.0-23.6 |
| AUDIO_REF | PANEL_RESET | same:F.Cu | 0.365 | 0.00 | 0.00 | 0.45 | 0.0000 | 0.1 | 0.0 | 0.000 | u43.1-43.7 v23.2-23.6 |
| MCU_LDO1 | ENC_A | same:B.Cu | 0.217 | 0.00 | 0.00 | 0.59 | 0.0000 | 0.0 | 0.0 | 0.000 | u35.4-37.3 v15.6-16.4 |
| AUDIO_REF | PANEL_RESET | shielded:F.Cu/B.Cu | 0.402 | 0.00 | 0.12 | 0.45 | 0.0000 | 0.0 | 0.0 | 0.000 | u43.1-43.7 v23.1-23.6 |
| MCU_LDO1 | DEBUG_TX_SWCLK | shielded:B.Cu/F.Cu | 0.037 | 0.00 | 0.36 | 0.87 | 0.0000 | 0.0 | 0.0 | 0.000 | u37.2-37.7 v20.5-21.5 |
| MCU_LDO1 | DEBUG_TX_SWCLK | shielded:In1.Cu/B.Cu | -0.237 | 0.90 | 1.50 | 2.72 | 0.0000 | 0.0 | 0.0 | 0.000 | u37.7-40.2 v20.9-21.2 |
| MCU_LDO1 | DEBUG_RX_SWDIO | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u38.1-40.6 v21.2-21.2 |
| MCU_LDO1 | ENC_A | shielded:B.Cu/F.Cu | 0.228 | 0.00 | 1.06 | 1.65 | 0.0000 | 0.0 | 0.0 | 0.000 | u35.4-37.3 v15.6-16.4 |
| MCU_LDO1 | FLASH_CS | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u40.2-42.7 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO1 | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u39.9-42.3 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO2 | shielded:In1.Cu/B.Cu | -0.237 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u39.5-42.0 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO0 | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u39.2-41.6 v21.2-21.2 |
| MCU_LDO1 | FLASH_CLK | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u38.8-41.3 v21.2-21.2 |
| MCU_LDO1 | FLASH_IO3 | shielded:F.Cu/B.Cu | 0.324 | 0.00 | 0.12 | 2.12 | 0.0000 | 0.0 | 0.0 | 0.000 | u36.7-37.5 v15.6-18.6 |
| MCU_LDO1 | FLASH_IO3 | shielded:In1.Cu/B.Cu | -0.238 | 0.90 | 1.50 | 2.50 | 0.0000 | 0.0 | 0.0 | 0.000 | u38.5-40.9 v21.2-21.2 |
| DAC_FILTER_P | BT_RF_LINE | shielded:F.Cu/B.Cu | -0.143 | 0.70 | 1.30 | 2.30 | 0.0000 | 0.0 | 10.4 | 0.000 | u46.1-46.1 v29.4-31.6 |
| DAC_FILTER_P | MIC_CLOCK | shielded:F.Cu/B.Cu | -0.131 | 1.25 | 1.91 | 2.48 | 0.0000 | 0.0 | 10.4 | 0.000 | u41.0-42.1 v62.9-65.4 |
| DAC_FILTER_P | MIC_PDM_DATA | shielded:F.Cu/B.Cu | -0.132 | 0.67 | 1.27 | 2.27 | 0.0000 | 0.0 | 10.4 | 0.000 | u41.8-43.3 v60.8-62.3 |
| DAC_FILTER_P | PANEL_RESET | shielded:F.Cu/B.Cu | 0.405 | 0.00 | 1.15 | 2.31 | 0.0000 | 0.0 | 10.4 | 0.000 | u46.1-46.5 v22.6-24.7 |
| DAC_FILTER_P | PANEL_D1 | shielded:F.Cu/B.Cu | -0.126 | 1.90 | 3.60 | 6.40 | 0.0000 | 0.0 | 10.4 | 0.000 | u46.1-46.1 v25.7-36.9 |
| DAC_FILTER_P | PANEL_D3 | shielded:F.Cu/B.Cu | -0.126 | 1.90 | 3.60 | 6.40 | 0.0000 | 0.0 | 10.4 | 0.000 | u46.1-46.1 v24.7-36.4 |
| DAC_FILTER_N | BT_RF_LINE | shielded:F.Cu/B.Cu | -0.148 | 0.70 | 1.30 | 2.30 | 0.0000 | 0.0 | 10.4 | 0.000 | u45.8-45.8 v29.4-31.6 |
| DAC_FILTER_N | MIC_CLOCK | shielded:F.Cu/B.Cu | -0.127 | 0.88 | 1.54 | 2.43 | 0.0000 | 0.0 | 10.4 | 0.000 | u39.6-40.7 v62.9-65.1 |
| DAC_FILTER_N | MIC_PDM_DATA | shielded:F.Cu/B.Cu | -0.127 | 0.67 | 1.27 | 2.27 | 0.0000 | 0.0 | 10.4 | 0.000 | u41.5-43.1 v60.5-62.1 |
| DAC_FILTER_N | PANEL_RESET | shielded:F.Cu/B.Cu | 0.055 | 1.02 | 1.67 | 2.82 | 0.0000 | 0.0 | 10.4 | 0.000 | u45.8-45.8 v22.1-24.9 |
| DAC_FILTER_N | PANEL_D1 | shielded:F.Cu/B.Cu | -0.132 | 1.90 | 3.60 | 6.42 | 0.0000 | 0.0 | 10.4 | 0.000 | u45.8-45.8 v25.4-37.3 |
| DAC_FILTER_N | PANEL_D3 | shielded:F.Cu/B.Cu | -0.132 | 1.90 | 3.60 | 6.42 | 0.0000 | 0.0 | 10.4 | 0.000 | u45.8-45.8 v24.4-36.8 |

Totals C_est (fF) per victim / aggressor class: {"DAC_FILTER_P": {"A_fast": 1.0, "A_slow": 0.1}, "DAC_FILTER_N": {"A_slow": 0.7, "A_fast": 0.0}, "MCU_LDO1": {"A_slow": 246.3, "A_fast": 84.7, "A_power": 0.0}, "ADC_REF": {"A_fast": 9.9, "A_slow": 0.0}, "AUDIO_REF": {"A_slow": 1.3, "A_fast": 0.1}, "DAC_N": {"A_fast": 0.0}, "DAC_P": {"A_fast": 0.0}}

## Reference plane coverage
- AUDIO_REF: {'len': 1.96, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '0.45/0.45 mm', 'B.Cu': '1.50/1.50 mm'}}
- AC_P: {'len': 1.32, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '1.32/1.32 mm'}}
- AMP_INP: {'len': 1.51, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '1.51/1.51 mm'}}
- AMP_BYPASS: {'len': 2.08, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '2.08/2.08 mm'}}
- AMP_INN: {'len': 1.51, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '1.51/1.51 mm'}}
- AC_N: {'len': 1.32, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '1.32/1.32 mm'}}
- DAC_FILTER_P: {'len': 47.75, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '45.14/45.14 mm', 'B.Cu': '2.61/2.61 mm'}}
- DAC_FILTER_N: {'len': 51.29, 'ref_frac': 1.0, 'per_layer': {'F.Cu': '48.19/48.19 mm', 'B.Cu': '3.11/3.11 mm'}}
- DAC_N: {'len': 3.16, 'ref_frac': 1.0, 'per_layer': {'B.Cu': '3.16/3.16 mm'}}
- ADC_REF: {'len': 1.9, 'ref_frac': 1.0, 'per_layer': {'B.Cu': '1.90/1.90 mm'}}
- DAC_P: {'len': 2.37, 'ref_frac': 1.0, 'per_layer': {'B.Cu': '2.37/2.37 mm'}}

## Pairs
- DAC_P/DAC_N: {'len_p': 2.37, 'len_n': 3.16, 'layers_p': ['B.Cu'], 'layers_n': ['B.Cu'], 'vias_p': 0, 'vias_n': 0, 'sep_p95': 0.54, 'sep_max': 0.6}
- DAC_FILTER_P/DAC_FILTER_N: {'len_p': 47.75, 'len_n': 51.29, 'layers_p': ['B.Cu', 'F.Cu'], 'layers_n': ['B.Cu', 'F.Cu'], 'vias_p': 1, 'vias_n': 1, 'sep_p95': 0.7, 'sep_max': 1.15}
- AC_P/AC_N: {'len_p': 1.32, 'len_n': 1.32, 'layers_p': ['F.Cu'], 'layers_n': ['F.Cu'], 'vias_p': 0, 'vias_n': 0, 'sep_p95': 1.13, 'sep_max': 1.13}
- AMP_INP/AMP_INN: {'len_p': 1.51, 'len_n': 1.51, 'layers_p': ['F.Cu'], 'layers_n': ['F.Cu'], 'vias_p': 0, 'vias_n': 0, 'sep_p95': 0.78, 'sep_max': 0.78}
- AMP_VOP/AMP_VON: {'len_p': 3.74, 'len_n': 3.74, 'layers_p': ['F.Cu'], 'layers_n': ['F.Cu'], 'vias_p': 0, 'vias_n': 0, 'sep_p95': 2.81, 'sep_max': 2.85}
- SPEAKER_PLUS/SPEAKER_MINUS: {'len_p': 16.89, 'len_n': 15.59, 'layers_p': ['F.Cu'], 'layers_n': ['F.Cu'], 'vias_p': 0, 'vias_n': 0, 'sep_p95': 2.27, 'sep_max': 3.11}

## Supply
- AVDD33_AUD: {'pin': '38', 'net': 'MCU_LDO1', 'nearest_cap': {'ref': 'C14', 'value': '2.2uF', 'dist_mm': 1.06, 'side': 'B.Cu'}}
- AUD_VREF: {'pin': '39', 'net': 'AUDIO_REF', 'nearest_cap': {'ref': 'C15', 'value': '1uF', 'dist_mm': 1.09, 'side': 'F.Cu'}}

## Class-D outputs
- AMP_VON: {'pin': '5', 'len_mm': 3.74, 'to': [{'ref': 'L2', 'dist': 3.5}]}
- AMP_VOP: {'pin': '8', 'len_mm': 3.74, 'to': [{'ref': 'L1', 'dist': 3.5}]}
- orientation: {'rin_to_in_pins': 1.45, 'rin_to_out_pins': 5.88}

## Amplifier power
- vdd_decap: {'ref': 'C11', 'value': '1uF', 'dist_mm': 1.8, 'side': 'F.Cu'}
- gnd_via_mm: 0.78
