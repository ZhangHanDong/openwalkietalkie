# Current mechanical source and component evidence

These inputs support the BodyV5 r3 PCB release. They are not a second PCB design.
`manifest.json` records the original locations and SHA-256 of every retained input.
Third-party drawings, datasheets and CAD retain their original copyrights and
licenses; the hardware licence (CERN-OHL-P-2.0, see [`../../LICENSE`](../../LICENSE))
covers only the project's own case CAD in `case/` and does not relicense those
materials. Vendor notices and the applicable KiCad model license are retained
alongside the CAD.

- `case/BodyV5-USB-face-battery-fit.f3d`: editable Fusion case/cap assembly.
  It also contains earlier PCB/reference bodies: do not use those as current
  PCB geometry. No Fusion geometry was modified during repository cleanup.
- `case/BodyV5-USB-face-battery-fit.step`: exact source assembly whose hash is
  recorded in r3's `verification/case-fit-audit.json`. Its case/cap solids are
  the fit reference; use r3's populated board STEP files for the electronics.
- `display/H0216F002AMT003A_V2.dxf` and specification PDF: selected display
  module mechanical evidence.
- `display/FPC3-H0175Y003MT006-1_V0.dxf`: the user's selected 38 mm display flex
  drawing, reviewed by r3's `source/main/docs/display-fpc-fold.md`. This is not
  the four-pin encoder interconnect. The superseded H0216 cable drawing is not
  retained as a current cable choice.
- `vendor/buttons/` and `vendor/encoder/`: original component CAD, vendor
  notices and KiCad model license/provenance. The registered, PCB-local models
  actually attached to the boards are in r3's `models/`; do not substitute the
  unregistered originals without reproducing their datum transforms.
- `datasheets/`: retained evidence for the selected MCU, switches, encoder,
  microphone, ferrite/NTC and USB connector. The current QFN charger, display
  connector and four-pin FPC connector drawings are already included in r3.

The case assembly is an editing source, not a claim of tolerance-qualified
hardware. Read the r3 release notes for remaining fit and firmware work.
