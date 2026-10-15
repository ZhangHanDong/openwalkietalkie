# SPDX-License-Identifier: MIT
# Regenerate: uv run --with openpyxl python hardware/rev-b/scripts/tooling/build_bom_zh.py [output.xlsx]
# Build the cascaded (multi-level) Chinese BOM for the OpenWalkieTalkie Rev B unit.
import csv, re, sys
from openpyxl import Workbook
from openpyxl.styles import Font, PatternFill, Alignment, Border, Side
from openpyxl.utils import get_column_letter
from openpyxl.worksheet.properties import Outline

from pathlib import Path
REPO = str(Path(__file__).resolve().parents[4])
OUT = sys.argv[1] if len(sys.argv) > 1 else f"{REPO}/hardware/rev-b/openwalkietalkie-revb-bom-zh.xlsx"
CSV = f"{REPO}/hardware/rev-b/fab-jlcpcb/main-bom-jlcpcb.csv"

rows = []  # dicts
def add(parent, name, spec="", mfr="", mpn="", pn="", refs="", cat="", src="", unit="个", qty=1, status="已确定", note=""):
    rows.append(dict(parent=parent, name=name, spec=spec, mfr=mfr, mpn=mpn, pn=pn, refs=refs, cat=cat,
                     src=src, unit=unit, qty=qty, status=status, note=note))
    return len(rows) - 1

# ---------------- Level 0 ----------------
root = add(None, "Cohdl OpenWalkieTalkie 整机", "主板 Rev B（2026-10-06）+ 编码器板 r6 + BodyV6.13 外壳", "Cohdl", "", "", "", "整机", "整机装配", "台", 1, "已确定",
           "顶层。整机用量 = 上级整机用量 × 单级用量（公式）。")

# ---------------- 1.1 Main PCBA ----------------
pcba = add(root, "主板组件（PCBA）Rev B", "4 层，53.43 × 92.43 mm，双面贴装 125 件 / 51 行", "", "main-revb", "hardware/rev-b/fab-jlcpcb/", "", "子装配", "JLCPCB 贴片",
           "块", 1, "已确定", "数据来源：hardware/rev-b/fab-jlcpcb/main-bom-jlcpcb.csv 及 ORDER-NOTES.md（2026-10-06 重建）。F 面（电池侧）60 件，B 面（显示侧）65 件。不贴装：C52/C53/C58/L6（RF 调谐 NF）、R44（10k 台架 NTC 替代）、TP1–TP10、MH2/MH4。")
add(pcba, "主板 PCB 裸板 Rev B", "4 层 1.0 mm，叠层 JLC04101H-3313（3313 半固化片 0.0994 mm + 0.7 mm 芯板），ENIG，最小过孔 0.20/0.45 mm（477 个），最小线宽/间距 0.12/0.10 mm，过孔树脂塞孔+盖孔，去除订单号", "JLCPCB", "main-revb-jlcpcb-gerber.zip", "hardware/rev-b/fab-jlcpcb/main-revb-jlcpcb-gerber.zip", "", "PCB", "JLCPCB 制板", "块", 1, "已确定",
    "厚度必须 1.0 mm（JLC 默认 1.6 mm 会改变外壳基准）。叠层不可替换：B 面 50 Ω 射频线按此叠层设计。48 个过孔位于 SMD 焊盘内，必须树脂塞孔+盖孔，麦克风声腔内 5 个过孔也需密封。")

