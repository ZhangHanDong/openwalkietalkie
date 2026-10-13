# Component-model provenance

The STEP models are provided as part of this PCB design, for assembly-fit work.
`manifest.json` identifies each model's MPN, bounds, source/evidence, geometry
type and SHA-256. `registered-model-provenance.json` records the rigid datum
transforms for the switch and encoder assets already supplied with the project.

- Molex 53261-0271: KiCad packages3D model by Rene Poeschl, copyright 2018.
  Its original STEP header retains the CC BY-SA 4.0 notice and KiCad design
  exception. See the [KiCad library license](https://github.com/KiCad/kicad-packages3D/blob/master/LICENSE.md).
- PTS636: registered KiCad community Sx25 model from the project's existing
  mechanical assets. This is not manufacturer-authored CAD. A 0.05 mm seating
  correction is recorded in the provenance report.
- SKTDLDE010 and EC05E1220401: registered copies of the existing project
  manufacturer STEP assets. Original shapes are retained, with only rigid
  seating-frame transforms.
- Other models are generated dimensional envelopes, not vendor STEP files.
  Their datasheet/package evidence is recorded in the manifest. The FH34SRJ
  envelope uses Hirose drawing EDC3-159714-05; no restricted Hirose CAD was copied.

The USB model deliberately does not invent the undimensioned rounded mouth
profile. The fit implications are described in the main README.
