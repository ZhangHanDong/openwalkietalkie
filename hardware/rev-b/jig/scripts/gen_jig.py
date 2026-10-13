# SPDX-License-Identifier: MIT
"""Generate the Rev B UART first-flash jig board from the Rev B main board.

Run with KiCad 10's bundled python (it has `pcbnew`):

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3 \
        hardware/rev-b/jig/scripts/gen_jig.py

Every DUT position (test pads, mounting holes, outline) is read from
`../../main/main-revb.kicad_pcb`, so the jig follows the board if it moves.

Geometry: the DUT lies battery side (F) down on the jig, so seen from above
the jig, DUT x is mirrored. Jig = (OX + MIRROR_X - x_dut, OY + y_dut).
"""

import math
import os

import pcbnew as P

HERE = os.path.dirname(os.path.abspath(__file__))
JIG = os.path.dirname(HERE)
DUT = os.path.join(JIG, "..", "main", "main-revb.kicad_pcb")
OUT = os.path.join(JIG, "revb-flash-jig.kicad_pcb")

OX, OY = 50.0, 30.0  # page offset of the jig frame
MIRROR_X = 62.0  # jig x = MIRROR_X - dut x (before the page offset)

# Base plate, jig frame (mm). The DUT spans x 7.3..60.7, y 1.3..93.7 here.
BASE = (3.0, 3.0, 65.0, 98.0)  # x0, y0, x1, y1; top edge clear of the USB plug
BASE_R = 3.0

# Pogo pins: DUT test pad -> net. Only the three a first flash needs.
PINS = {"TP6": "DUT_RX", "TP4": "GND", "TP8": "DUT_TX"}
POGO_PAD = 1.5  # annular ring; TP6/TP4 are 2.0 mm apart, so 0.5 mm gap
POGO_DRILL = 0.70  # plated; the pin's tail self-centres in it

# Rests (M2x5 F-F standoff, M2x4 screw from below), DUT coordinates, chosen
# on flat F-side copper >= 3 mm from every F courtyard/pad and the edge.
RESTS = {"A": (48.5, 48.5), "B": (16.0, 27.5), "D": (40.0, 31.0), "C": (33.5, 68.5)}
# Locators: M2x10 screw from below + thin nut on top, through the DUT's NPTHs.
LOCATORS = ("MH2", "MH4")
M2_CLEAR = 2.2

HEADER_AT = (40.0, 70.0)  # jig frame, pin 1; outside the DUT outline
HEADER_ORDER = ("DUT_RX", "GND", "DUT_TX")  # pins 1..3, top to bottom

FEET = ((9.0, 9.0), (59.0, 9.0), (9.0, 92.0), (59.0, 92.0))

KICAD = "/Applications/KiCad/KiCad.app/Contents/SharedSupport"


def mm(v):
    return P.ToMM(v)


def V(x, y):
    return P.VECTOR2I(P.FromMM(x), P.FromMM(y))


def J(x, y):
    """Jig frame -> board coordinates."""
    return V(OX + x, OY + y)


def from_dut(x, y):
    """DUT coordinates -> jig frame (mirrored in x)."""
    return MIRROR_X - x, y


def read_dut():
    b = P.LoadBoard(DUT)
    pads, holes = {}, {}
    for fp in b.GetFootprints():
        ref = fp.GetReference()
        if ref.startswith("TP"):
            p = fp.Pads()[0].GetPosition()
            pads[ref] = (mm(p.x), mm(p.y))
        if ref.startswith("MH"):
            p = fp.GetPosition()
            holes[ref] = (mm(p.x), mm(p.y))
    poly = P.SHAPE_POLY_SET()
    b.GetBoardPolygonOutlines(poly, True)
    o = poly.Outline(0)
    outline = [(mm(o.CPoint(i).x), mm(o.CPoint(i).y)) for i in range(o.PointCount())]
    return pads, holes, outline


def seg(board, a, b, layer, width):
    s = P.PCB_SHAPE(board)
    s.SetShape(P.SHAPE_T_SEGMENT)
    s.SetStart(a)
    s.SetEnd(b)
    s.SetLayer(layer)
    s.SetWidth(P.FromMM(width))
    board.Add(s)


def arc(board, start, mid, end, layer, width):
    s = P.PCB_SHAPE(board)
    s.SetShape(P.SHAPE_T_ARC)
    s.SetArcGeometry(start, mid, end)
    s.SetLayer(layer)
    s.SetWidth(P.FromMM(width))
    board.Add(s)