# --- parse the JLCPCB BOM ---
PKG = {"CHIP_0402": "0402", "CHIP_0603": "0603", "CHIP_0805": "0805", "CHIP_1206": "1206"}
SPECIAL = {  # first designator -> (name, spec)
    "AE1": ("2.4 GHz 陶瓷贴片天线", "2450AT18B100E，蓝牙 BLE 天线"),
    "J1": ("扬声器连接器 MX1.25 2P", "Molex PicoBlade 53261-0271，2 位 1.25 mm 卧式贴片针座；配对插头 51021-0200"),
    "J2": ("板间 FPC 连接器 4P 0.5 mm", "Hirose FH34SRJ-4S，0.5 mm 间距 4 位，下接触，接编码器板"),
    "J3": ("电池连接器 MX1.25 3P", "Molex PicoBlade 53261-0371，3 位 1.25 mm 卧式贴片针座：1 VCELL、2 NTC、3 GND；配对插头 51021-0300"),
    "J5": ("USB Type-C 母座 16P", "HRO TYPE-C-31-M-12，16 位贴片+插件外壳，USB 2.0"),
    "L1": ("铁氧体磁珠", "600 Ω @100 MHz，1.5 A，1206（BLM31PG601SN1L），扬声器输出线"),
    "L3": ("功率电感 4.7 µH", "2016 封装（WPN201610U4R7MT），降压电感"),
    "L4": ("功率电感 2.2 µH", "2016 封装（DFE201612E-2R2M=P2）"),
    "MK1": ("PDM 数字麦克风", "MP34DT05TR-A，顶部进音，HCLGA 3×4 mm；B 面，装于壳体声柱内"),
    "SW1": ("侧按轻触开关", "ALPS SKTDLDE010，侧向按压贴片；SW1 ENTER、SW2 ESC、SW6 SHIFT（左侧）、SW5 PTT（右侧）"),
    "SW3": ("轻触开关", "C&K PTS636SM25SMTRLFS，贴片；SW3/SW4 前面左/右键（MACRO_LEFT/RIGHT）"),
    "U1": ("音频功率放大器", "NS4150B，SOP-8，单声道 D 类/AB 类，驱动扬声器（BTL）"),
    "U2": ("负载开关", "TPS22918DBVR，SOT-23-6；U2 麦克风供电，U10 显示逻辑供电"),
    "U3": ("显示/触摸 FPC 连接器 31P 0.3 mm", "DIANWEI 8.03A5.031250，0.3 mm 间距 31 位，接显示模组 FPC；B 面"),
    "U4": ("SPI NOR Flash 128 Mbit", "W25Q128JVSIQ，SOIC-8，QSPI"),
    "U5": ("主控 MCU（BLE SoC）", "SiFli SF32LB525UC6，QFN-68，Cortex-M33，含蓝牙；B 面"),
    "U6": ("单节锂电充电管理", "TI BQ24073RGTR，VQFN-16，线性充电+电源路径，247 mA 充电/500 mA 输入限流，系统轨 4.4 V；需 10 kΩ 电池 NTC"),
    "U7": ("同步降压 DC-DC", "Richtek RT6150BGQW，WDFN-10，可调输出；U7 产生 IO_3V3（≈3.28 V），U8 另一路"),
    "U20": ("USB ESD 保护", "USBLC6-2SC6，SOT-23-6"),
    "Y1": ("晶振 48 MHz", "2016 封装 4 脚（Hosonic E1SB48E001G00E），SiFli 要求 CL 7–12 pF、ESR ≤ 30 Ω"),
}
NOTE_ZH = {
    "AE1": ("待确认", "与设计型号一致。2026-09-23 LCSC 缺货，下单前核查 JLC 库存或全球采购。"),
    "C1": ("已确定", "JLC 基础库替代 Samsung CL10A105KB8NNNC（50 V），容值/介质/封装相同，耐压更高。"),
    "C2": ("已确定", "与设计型号一致。"),
    "C6": ("已确定", "与设计型号一致。"),
    "C7": ("已确定", "设计型号各分销商均无货，替代 Murata GRM31C5C1H104JA01L（100 nF C0G 1206 50 V）。"),
    "C8": ("已确定", "与设计型号一致。"),
    "C12": ("已确定", "JLC 基础库替代 Samsung CL21A106KAYNNNE，规格相同（原型号 LCSC 缺货）。"),
    "C13": ("已确定", "Rev B：MCU 移位后由 0603 改为 0402；4.7 µF 10 V X5R（JLC 基础库），轨电压 ≤ 3.3 V。"),
    "C14": ("已确定", "Rev B：由 0603 改为 0402；2.2 µF 10 V X5R（6.3 V 基础库 C12530 亦可）。"),
    "C15": ("已确定", "Rev B：由 0603 改为 0402；1 µF 25 V X5R（JLC 基础库）。"),
    "C18": ("已确定", "JLC 基础库替代 Samsung CL10A475KO8NNNC（16 V），规格相同，耐压更高。"),
    "C19": ("已确定", "JLC 基础库替代 Samsung CL05B104KO5NNNC，规格相同。"),
    "C25": ("已确定", "与设计型号一致。"),
    "C28": ("已确定", "Rev B：改 0402 以放入 MCU 后方 F 面电容排（U5 pin 24 VCC 去耦）。位于 SYSTEM_SUPPLY（≤ 4.4 V），直流偏压后有效容量约减半，由 C12/C42/C45 补足。"),
    "C42": ("已确定", "同规格同厂替代 TDK C3216X7R1C106KT000N（原型号 LCSC 缺货）。"),
    "C57": ("已确定", "Rev B：射频直流隔断电容，串联于 U5 pin 53 的 π 网络；C0G 50 V（JLC 基础库）。"),
    "J1": ("已确定", "与设计型号一致。"),
    "J2": ("已确定", "同一连接器：(50) 为卷装，(99) 为小包装（无库存）。"),
    "J3": ("已确定", "Rev B 新电池连接器（MX1.25 3 位卧式贴片）：1 VCELL、2 NTC、3 GND；配对插头 Molex 51021-0300。"),
    "J5": ("已确定", "与设计型号一致。"),
    "L1": ("已确定", "与设计型号一致。"),
    "L3": ("已确定", "与设计型号一致。"),
    "L4": ("已确定", "与设计型号一致。"),
    "MK1": ("已确定", "与设计型号一致（扩展库）。2026-09-23 LCSC 约 577 件。B 面安装，进音孔朝壳体声孔。"),
    "R1": ("已确定", "JLC 基础库替代 UniOhm，规格相同。"),
    "R3": ("已确定", "JLC 基础库替代 UniOhm，规格相同。"),
    "R4": ("已确定", "JLC 基础库替代 UniOhm，规格相同；沿用 r6 的 22k。"),
    "R6": ("已确定", "JLC 基础库替代（22 Ω）。"),
    "R9": ("已确定", "JLC 基础库替代 UniOhm，规格相同。"),
    "R10": ("已确定", "JLC 基础库替代 UniOhm，规格相同。"),
    "R28": ("已确定", "JLC 基础库替代 UniOhm，规格相同；I²C 上拉。"),
    "R31": ("已确定", "与设计型号一致。"),
    "R32": ("待确认", "与设计型号一致。LCSC 缺货，JLC 库存偏低，下单前核查。"),
    "R37": ("已确定", "与设计型号一致。R37 改为 3.6k（RC0402FR-073K6L，C137990），设定 BQ24073 充电电流：890/3.6k ≈ 247 mA，为 500 mAh 电芯的约 0.5C。"),
    "R38": ("已确定", "UniOhm 同规格替代。ILIM 电阻仅在 ILIM 模式起作用，本板运行于 USB500 模式。"),
    "R40": ("已确定", "Rev B：天线侧匹配 S1/S2，调谐前为 0 Ω（Johanson 评估值：S1 1.2 pF、S2 3.3 nH）。"),
    "R43": ("已确定", "UniOhm 同规格替代；U7 反馈分压下臂电阻。"),
    "R45": ("已确定", "Rev B：TOUCH_RESET 下拉。"),
    "R46": ("已确定", "Rev B：CHG_STATUS 上拉至 IO_3V3（充电器 CHG → MCU PA33）。"),
    "R48": ("已确定", "Rev B：将显示模组 TP_VCC（U3 pin 6）接到 IO_3V3；拆除则悬空（同 r6）。"),
    "SW1": ("已确定", "与设计型号一致。"),
    "SW3": ("待确认", "与设计型号一致。2026-09-23 LCSC 缺货，核查 JLC 库存。"),
    "U1": ("已确定", "与设计型号一致。"),
    "U2": ("已确定", "与设计型号一致。"),
    "U3": ("已确定", "与设计型号一致。"),
    "U4": ("已确定", "与设计型号一致（JLC 基础库）。"),
    "U5": ("待确认", "与设计型号一致。LCSC 缺货，JLC 另列编号 C9900153074，下单前核查。"),
    "U6": ("已确定", "与设计型号一致（扩展库），约 16,900 件。替代已缺货的 SGM41562SA。"),
    "U7": ("已确定", "与设计型号一致，约 1,350 件。U7 亦改用可调版（配 R42/R43），固定 3.3 V 版 RT6150B-33GQW 缺货。"),
    "U20": ("已确定", "与设计型号一致。"),
    "Y1": ("待确认", "LCSC/JLC 均无货。候选 C54561124（TKD SX20Y048000B81T001，48 MHz 8 pF ±10 ppm，ESR 未标注）需按 SiFli 限值（CL 7–12 pF，ESR ≤ 30 Ω）评审后再用。"),
}
MFRS = ("Samsung", "Murata", "TDK", "UniOhm", "Yageo")

