# OpenWalkieTalkie @VERSION@ 工厂合并烧录包

本包用于新板首烧，包含完整 Flash 分区表、带屏幕提示的 USB 恢复引导程序（协议 2）、
正式运行应用和中文字库。应用版本为 `@VERSION@-openwalkietalkie`，不是临时迁移程序。

**这是整片 Flash 镜像，会清除配对、设置和 Flash 中已有的板级数据。只用于新板首烧或明确要求的整片重置。
已交付用户的设备请使用桌面 App 的正常 USB 更新流程。**

## 选择文件

| 实际主板 | 使用文件 | 解压后大小 |
| --- | --- | --- |
| Rev B（当前生产主板） | `Rev-B/OpenWalkieTalkie-@VERSION@-factory-hw-b.bin` | 16,777,216 字节（16 MiB） |
| Rev A（旧版主板） | `Rev-A/OpenWalkieTalkie-@VERSION@-factory-hw-a.bin` | 16,777,216 字节（16 MiB） |

两个文件不能混用；Rev A/B 的引脚配置不同。每种主板的同一个文件适用于下表三种 U4，无需按 Flash 品牌分别选固件。

| 料号 | 型号 | JEDEC ID（线上顺序） |
| --- | --- | --- |
| C97521 | Winbond W25Q128JVSIQ | EF 40 18 |
| C5354566 | Puya PY25Q128HA-SUH-IR | 85 20 18 |
| C2928891 | GigaDevice GD25Q128ESIGR | C8 40 18 |

固件已加入两个替代料的识别及写入支持。此前的开发固件在现有 Rev A 上完成了应用/引导程序升级及启动验证，
但不能视为本包重新构建的正式文件已经过实机验证。Rev B 和两种替代 Flash 尚需首件实测。本包完成了软件布局、组件一致性与校验和检查，
尚未执行空白板整片烧录验证。请先完成所用主板及 Flash 组合的首件确认，再批量烧录。

## 校验文件

解压后，在本目录运行（只检查文件，不访问设备）：

```sh
python3 verify.py
```

Windows 安装 Python 3 后使用 `py -3 verify.py`。所有 5 个文件应显示 `OK`。
也可用 macOS 的 `shasum -a 256 -c SHA256SUMS` 或 Linux 的 `sha256sum -c SHA256SUMS`。
`manifest.json` 记录整片镜像及其各组成部分的 SHA-256、长度和地址偏移。

## 烧录地址：务必区分两种地址

| 烧录方式 | 文件起始地址 | 长度 |
| --- | --- | --- |
| 通过 SF32LB52 的 UART 下载工具 | **0x12000000**（MCU 映射地址） | 0x01000000 |
| 对独立 SPI NOR 芯片使用离线烧录器 | **0x00000000**（芯片内偏移） | 0x01000000 |

合并文件已经包含引导程序，**不要写到应用地址 0x12020000**。不需要再单独烧写 bootloader 或应用文件。
烧录器选择实装的 3 V / 128 Mbit NOR 型号，执行整片擦除、写入及完整读回校验；不要只跳过 FF 区域而保留旧数据。

## UART 治具接线与命令

使用 3.3 V 逻辑电平 UART，板子通过 USB-C 供电：

| 适配器 | 主板测试点 |
| --- | --- |
| TX | TP6（主板 RX） |
| RX | TP8（主板 TX） |
| GND | TP4 |

TP5 为高有效复位，保持悬空；TP3、TP10 不接。空白板需要 UART 或预烧录的 Flash，
只有 USB-C 数据线无法完成首烧。首烧成功后，设备自身 USB-C 才能用于恢复和后续更新。

使用官方 [OpenSiFli/sftool](https://github.com/OpenSiFli/sftool) 下载工具。
以下命令按本项目环境中的 `sftool 0.2.5` 参数编写；先用 `sftool --version` 检查版本。
将串口替换为治具实际端口，以下默认烧录 **Rev B**。

Windows（PowerShell 或 CMD，在解压目录执行）：

```text
sftool -c SF32LB52 -p COM9 --before no_reset --compat true write_flash --erase-all --verify Rev-B/OpenWalkieTalkie-@VERSION@-factory-hw-b.bin@0x12000000
```

macOS 示例（Linux 将端口改为 `/dev/ttyUSB0` 等）：

```sh
sftool -c SF32LB52 -p /dev/cu.usbserial-XXXX --before no_reset --compat true write_flash --erase-all --verify Rev-B/OpenWalkieTalkie-@VERSION@-factory-hw-b.bin@0x12000000
```

Rev A 将文件路径替换为 `Rev-A/OpenWalkieTalkie-@VERSION@-factory-hw-a.bin@0x12000000`。
工具报校验失败时，不得判定烧录通过；检查接线、供电、型号与文件校验和，再重新擦除烧录。

## 镜像布局

| 芯片偏移（含起止） | MCU 地址（含起止） | 内容 |
| --- | --- | --- |
| 000000–007FFF | 12000000–12007FFF | SDK Flash 表（实际 11,280 字节，其余 FF） |
| 008000–00FFFF | 12008000–1200FFFF | 保留/板级配置区，初始 FF |
| 010000–01FFFF | 12010000–1201FFFF | 协议 2 恢复引导程序，剩余 FF |
| 020000–6FFFFF | 12020000–126FFFFF | 应用代码、常量及初始化数据 |
| 700000–70FFFF | 12700000–1270FFFF | 原生蓝牙数据库，初始 FF |
| 710000–71FFFF | 12710000–1271FFFF | 配对/设置日志，初始 FF |
| 720000–FFFFFF | 12720000–12FFFFFF | 应用资源及中文字库，未用空间 FF |

文件不是某台设备的 Flash 拷贝，不含现有设备的蓝牙配对密钥或用户设置。
USB 序列号及蓝牙地址在运行时由 MCU 的 UID 派生；此镜像不会复制同一台设备的身份。
本包不包含板级射频校准结果。若产线另有校准或个性化数据写入工序，应放在整片烧录之后；
整片重烧会清除已写入外部 Flash 的这些数据，MCU 内部 eFuse 不属于本镜像。

## 首件验收

1. 烧录工具完成完整写入及读回校验，无错误。
2. 冷启动后屏幕方向、触摸、按键与旋钮正常；设置 → 关于显示 `@VERSION@ · Rev B`（或对应的 Rev A）。
3. 连接桌面 App 后识别为对应主板；系统信息为 `@VERSION@-openwalkietalkie`、恢复协议 `2`。
4. 验证蓝牙配对并重启，确认配对/设置可保存；抽检不同设备的 USB 序列号不同。
5. 进入恢复模式检查屏幕，再通过 USB 更新匹配主板的正常固件并确认重新启动。

首件记录应包含主板版本、U4 料号、镜像 SHA-256、烧录器/工具版本与测试结果。
本次制包与已连接设备的验证未执行音频测试。