def circle(owner, centre, radius, layer, width, filled=False):
    s = P.PCB_SHAPE(owner)
    s.SetShape(P.SHAPE_T_CIRCLE)
    s.SetCenter(centre)
    s.SetEnd(P.VECTOR2I(centre.x + P.FromMM(radius), centre.y))
    s.SetLayer(layer)
    s.SetWidth(P.FromMM(width))
    s.SetFilled(filled)
    owner.Add(s)


def text(board, msg, at, layer, size=1.0, thick=0.15, justify=None, mirror=False, angle=0):
    t = P.PCB_TEXT(board)
    t.SetText(msg)
    t.SetPosition(at)
    t.SetLayer(layer)
    t.SetTextSize(V(size, size))
    t.SetTextThickness(P.FromMM(thick))
    if justify == "left":
        t.SetHorizJustify(P.GR_TEXT_H_ALIGN_LEFT)
    elif justify == "right":
        t.SetHorizJustify(P.GR_TEXT_H_ALIGN_RIGHT)
    t.SetMirrored(mirror)
    t.SetTextAngleDegrees(angle)
    board.Add(t)


def outline(board):
    x0, y0, x1, y1 = BASE
    r = BASE_R
    c = math.cos(math.radians(45)) * r
    e = P.Edge_Cuts
    seg(board, J(x0 + r, y0), J(x1 - r, y0), e, 0.1)
    seg(board, J(x1, y0 + r), J(x1, y1 - r), e, 0.1)
    seg(board, J(x1 - r, y1), J(x0 + r, y1), e, 0.1)
    seg(board, J(x0, y1 - r), J(x0, y0 + r), e, 0.1)
    arc(board, J(x1 - r, y0), J(x1 - r + c, y0 + r - c), J(x1, y0 + r), e, 0.1)
    arc(board, J(x1, y1 - r), J(x1 - r + c, y1 - r + c), J(x1 - r, y1), e, 0.1)
    arc(board, J(x0 + r, y1), J(x0 + r - c, y1 - r + c), J(x0, y1 - r), e, 0.1)
    arc(board, J(x0, y0 + r), J(x0 + r - c, y0 + r - c), J(x0 + r, y0), e, 0.1)


def net(board, name):
    n = board.FindNet(name)
    if n is None:
        n = P.NETINFO_ITEM(board, name)
        board.Add(n)
    return n


def set_fields(fp, fields):
    """BOM fields, kept off the silkscreen."""
    for name, value in fields:
        fp.SetField(name, value)
        f = fp.GetField(name)
        f.SetLayer(P.F_Fab)
        f.SetVisible(False)


def pogo(board, ref, at, n):
    fp = P.FootprintLoad(os.path.join(JIG, "revb-jig.pretty"), "Pogo_Xinyangze_YZ105312061P-01")
    fp.SetParent(board)
    fp.SetFPIDAsString("revb-jig:Pogo_Xinyangze_YZ105312061P-01")
    fp.SetReference(ref)
    fp.SetPosition(at)
    set_fields(fp, (("MPN", "YZ105312061P-01"), ("Manufacturer", "Xinyangze")))
    for pad in fp.Pads():
        pad.SetNet(n)
    board.Add(fp)
    return fp


def stock(board, lib, name, ref, value, at, fields=()):
    fp = P.FootprintLoad(os.path.join(KICAD, "footprints", lib + ".pretty"), name)
    fp.SetParent(board)
    fp.SetReference(ref)
    fp.SetValue(value)
    fp.SetPosition(at)
    set_fields(fp, fields)
    board.Add(fp)
    return fp


def hole(board, ref, at, label, offset=(0.0, -2.4), justify=None):
    fp = stock(board, "MountingHole", "MountingHole_2.2mm_M2", ref, label, at)
    fp.Reference().SetVisible(False)
    fp.Value().SetVisible(False)
    fp.SetAttributes(fp.GetAttributes() | P.FP_EXCLUDE_FROM_BOM | P.FP_EXCLUDE_FROM_POS_FILES)
    text(board, label, V(mm(at.x) + offset[0], mm(at.y) + offset[1]), P.F_SilkS, size=0.8, thick=0.12, justify=justify)


def track(board, pts, n, layer=P.B_Cu, width=0.3):
    for a, b in zip(pts, pts[1:]):
        t = P.PCB_TRACK(board)
        t.SetStart(J(*a))
        t.SetEnd(J(*b))
        t.SetLayer(layer)
        t.SetWidth(P.FromMM(width))
        t.SetNet(n)
        board.Add(t)