def placed_part(placed, design_mfr, design_mpn):
    """Return (manufacturer, MPN) actually ordered."""
    if not placed:
        return design_mfr, design_mpn
    tok = placed.split()
    if tok[0] in MFRS:
        return tok[0], tok[1]
    return design_mfr, tok[0]

smt_rows = []
with open(CSV, newline="", encoding="utf-8") as f:
    for r in csv.DictReader(f):
        refs = [d.strip() for d in r["Designator"].split(",")]
        first = refs[0]
        pkg = PKG.get(r["Footprint"], "")
        comment = r["Comment"]
        if first in SPECIAL:
            name, spec = SPECIAL[first]
        elif first.startswith("C"):
            name = f"贴片电容 {comment} {pkg}"
            extra = re.search(r"(\d+(?:\.\d+)?V\s+\S+)", r["Part placed (if substituted)"] + " " + r["Note"])
            spec = f"{comment}，{pkg}" + (f"，{extra.group(1)}" if extra else "")
        elif first.startswith("R"):
            name = f"贴片电阻 {comment.replace('ohm', 'Ω').replace('kΩ','kΩ')} {pkg}"
            spec = f"{comment.replace('ohm', 'Ω')}，1%，{pkg}"
        else:
            name, spec = comment, pkg
        mfr, mpn = placed_part(r["Part placed (if substituted)"], r["Design Manufacturer"], r["Design MPN"])
        key = next((k for k in NOTE_ZH if k == first), None)
        status, note = NOTE_ZH.get(key, ("已确定", ""))
        if r["Part placed (if substituted)"] and "设计" not in note:
            note = f"设计型号 {r['Design Manufacturer']} {r['Design MPN']}。" + note
        smt_rows.append((name, spec, mfr, mpn, r["LCSC Part #"] or "—", ", ".join(refs), int(r["Quantity"]), status, note))