def write_pogo_lib():
    """The pogo footprint as a library part, matching `pogo()`."""
    lib = os.path.join(JIG, "revb-jig.pretty")
    os.makedirs(lib, exist_ok=True)
    with open(os.path.join(lib, "Pogo_Xinyangze_YZ105312061P-01.kicad_mod"), "w") as f:
        f.write(f"""(footprint "Pogo_Xinyangze_YZ105312061P-01"
  (version 20241229)
  (generator "gen_jig.py")
  (layer "F.Cu")
  (descr "Xinyangze YZ105312061P-01 pogo pin (LCSC C5157237): 1.24 mm body, 6.1 mm long, tail in a plated hole")
  (property "Reference" "P**" (at 0 -1.6 0) (layer "F.Fab") (effects (font (size 0.8 0.8) (thickness 0.12))))
  (property "Value" "YZ105312061P-01" (at 0 1.6 0) (layer "F.Fab") (hide yes) (effects (font (size 0.8 0.8) (thickness 0.12))))
  (property "LCSC" "C5157237" (at 0 0 0) (layer "F.Fab") (hide yes) (effects (font (size 0.8 0.8) (thickness 0.12))))
  (attr through_hole)
  (fp_circle (center 0 0) (end 0.62 0) (stroke (width 0.05) (type solid)) (fill no) (layer "F.Fab"))
  (fp_circle (center 0 0) (end 0.9 0) (stroke (width 0.05) (type solid)) (fill no) (layer "F.CrtYd"))
  (pad "1" thru_hole circle (at 0 0) (size {POGO_PAD} {POGO_PAD}) (drill {POGO_DRILL}) (layers "*.Cu" "*.Mask" "F.Paste"))
  (model "${{KIPRJMOD}}/models/YZ105312061P-01.wrl" (offset (xyz 0 0 -0.6)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0)))
)
""")