for name, spec, mfr, mpn, pn, refs, qty, status, note in smt_rows:
    add(pcba, name, spec, mfr, mpn, pn, refs, "电子元件（SMT）", "JLCPCB 贴片", "个", qty, status, note)
smt_first, smt_last = len(rows) - len(smt_rows), len(rows) - 1

# ---------------- 1.2 Encoder PCBA ----------------
enc = add(root, "编码器板组件（PCBA）r6", "拨轮编码器小板，经 4P FPC 接主板 J2；Rev B 未改动，沿用 r6 的 encoder/ 包", "", "encoder-45deg", "hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/fab-jlcpcb/encoder/", "", "子装配", "JLCPCB 贴片", "块", 1, "已确定",
          "二选一：r6 标准板（ENC1 在 F 面，配标准拨轮）或 encoder-wheel 变体（ENC1 翻到拨轮侧，需带腔改型拨轮，1.7.3 另注）。本 BOM 按标准板。")
add(enc, "编码器板 PCB 裸板", "2 层 1.0 mm，ENIG，过孔 0.20/0.45 mm（盖油）；MH2 Ø2.8 套壳体定位柱，MH1 Ø2.6 过 M2 螺钉", "JLCPCB", "encoder-jlcpcb-gerber.zip", "hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/fab-jlcpcb/encoder/encoder-jlcpcb-gerber.zip", "", "PCB", "JLCPCB 制板", "块", 1, "已确定", "")
add(enc, "旋转编码器（拨轮）", "ALPS EC05E1220401，增量式机械编码器，六角轴孔 1.72 mm 对边；拨轮六角轴插入", "Alps Alpine", "EC05E1220401", "C116648", "ENC1", "电子元件（SMT）", "JLCPCB 贴片", "个", 1, "已确定", "与设计型号一致。")
add(enc, "板间 FPC 连接器 4P 0.5 mm", "Hirose FH34SRJ-4S，0.5 mm 间距 4 位，下接触；接主板 J2", "Hirose", "FH34SRJ-4S-0.5SH(50)", "C596732", "J1", "电子元件（SMT）", "JLCPCB 贴片", "个", 1, "已确定", "设计型号 FH34SRJ-4S-0.5SH(99)。同一连接器，(50) 为卷装。")

# ---------------- 1.3 Display ----------------
add(root, "显示模组 2.16″ AMOLED 480×480（带触摸）", "华夏彩 H0216F002AMT003A V2：2.16″ LTPS-AMOLED，480RGB×480，QSPI 接口，驱动 IC CO5300，触摸 IC CST9220（I²C），31P 0.3 mm FPC；模组 40.29 × 41.09 × 0.8 mm，含盖板 44.09 × 45.39 mm，总厚 1.93 mm，亮度 700 cd/m²",
    "华夏彩光电（深圳）有限公司 Huaxia RGB", "H0216F002AMT003A V2", "H0216F002AMT003A V2", "插入主板 U3", "显示模组", "外购", "片", 1, "已确定",
    "规格书：hardware/rev-a/mechanical-source/display/H0216F002AMT003A_V2_specification.pdf（V2，2026-07-14 更改盖板尺寸）。FPC 插入主板 B 面 U3（8.03A5.031250）。")