def main():
    write_pogo_lib()
    pads, holes, dut_outline = read_dut()
    board = P.BOARD()
    ds = board.GetDesignSettings()
    ds.SetBoardThickness(P.FromMM(1.6))
    outline(board)

    nets = {name: net(board, name) for name in ("GND", "DUT_RX", "DUT_TX")}

    # Pogo pins under the DUT's test pads.
    jig_pin = {}
    for i, (tp, n) in enumerate(PINS.items(), 1):
        jx, jy = from_dut(*pads[tp])
        jig_pin[tp] = (jx, jy)
        pogo(board, f"P{i}", J(jx, jy), nets[n])

    # Pin labels: RX/GND to the right of the TP6/TP4 column, TX to the left.
    (rx_x, rx_y), (g_x, g_y), (tx_x, tx_y) = jig_pin["TP6"], jig_pin["TP4"], jig_pin["TP8"]
    text(board, "RX (TP6)", J(rx_x + 1.2, rx_y), P.F_SilkS, size=0.8, thick=0.12, justify="left")
    text(board, "GND (TP4)", J(g_x + 1.2, g_y), P.F_SilkS, size=0.8, thick=0.12, justify="left")
    text(board, "TX (TP8)", J(tx_x, tx_y + 1.6), P.F_SilkS, size=0.8, thick=0.12)

    # The DUT's other cluster pads, for orientation only (no pins).
    for tp in ("TP3", "TP5", "TP10"):
        jx, jy = from_dut(*pads[tp])
        circle(board, J(jx, jy), 0.5, P.F_Fab, 0.05)
        text(board, tp, J(jx, jy - 1.0), P.F_Fab, size=0.5, thick=0.08)

    # Locators and rests.
    # MH2 sits in an edge notch of the DUT, so its label goes inboard.
    label_at = {"MH2": ((-2.2, 0.0), "right")}
    for i, mh in enumerate(LOCATORS, 1):
        off, just = label_at.get(mh, ((0.0, -2.4), None))
        hole(board, f"H{i}", J(*from_dut(*holes[mh])), f"LOCATE {mh}", off, just)
    for i, (name, xy) in enumerate(RESTS.items(), 1):
        hole(board, f"H{len(LOCATORS) + i}", J(*from_dut(*xy)), f"REST {name}")

    # Header for the USB-UART adapter.
    hx, hy = HEADER_AT
    hdr = stock(
        board,
        "Connector_PinHeader_2.54mm",
        "PinHeader_1x03_P2.54mm_Vertical",
        "J1",
        "UART",
        J(hx, hy),
        (("LCSC", "C49257"), ("MPN", "2.54-1*3PPin"), ("Manufacturer", "BOOMELE")),
    )
    hdr_pad = {}
    for pad in hdr.Pads():
        name = HEADER_ORDER[int(pad.GetNumber()) - 1]
        pad.SetNet(nets[name])
        hdr_pad[name] = (mm(pad.GetPosition().x) - OX, mm(pad.GetPosition().y) - OY)
    labels = {"DUT_RX": "RX  <- adapter TX", "GND": "GND", "DUT_TX": "TX  -> adapter RX"}
    for name, (x, y) in hdr_pad.items():
        text(board, labels[name], J(x + 2.0, y), P.F_SilkS, size=0.9, thick=0.14, justify="left")
        back = {"DUT_RX": "RX", "GND": "GND", "DUT_TX": "TX"}[name]
        text(board, back, J(x - 2.0, y), P.B_SilkS, size=0.9, thick=0.14, justify="left", mirror=True)

    # Routing, all on B.Cu: three parallel lanes, 45 degree bends.
    rx, gnd, tx = hdr_pad["DUT_RX"], hdr_pad["GND"], hdr_pad["DUT_TX"]
    def lane(x, end):
        return [(x, end[1] - (end[0] - x)), end]

    track(board, [(rx_x, rx_y), (32.0, rx_y)] + lane(32.0, rx), nets["DUT_RX"])
    track(board, [(g_x, g_y), (30.0, g_y)] + lane(30.0, gnd), nets["GND"])
    track(board, [(tx_x, tx_y), (tx_x, 51.0), (28.0, 51.0)] + lane(28.0, tx), nets["DUT_TX"])

    # DUT outline (mirrored) on the top silk, for placement.
    # One polyline item (not 200 segments), clipped to the base plate.
    dut = P.SHAPE_LINE_CHAIN()
    for x, y in dut_outline:
        dut.Append(J(*from_dut(x, y)))
    dut.SetClosed(True)
    x0, y0, x1, y1 = BASE
    keep = P.SHAPE_LINE_CHAIN()
    for x, y in ((x0, y0 + 0.4), (x1, y0 + 0.4), (x1, y1), (x0, y1)):
        keep.Append(J(x, y))
    keep.SetClosed(True)
    a, k = P.SHAPE_POLY_SET(), P.SHAPE_POLY_SET()
    a.AddOutline(dut)
    k.AddOutline(keep)
    a.BooleanIntersection(k)
    sil = P.PCB_SHAPE(board)
    sil.SetShape(P.SHAPE_T_POLY)
    sil.SetPolyShape(a)
    sil.SetFilled(False)
    sil.SetLayer(P.F_SilkS)
    sil.SetWidth(P.FromMM(0.15))
    board.Add(sil)

    # Text.
    x0, y0, x1, y1 = BASE
    text(board, "OpenWalkieTalkie Rev B", J(48.0, 80.0), P.F_SilkS, size=1.2, thick=0.18)
    text(board, "UART first-flash jig v1", J(48.0, 82.0), P.F_SilkS, size=1.0, thick=0.15)
    text(board, "DUT battery side (F) DOWN", J(48.0, 85.0), P.F_SilkS, size=0.9, thick=0.14)
    text(board, "power the DUT from USB-C", J(48.0, 86.6), P.F_SilkS, size=0.9, thick=0.14)
    # Binder clip / thumb position: over REST A, beside the pins.
    text(board, "CLIP", J(x0 + 2.4, 48.5), P.F_SilkS, size=0.8, thick=0.12, angle=90)
    seg(board, J(x0 + 0.8, 45.5), J(x0 + 4.5, 45.5), P.F_SilkS, 0.15)
    seg(board, J(x0 + 0.8, 51.5), J(x0 + 4.5, 51.5), P.F_SilkS, 0.15)
    for f in FEET:
        circle(board, J(*f), 5.0, P.B_SilkS, 0.15)
    legend = (
        "REST A-D: M2x5 F-F standoff + M2x4 screw from below",
        "LOCATE MH2/MH4: M2x10 screw from below + M2 thin nut on top",
        "FEET: 4x bumper 10 mm dia, >= 5 mm tall",
        "pogo pins: Xinyangze YZ105312061P-01 (LCSC C5157237)",
    )
    for i, line in enumerate(legend):
        text(board, line, J(34.0, 56.0 + 1.8 * i), P.B_SilkS, size=0.9, thick=0.14, mirror=True)

    # Fiducials for JLC SMT.
    for i, (x, y) in enumerate(((x0 + 4.0, y0 + 14.0), (x1 - 4.0, y1 - 14.0)), 1):
        fid = stock(board, "Fiducial", "Fiducial_1mm_Mask2mm", f"FID{i}", "Fiducial", J(x, y))
        fid.Reference().SetVisible(False)

    board.SetFileName(OUT)
    P.SaveBoard(OUT, board)
    print("wrote", OUT)
    for tp, (x, y) in jig_pin.items():
        print(f"  {tp} -> jig ({x:.3f}, {y:.3f})")


if __name__ == "__main__":
    main()