# ---------------- 1.4 Inter-board FPC ----------------
add(root, "板间 FPC 软排线 4P 0.5 mm（主板 J2 ↔ 编码器板 J1）", "4 位 0.5 mm 间距，两端加强板，端部厚 0.30 ± 0.03 mm，名义宽 2.5 mm；展开长度约 31.1 mm（端到端中心线），U 形弯折半径 ≈ 2.9 mm；引脚 1 GND、2 ENC_A、3 ENC_B、4 GND",
    "待选", "待定", "待定", "主板 J2 ↔ 编码器 J1", "线材", "外购/定制", "条", 1, "待确认",
    "尺寸来自 assembly-notes/cable-concept.json（r4 概念，非正式柔性板图纸）。两板 F 面同向且走线为 U 形，推断两端触点在异面（Type B）；下单前按装配路径复核触点朝向与长度（HW-009）。")

# ---------------- 1.5 Battery ----------------
add(root, "锂聚合物电池 702040 3.7 V 500 mAh（带保护板、NTC、插头）", "型号 702040（7.0 × 20 × 40 mm），3.7 V 单节，标称 500 mAh，带保护板；内置 10 kΩ NTC；3 线引出，插头 Molex 51021-0300（MX1.25 3P）：1 电池+、2 NTC、3 GND（与主板 J3 对应）",
    "待选", "702040", "待定", "插入主板 J3", "电池", "外购/定制", "只", 1, "待确认",
    "用户指定 702040 / 3.7 V / 500 mAh（2026-10-10）。NTC 必须为 10 kΩ（BQ24073 TS 输入；100k 会被判为低温不充电）。充电电流 247 mA ≈ 0.5C，核对电芯允许的充电倍率；引线长度、保护板参数按供应商规格确认。文档中早期原型电芯为 450 mAh（HW-006），非本型号。")

# ---------------- 1.6 Speaker ----------------
add(root, "扬声器（带引线和插头）", "4 Ω 微型扬声器（尺寸/额定功率待定），2 线引出，插头 Molex 51021-0200（MX1.25 2P），接主板 J1；由 NS4150B BTL 驱动，两根引线均带电",
    "待选", "待定", "待定", "插入主板 J1", "扬声器", "外购/定制", "只", 1, "待确认",
    "仓库中无具体型号。docs/hardware-design-constraints.md（HW-006）：4 Ω/3 W 标称不代表单节供电下能输出 3 W。选型需与壳体前格栅配合。")

# ---------------- 1.7 Mechanical (3D printed) ----------------
mech = add(root, "外壳结构件（3D 打印）BodyV6.13", "Fusion 设计 BodyV6.13（hardware/rev-a/mechanical-source/case/BodyV6.13.f3d / .step），导出含 8 个实体；后盖未在导出中", "Cohdl", "BodyV6.13", "BodyV6.13.step", "", "子装配", "3D 打印", "套", 1, "待确认",
           "打印材料与工艺未在仓库中指定（待定）。拨轮六角轴尺寸小，建议树脂（SLA/MJF）打印。")
for name, solid, note in [
    ("主壳体", "MainCase", "含显示窗口、麦克风声柱、天线角、右侧 M2 螺钉凸台（Ø1.75 盲导孔）、主板角柱与座块；显示模组用 B-7000 胶粘接。"),
    ("后盖", "（未导出）", "BodyV6.13 导出为 8 个实体而非 9 个，后盖缺失；ENC1 穿后盖及编码器板 MH2 定位柱/后盖凸台配合未验证。需导出后补充。"),
    ("拨轮", "Wheel", "标准拨轮：Ø17，六角轴 1.905 mm 对边穿编码器板入 ENC1。若选 encoder-wheel 板，改用 encoder-wheel/mechanical/wheel-with-encoder-cavity-bodyv6.13.step（Ø10.6 腔，六角 1.68 对边，需树脂/MJF）。"),
    ("PTT 按键", "PushToTalk", "右侧，压 SW5。"),
    ("Shift 按键", "Shift", "左侧，压 SW6。"),
    ("Enter 按键", "Enter", "左侧，压 SW1。"),
    ("Cancel 按键", "Cancel", "左侧，压 SW2（ESC）。"),
    ("左前按键 Left", "Left", "前面，压 SW3（MACRO_LEFT）。"),
    ("右前按键 Right", "Right", "前面，压 SW4（MACRO_RIGHT）。"),
]:
    add(mech, name, f"BodyV6.13 实体 “{solid}”", "Cohdl", solid, "BodyV6.13.step", "", "结构件（3D 打印）", "3D 打印", "个", 1,
        "待确认" if solid == "（未导出）" else "已确定", note)

# ---------------- 1.8 Fasteners ----------------
fast = add(root, "紧固件", "M2×4 螺钉两种头型（用户指定 2026-10-10）", "", "", "", "", "子装配", "外购", "套", 1, "待确认", "数量按装配位置点数核对。")
add(fast, "螺钉 M2×4 平头（PCB 与喇叭固定）", "M2 × 4 mm，平头，自攻/塑料用；固定主板（右侧 U 形槽 → 壳体凸台 (52.2, 52.0)）、编码器板（MH1 Ø2.6 → 座块）和扬声器", "待选", "M2x4 平头", "待定",
    "主板 U 槽 ×1、编码器板 MH1 ×1、扬声器 ×1（待核）", "紧固件", "外购", "颗", 3, "待确认",
    "用户指定 M2×4 平头。数量按 主板 1 + 编码器板 1 + 扬声器 1 估计，扬声器点数待核。注意 r6 README 对主板凸台的核算：板 F 面到导孔底 3.975 mm，M2×4 会顶底，M2×3 咬合 2.0 mm；凸台右侧肋条距螺钉轴 1.8 mm，头径需 ≤ Ø3.4 或在 Fusion 中局部让位。请按 BodyV6.13 现行 CAD 复核导孔深度与头部空间。")
add(fast, "螺钉 M2×4 沉头（背板固定）", "M2 × 4 mm，沉头（90°），自攻/塑料用；固定后盖/背板到主壳体", "待选", "M2x4 沉头", "待定",
    "后盖 → 主壳体（点数待核）", "紧固件", "外购", "颗", 2, "待确认",
    "用户指定 M2×4 沉头。后盖未在 BodyV6.13 导出中，螺钉点数按 2 估计，待后盖设计确定后更新；沉头孔需在后盖上做 90° 锪孔。")

# ---------------- 1.9 Consumables ----------------
cons = add(root, "辅料", "", "", "", "", "", "子装配", "外购", "套", 1, "待确认", "")
add(cons, "B-7000 胶", "B-7000 多用途透明胶（显示模组/盖板与壳体粘接）", "待选", "B-7000", "待定", "显示模组 ↔ 主壳体", "辅料", "外购", "支", 1, "待确认",
    "用户指定。单台用量按工艺定量；此处按每台 1 支计入。")
add(cons, "麦克风密封垫（可选）", "泡棉或硅胶环，约 1 mm 厚，套在麦克风声柱盖与前壁之间，围住 Ø2 声孔", "待选", "待定", "待定", "壳体声柱 ↔ 前壁", "辅料", "外购", "个", 1, "待确认",
    "r6 README 建议：缩小声柱空气腔，把约 5 kHz 的亥姆霍兹共振峰推出语音频带。非必需，可删除。")

# ---------------- numbering ----------------
children = {}
for i, r in enumerate(rows):
    children.setdefault(r["parent"], []).append(i)
code = {}
def number(i, prefix):
    code[i] = prefix
    for n, c in enumerate(children.get(i, []), 1):
        number(c, f"{prefix}.{n}")
number(root, "1")
level = {i: code[i].count(".") for i in range(len(rows))}

# ---------------- workbook ----------------
wb = Workbook()
ws = wb.active
ws.title = "多级BOM"
FONT = "Arial"
hdr = ["层级", "物料编号", "物料名称", "规格 / 描述", "制造商", "制造商型号", "采购编号 / 文件", "位号 / 装配位置", "类别", "来源 / 工艺", "单位", "单级用量", "整机用量", "状态", "备注"]
widths = [6, 10, 34, 60, 22, 26, 26, 26, 16, 14, 6, 9, 9, 8, 70]
thin = Side(style="thin", color="BFBFBF")
border = Border(left=thin, right=thin, top=thin, bottom=thin)
ws.append(hdr)
for c in range(1, len(hdr) + 1):
    cell = ws.cell(row=1, column=c)
    cell.font = Font(name=FONT, bold=True, color="FFFFFF", size=10)
    cell.fill = PatternFill("solid", fgColor="1F3864")
    cell.alignment = Alignment(horizontal="center", vertical="center", wrap_text=True)
    cell.border = border
    ws.column_dimensions[get_column_letter(c)].width = widths[c - 1]
ws.row_dimensions[1].height = 30

fills = {0: "D9E1F2", 1: "EDEDED", 2: None, 3: None}
excel_row = {}
for i, r in enumerate(rows):
    rr = i + 2
    excel_row[i] = rr
    lv = level[i]
    vals = [lv, code[i], r["name"], r["spec"], r["mfr"], r["mpn"], r["pn"], r["refs"], r["cat"], r["src"], r["unit"], r["qty"], None, r["status"], r["note"]]
    ws.append(vals)
    # extended quantity = parent's extended quantity x this level's quantity
    if r["parent"] is None:
        ws.cell(row=rr, column=13).value = f"=L{rr}"
    else:
        ws.cell(row=rr, column=13).value = f"=M{excel_row[r['parent']]}*L{rr}"
    bold = lv <= 1
    for c in range(1, len(hdr) + 1):
        cell = ws.cell(row=rr, column=c)
        cell.font = Font(name=FONT, size=10, bold=bold, color=("C00000" if (c == 14 and r["status"] == "待确认") else "000000"))
        cell.border = border
        cell.alignment = Alignment(vertical="top", wrap_text=(c in (3, 4, 8, 15)), horizontal=("center" if c in (1, 2, 11, 12, 13, 14) else "left"))
        if fills.get(lv):
            cell.fill = PatternFill("solid", fgColor=fills[lv])
    ws.cell(row=rr, column=3).alignment = Alignment(vertical="top", wrap_text=True, indent=lv * 2)
    ws.cell(row=rr, column=12).number_format = "0"
    ws.cell(row=rr, column=13).number_format = "0"
    ws.cell(row=rr, column=12).font = Font(name=FONT, size=10, bold=bold, color="0000FF")  # input

# outline groups: children collapse under their parent
ws.sheet_properties.outlinePr = Outline(summaryBelow=False, summaryRight=False)
def group(i):
    kids = children.get(i, [])
    if not kids:
        return
    first, last = excel_row[kids[0]], excel_row[kids[-1]]
    # last descendant row
    def last_desc(j):
        k = children.get(j, [])
        return last_desc(k[-1]) if k else excel_row[j]
    last = last_desc(kids[-1])
    for rr in range(first, last + 1):
        pass
    ws.row_dimensions.group(first, last, outline_level=level[i] + 1, hidden=False)
    for k in kids:
        group(k)
# openpyxl's group() sets the same outline level for the whole range; nested levels are set per row below instead.
for i, r in enumerate(rows):
    if level[i] >= 1:
        ws.row_dimensions[excel_row[i]].outlineLevel = level[i]
ws.freeze_panes = "D2"
ws.auto_filter.ref = f"A1:{get_column_letter(len(hdr))}{len(rows) + 1}"

# check rows
chk = len(rows) + 3
ws.cell(row=chk, column=3, value="校验：主板贴片元件合计（应为 125）").font = Font(name=FONT, size=10, bold=True)
ws.cell(row=chk, column=13, value=f"=SUM(M{excel_row[smt_first]}:M{excel_row[smt_last]})").font = Font(name=FONT, size=10, bold=True)
ws.cell(row=chk + 1, column=3, value="校验：主板 BOM 行数（应为 51）").font = Font(name=FONT, size=10, bold=True)
ws.cell(row=chk + 1, column=13, value=f"=COUNTA(C{excel_row[smt_first]}:C{excel_row[smt_last]})").font = Font(name=FONT, size=10, bold=True)
ws.cell(row=chk + 2, column=3, value="图例：蓝色数字 = 可编辑的单级用量；整机用量为公式；状态红色“待确认”= 下单前需确认。").font = Font(name=FONT, size=9, italic=True)

# ---------------- summary sheet ----------------
ss = wb.create_sheet("汇总")
ss.append(["类别", "物料行数", "整机用量合计", "说明"])
cats = [("电子元件（SMT）", "主板 + 编码器板贴片元件"), ("PCB", "裸板"), ("显示模组", ""), ("线材", "板间 FPC"), ("电池", ""), ("扬声器", ""),
        ("结构件（3D 打印）", "BodyV6.13 实体 + 后盖"), ("紧固件", ""), ("辅料", "B-7000 胶、密封垫"), ("子装配", "分组行，不计入物料")]
for n, (cat, desc) in enumerate(cats, 2):
    ss.cell(row=n, column=1, value=cat)
    ss.cell(row=n, column=2, value=f"=COUNTIF('多级BOM'!$I:$I,A{n})")
    ss.cell(row=n, column=3, value=f"=SUMIF('多级BOM'!$I:$I,A{n},'多级BOM'!$M:$M)")
    ss.cell(row=n, column=4, value=desc)
tot = len(cats) + 2
ss.cell(row=tot, column=1, value="合计（不含子装配）")
ss.cell(row=tot, column=2, value=f"=SUM(B2:B{tot-2})")
ss.cell(row=tot, column=3, value=f"=SUM(C2:C{tot-2})")
for row in ss.iter_rows(min_row=1, max_row=tot, max_col=4):
    for cell in row:
        cell.font = Font(name=FONT, size=10, bold=(cell.row in (1, tot)))
        cell.border = border
        if cell.row == 1:
            cell.fill = PatternFill("solid", fgColor="1F3864"); cell.font = Font(name=FONT, size=10, bold=True, color="FFFFFF")
for col, w in zip("ABCD", (24, 12, 14, 40)):
    ss.column_dimensions[col].width = w

# ---------------- notes sheet ----------------
ns = wb.create_sheet("说明")
notes = [
    ("文件", "Cohdl OpenWalkieTalkie 整机多级 BOM（中文），基于主板 Rev B"),
    ("生成日期", "2026-10-10"),
    ("主板版本", "Rev B，2026-10-06 重建（hardware/rev-b/manifest.json）；KiCad 10.0.4；4 层，125 件贴装"),
    ("编码器板", "r6 标准板（hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/encoder/），Rev B 未改动；可选 encoder-wheel 变体"),
    ("外壳", "BodyV6.13（hardware/rev-a/mechanical-source/case/BodyV6.13.step），8 个实体：MainCase、Wheel、PushToTalk、Shift、Enter、Cancel、Left、Right；后盖未导出"),
    ("层级规则", "层级 0 整机；层级 1 子装配或整机级独立物料；层级 2 子装配内的元件/裸板。物料编号按 1 / 1.x / 1.x.y 级联。"),
    ("用量公式", "整机用量 = 上级行的整机用量 × 本行单级用量（M 列公式）。修改蓝色的单级用量后自动重算。"),
    ("数据来源 1", "hardware/rev-b/fab-jlcpcb/main-bom-jlcpcb.csv、main-cpl-jlcpcb.csv、ORDER-NOTES.md（制造商型号为实际贴装型号；设计型号写在备注）"),
    ("数据来源 2", "hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/fab-jlcpcb/encoder/encoder-bom-jlcpcb.csv、README.md（螺钉、座块、声柱）"),
    ("数据来源 3", "hardware/rev-a/mechanical-source/display/H0216F002AMT003A_V2_specification.pdf（显示模组参数）"),
    ("数据来源 4", "hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/assembly-notes/cable-concept.json（板间 FPC 几何）"),
    ("数据来源 5", "docs/hardware-design-constraints.md HW-006 / HW-009（电池、扬声器、FPC 约束）"),
    ("用户指定", "702040 3.7 V 500 mAh 电池、B-7000 胶、3D 打印结构件、显示模组、扬声器、板间 FPC 由用户要求列入；其具体供应商/型号仓库中无记录，标为“待确认”。"),
    ("待确认 1", "电子元件缺货项：AE1、SW3/SW4、U5、R32、Y1（Y1 需替代晶振评审）"),
    ("待确认 2", "板间 FPC：触点同面/异面、长度（按 cable-concept 推断为异面 Type B，约 31 mm）"),
    ("待确认 3", "电池：供应商、引线长度、保护板参数、充电倍率（247 mA ≈ 0.5C）；NTC 必须 10 kΩ"),
    ("待确认 4", "扬声器：型号、尺寸、功率（4 Ω）"),
    ("待确认 5", "结构件：打印材料/工艺；后盖未导出；拨轮按编码器板变体二选一"),
    ("待确认 6", "紧固件：用户指定 M2×4 平头（PCB、喇叭）与 M2×4 沉头（背板）；点数待核（估计 3 + 2）；r6 README 核算主板凸台导孔深 3.975 mm、M2×4 会顶底且头径需 ≤ Ø3.4，须按现行 CAD 复核"),
    ("未包含", "包装、标签、烧录治具（hardware/rev-b/jig）、电池固定胶带/泡棉（仓库未指定）"),
    ("公式校验", "本机无 LibreOffice，未做自动重算校验；公式仅为乘法与 SUM/COUNTIF/SUMIF，Excel 打开时自动计算。"),
]
ns.append(["项目", "内容"])
for k, v in notes:
    ns.append([k, v])
for row in ns.iter_rows(min_row=1, max_row=len(notes) + 1, max_col=2):
    for cell in row:
        cell.font = Font(name=FONT, size=10, bold=(cell.row == 1 or cell.column == 1))
        cell.alignment = Alignment(vertical="top", wrap_text=True)
        cell.border = border
        if cell.row == 1:
            cell.fill = PatternFill("solid", fgColor="1F3864"); cell.font = Font(name=FONT, size=10, bold=True, color="FFFFFF")
ns.column_dimensions["A"].width = 14
ns.column_dimensions["B"].width = 120

wb.calculation.fullCalcOnLoad = True
wb.save(OUT)
print(f"rows={len(rows)} smt_lines={len(smt_rows)} smt_qty={sum(r[6] for r in smt_rows)} saved={OUT}")
