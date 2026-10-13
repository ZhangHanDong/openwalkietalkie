/*
 * OpenWalkieTalkie recovery mode for the SiFli second-stage bootloader.
 *
 * The SDK bootloader (example/boot_loader/project/butterflmicro) boots the
 * application by loading SP/PC from 0x12020000 with no check at all, and has
 * no way in from the board's only connector. This file wraps the last call
 * before that jump (board_init_psram(), from boot_images_help()) and goes
 * into USB recovery instead when
 *
 *   - the application asked for it (RTC backup register RECOVERY_BKP holds
 *     RECOVERY_MAGIC, written by the firmware before a PMU reboot),
 *   - SHIFT and CANCEL are both held while the chip comes up, or
 *   - the application image is not valid (erased flash, an interrupted
 *     download: its vector table is checked before every jump).
 *
 * Recovery mode is the application's own USB device, descriptor for
 * descriptor: the vendor HID collection the firmware presents as
 * "OpenWalkieTalkie" (the same identity, strings, serial, endpoints and
 * report descriptor), so a host that has accepted the application accepts this
 * too, with no driver and no prompt. The image travels on channel 9 of the
 * same 64-byte reports (tools/flash-usb.py), one 4 KB sector at a time with
 * a CRC, written to the application window only: the flash table, this
 * bootloader and the pairing journal above the window are never touched,
 * so no download can take the board below this point. The vector-table
 * sector is written last, so a download that stops half way leaves an
 * invalid image and the next boot comes back here by itself. The chip's
 * UART debug port on TP6/TP8 stays the path of last resort for the
 * bootloader itself.
 */
#include <string.h>
#include "rtconfig.h"
#include "bf0_hal.h"
#include "mem_map.h"
#include "board.h"
#include "boot_flash.h"
#include "usbd_core.h"
#include "usbd_hid.h"

/* The application window: the Rust image links FLASH at 0x12020000 for
 * 7 MB - 64 KB; the pairing-key journal follows and is never written here. */
#define APP_BASE        HCPU_FLASH_CODE_START_ADDR
#define APP_END         (APP_BASE + (7u * 1024u * 1024u) - (64u * 1024u))
#define SECTOR          4096u

/* "OWTR" in RTC BKP7R: the SDK labels that register BT_TXPWR and never writes
 * it on this chip; the application's pmu::reboot(recovery) does. */
#define RECOVERY_MAGIC  0x4F575452u
#define RECOVERY_BKP    RTC_BACKUP_BT_TXPWR

/* Nobody spoke the recovery protocol within this long: boot the application
 * if it is valid (a pocketed button combination must not leave the deck dark
 * for good). With no valid image, wait for ever. */
#define HOST_WAIT_MS    120000u

/* The application's USB device (src/link/usb.rs, src/link/wire.rs). */
#define USBD_VID        0x1209
#define USBD_PID        0x5754
#define HID_IN_EP       0x81
#define HID_OUT_EP      0x02
#define REPORT_LEN      64
#define REPORT_ID       6
#define PAYLOAD_MAX     61

/* Channel 9 of the vendor reports: the recovery protocol. */
#define CHANNEL         9
#define OP_HELLO        1   /* -> 'O' 'W' 'T' 'B', protocol 1, image valid */
#define OP_BEGIN        2   /* erase the vector-table sector, start a session */
#define OP_DATA         3   /* [off lo][off hi][n][n bytes] into the sector buffer */
#define OP_COMMIT       4   /* [addr u32][len u16][crc32 u32] -> [status][addr u32] */
#define OP_FINISH       5   /* write the held vector-table sector -> [status][valid] */
#define OP_BOOT         6   /* reboot (into the application when it is valid) */
#define OP_READ         7   /* [addr u32][n] -> [n][bytes], for verification */
#define ST_OK           0
#define ST_ARGS         1
#define ST_CRC          2
#define ST_FLASH        3
#define ST_VERIFY       4
#define PROTOCOL        1

extern struct sec_configuration sec_config_cache;
extern void boot_uart_tx(USART_TypeDef *uart, uint8_t *data, int len);
extern void __real_board_init_psram(void);

static void say(const char *s)
{
    boot_uart_tx(hwp_usart1, (uint8_t *)s, strlen(s));
}

static void say_hex(uint32_t v)
{
    static const char digits[] = "0123456789ABCDEF";
    char text[11] = "0x";
    for (int i = 0; i < 8; i++)
        text[2 + i] = digits[(v >> (28 - 4 * i)) & 0xF];
    text[10] = 0;
    say(text);
}

/* The vector table at the top of the window: a stack pointer inside HPSYS
 * SRAM and a Thumb reset vector inside the window. Erased flash fails both. */
static int app_valid(void)
{
    uint32_t sp = *(volatile uint32_t *)APP_BASE;
    uint32_t pc = *(volatile uint32_t *)(APP_BASE + 4);
    if (sp < HPSYS_RAM0_BASE || sp > HPSYS_RAM0_BASE + HPSYS_RAM_SIZE || (sp & 3))
        return 0;
    if ((pc & 1) == 0 || pc < APP_BASE || pc >= APP_END)
        return 0;
    return 1;
}

/* SHIFT (PA27) and CANCEL (PA26) both low. The board pulls every button up
 * through 100k, so two lows together are a deliberate hold, not noise. */
static int buttons_held(void)
{
    HAL_RCC_EnableModule(RCC_MOD_GPIO1);
    HAL_PIN_Set(PAD_PA26, GPIO_A26, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA27, GPIO_A27, PIN_PULLUP, 1);
    HAL_Delay_us(2000);
    int cancel = HAL_GPIO_ReadPin(hwp_gpio1, 26) == GPIO_PIN_RESET;
    int shift = HAL_GPIO_ReadPin(hwp_gpio1, 27) == GPIO_PIN_RESET;
    return cancel && shift;
}

static int flag_set(void)
{
    if (HAL_Get_backup(RECOVERY_BKP) != RECOVERY_MAGIC)
        return 0;
    HAL_Set_backup(RECOVERY_BKP, 0);
    return 1;
}

static uint32_t crc32_of(const uint8_t *data, uint32_t len)
{
    uint32_t crc = 0xFFFFFFFFu;
    for (uint32_t i = 0; i < len; i++)
    {
        crc ^= data[i];
        for (int b = 0; b < 8; b++)
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
    }
    return ~crc;
}

/* ---- the application's USB device ------------------------------------------ */

/* Captured from the running application (pyusb, 2026-10-03): USB 2.1 with a
 * BOS, class 0/0/0, EP0 64, release 1.1, strings 1/2/3. */
static const uint8_t device_descriptor[] = {
    0x12, 0x01, 0x10, 0x02, 0x00, 0x00, 0x00, 0x40,
    (uint8_t)(USBD_VID & 0xFF), (uint8_t)(USBD_VID >> 8),
    (uint8_t)(USBD_PID & 0xFF), (uint8_t)(USBD_PID >> 8),
    0x01, 0x01, 0x01, 0x02, 0x03, 0x01,
};

/* One HID 1.10 interface, interrupt IN 0x81 and OUT 0x02 of 64 bytes at
 * 1 ms, bus powered, 100 mA. */
static const uint8_t config_descriptor[] = {
    0x09, 0x02, 0x29, 0x00, 0x01, 0x01, 0x00, 0x80, 0x32,
    0x09, 0x04, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00,
    0x09, 0x21, 0x10, 0x01, 0x00, 0x01, 0x22, 0x1D, 0x00,
    0x07, 0x05, HID_IN_EP, 0x03, 0x40, 0x00, 0x01,
    0x07, 0x05, HID_OUT_EP, 0x03, 0x40, 0x00, 0x01,
};

/* wire::REPORT_DESC: report id 6, 63 bytes in, 63 bytes out. */
static const uint8_t report_descriptor[29] = {
    0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x06, 0x15, 0x00,
    0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x3F, 0x09, 0x01, 0x81, 0x02,
    0x95, 0x3F, 0x09, 0x02, 0x91, 0x02, 0xC0,
};

static const uint8_t bos_bytes[] = {
    0x05, 0x0F, 0x0C, 0x00, 0x01, 0x07, 0x10, 0x02, 0x00, 0x00, 0x00, 0x00,
};
static const struct usb_bos_descriptor bos_desc = {
    .string = bos_bytes,
    .string_len = sizeof(bos_bytes),
};

static const uint8_t device_quality_descriptor[] = {
    0x0a, USB_DESCRIPTOR_TYPE_DEVICE_QUALIFIER, 0x10, 0x02, 0x00, 0x00, 0x00, 0x40, 0x01, 0x00,
};

/* The application's serial: its random-static Bluetooth address, folded out
 * of the 16-byte chip UID (src/board.rs), as upper-case hex. */
static char serial_text[13] = "000000000000";

static void derive_serial(void)
{
    static const char digits[] = "0123456789ABCDEF";
    uint8_t uid[16] = { 0 };
    uint8_t addr[6] = { 0xc2, 0x53, 0x46, 0x32, 0x52, 0xc0 };
    if (HAL_EFUSE_Read(0, uid, sizeof(uid)) == (int32_t)sizeof(uid))
    {
        for (int i = 0; i < 16; i++)
            addr[i % 6] ^= uid[i];
    }
    addr[5] |= 0xc0;
    for (int i = 0; i < 6; i++)
    {
        serial_text[2 * i] = digits[addr[i] >> 4];
        serial_text[2 * i + 1] = digits[addr[i] & 15];
    }
    serial_text[12] = 0;
}

static const char *string_descriptors[] = {
    (const char[]){ 0x09, 0x04 },     /* Langid */
    "Cohdl",                          /* Manufacturer */
    "OpenWalkieTalkie",               /* Product */
    serial_text,                      /* Serial Number */
};

static const uint8_t *device_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_descriptor;
}

static const uint8_t *config_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return config_descriptor;
}

static const uint8_t *device_quality_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_quality_descriptor;
}

static const char *string_descriptor_callback(uint8_t speed, uint8_t index)
{
    (void)speed;
    if (index >= (sizeof(string_descriptors) / sizeof(char *)))
        return NULL;
    return string_descriptors[index];
}

static const struct usb_descriptor hid_descriptor = {
    .device_descriptor_callback = device_descriptor_callback,
    .config_descriptor_callback = config_descriptor_callback,
    .device_quality_descriptor_callback = device_quality_descriptor_callback,
    .string_descriptor_callback = string_descriptor_callback,
    .bos_descriptor = &bos_desc,
};

/* ---- reports --------------------------------------------------------------- */

static struct usbd_interface intf0;
static uint8_t rx_report[REPORT_LEN];
static uint8_t tx_report[REPORT_LEN];
static volatile uint32_t rx_pending;     /* bytes of an unread OUT report */
static volatile int tx_busy;
static volatile int configured;
static volatile int talking;             /* a recovery-protocol report came in */
static volatile uint32_t ticks_ms;
static volatile uint32_t reboot_at_ms;   /* 0 = not scheduled */

static uint8_t sector_buf[SECTOR];
static uint8_t sector0[SECTOR];
static uint32_t sector0_len;
static int sector0_held;
static int session;

static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    if (event == USBD_EVENT_CONFIGURED)
    {
        configured = 1;
        rx_pending = 0;
        tx_busy = 0;
        usbd_ep_start_read(busid, HID_OUT_EP, rx_report, REPORT_LEN);
    }
}

/* The report is handed to the main loop; the endpoint is re-armed there. */
static void hid_out_callback(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid;
    (void)ep;
    rx_pending = nbytes ? nbytes : 1;
}

static void hid_in_callback(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid;
    (void)ep;
    (void)nbytes;
    tx_busy = 0;
}

static struct usbd_endpoint hid_in_ep = { .ep_addr = HID_IN_EP, .ep_cb = hid_in_callback };
static struct usbd_endpoint hid_out_ep = { .ep_addr = HID_OUT_EP, .ep_cb = hid_out_callback };

/* One channel-9 reply: `payload` of `len` bytes after the op byte. */
static void reply(uint8_t op, const uint8_t *payload, uint32_t len)
{
    uint32_t waited = 0;
    while (tx_busy && waited < 500)
    {
        HAL_Delay_us(1000);
        waited++;
    }
    if (len > PAYLOAD_MAX - 1)
        len = PAYLOAD_MAX - 1;
    memset(tx_report, 0, sizeof(tx_report));
    tx_report[0] = REPORT_ID;
    tx_report[1] = CHANNEL;
    tx_report[2] = (uint8_t)(len + 1);
    tx_report[3] = op;
    if (len)
        memcpy(&tx_report[4], payload, len);
    tx_busy = 1;
    usbd_ep_start_write(0, HID_IN_EP, tx_report, REPORT_LEN);
}

static uint32_t rd32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static void wr32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

/* Erase one sector, program `len` bytes there and read them back. */
static uint8_t program_sector(uint32_t addr, const uint8_t *data, uint32_t len)
{
    if (g_flash_erase(addr, SECTOR) != 0)
        return ST_FLASH;
    if (len && (uint32_t)g_flash_write(addr, (const int8_t *)data, len) != len)
        return ST_FLASH;
    if (len && memcmp((const void *)addr, data, len) != 0)
        return ST_VERIFY;
    return ST_OK;
}

static void handle_report(const uint8_t *rep, uint32_t n)
{
    if (n < 4 || rep[0] != REPORT_ID || rep[1] != CHANNEL)
        return;             /* the deck host's traffic, or noise */
    uint32_t len = rep[2];
    if (len > PAYLOAD_MAX || len + 3 > n)
        return;
    const uint8_t *p = &rep[3];
    uint8_t out[PAYLOAD_MAX];
    talking = 1;
    switch (p[0])
    {
    case OP_HELLO:
        out[0] = 'O'; out[1] = 'W'; out[2] = 'T'; out[3] = 'B';
        out[4] = PROTOCOL;
        out[5] = (uint8_t)app_valid();
        reply(OP_HELLO, out, 6);
        break;
    case OP_BEGIN:
        /* The vector table goes first and comes back last: an interrupted
         * download cannot be booted. */
        out[0] = g_flash_erase(APP_BASE, SECTOR) == 0 ? ST_OK : ST_FLASH;
        sector0_len = 0;
        sector0_held = 0;
        session = out[0] == ST_OK;
        say("OWTB: download started\n");
        reply(OP_BEGIN, out, 1);
        break;
    case OP_DATA:
    {
        if (len < 4)
            return;
        uint32_t off = (uint32_t)p[1] | ((uint32_t)p[2] << 8);
        uint32_t cnt = p[3];
        if (cnt > len - 4 || off + cnt > SECTOR)
            return;
        memcpy(&sector_buf[off], &p[4], cnt);
        break;
    }
    case OP_COMMIT:
    {
        uint32_t addr = rd32(&p[1]);
        uint32_t cnt = (uint32_t)p[5] | ((uint32_t)p[6] << 8);
        uint32_t crc = rd32(&p[7]);
        uint8_t status = ST_OK;
        if (len < 11 || !session || cnt == 0 || cnt > SECTOR || (addr & (SECTOR - 1))
            || addr < APP_BASE || addr + cnt > APP_END)
            status = ST_ARGS;
        else if (crc32_of(sector_buf, cnt) != crc)
            status = ST_CRC;
        else if (addr == APP_BASE)
        {
            memcpy(sector0, sector_buf, cnt);
            sector0_len = cnt;
            sector0_held = 1;
        }
        else
            status = program_sector(addr, sector_buf, cnt);
        out[0] = status;
        wr32(&out[1], addr);
        reply(OP_COMMIT, out, 5);
        break;
    }
    case OP_FINISH:
    {
        uint8_t status = ST_OK;
        if (!session)
            status = ST_ARGS;
        else if (sector0_held)
            status = program_sector(APP_BASE, sector0, sector0_len);
        sector0_held = 0;
        session = 0;
        int valid = app_valid();
        out[0] = status;
        out[1] = (uint8_t)valid;
        say(valid ? "OWTB: image written, vector table valid\n"
                  : "OWTB: image written, vector table INVALID\n");
        reply(OP_FINISH, out, 2);
        if (status == ST_OK && valid)
            reboot_at_ms = ticks_ms + 500;
        break;
    }
    case OP_BOOT:
        out[0] = ST_OK;
        reply(OP_BOOT, out, 1);
        reboot_at_ms = ticks_ms + 300;
        break;
    case OP_READ:
    {
        uint32_t addr = rd32(&p[1]);
        uint32_t cnt = p[5];
        if (len < 6 || cnt > PAYLOAD_MAX - 2 || addr < APP_BASE || addr + cnt > APP_END)
        {
            out[0] = 0;
            reply(OP_READ, out, 1);
            break;
        }
        out[0] = (uint8_t)cnt;
        memcpy(&out[1], (const void *)addr, cnt);
        reply(OP_READ, out, cnt + 1);
        break;
    }
    default:
        break;
    }
}

/* 60 MHz for the USB block: sysclk from DLL1 at 240 MHz, divided by 4, as
 * the application firmware runs it. */
static void usb_clocks(void)
{
    HAL_RCC_HCPU_ConfigHCLK(240);
    hwp_hpsys_rcc->USBCR = 4;
    hwp_hpsys_rcc->CSR &= ~HPSYS_RCC_CSR_SEL_USBC;
    HAL_Delay_us(0);
}

static void recovery_run(const char *why, int app_is_valid)
{
    derive_serial();
    say("OWTB: recovery mode (");
    say(why);
    say("); USB HID 1209:5754 serial ");
    say(serial_text);
    say("\n");
    usb_clocks();
    HAL_PIN_Set_Analog(PAD_PA35, 1);   /* USB_DP */
    HAL_PIN_Set_Analog(PAD_PA36, 1);   /* USB_DM */
    usbd_desc_register(0, &hid_descriptor);
    usbd_add_interface(0, usbd_hid_init_intf(0, &intf0, report_descriptor, sizeof(report_descriptor)));
    usbd_add_endpoint(0, &hid_in_ep);
    usbd_add_endpoint(0, &hid_out_ep);
    usbd_initialize(0, USBC_BASE, usbd_event_handler);

    uint32_t waited = 0;
    for (;;)
    {
        if (rx_pending)
        {
            uint32_t n = rx_pending;
            rx_pending = 0;
            handle_report(rx_report, n);
            usbd_ep_start_read(0, HID_OUT_EP, rx_report, REPORT_LEN);
            continue;
        }
        HAL_Delay_us(1000);
        ticks_ms++;
        if (reboot_at_ms && (int32_t)(ticks_ms - reboot_at_ms) >= 0)
        {
            say("OWTB: rebooting\n");
            HAL_PMU_Reboot();
        }
        if (!talking && app_is_valid && ++waited >= HOST_WAIT_MS)
        {
            say("OWTB: nobody asked for a download; booting the application\n");
            HAL_PMU_Reboot();
        }
    }
}

/* The SDK's boot_images_help() has read the flash table and chosen the
 * application image when it calls board_init_psram(); its very next step is
 * the jump. The decision is made here: recovery, or on to the application. */
void __wrap_board_init_psram(void)
{
    say("OWTB: OpenWalkieTalkie bootloader 1 at ");
    say_hex(FLASH_BOOT_LOADER_EXEC_START_ADDR);
    say("\n");
    int valid = app_valid();
    const char *why = NULL;
    if (flag_set())
        why = "requested by the application";
    else if (buttons_held())
        why = "SHIFT+CANCEL held at power-on";
    else if (!valid)
        why = "no valid application image";
    if (why)
        recovery_run(why, valid);   /* never returns */
    say("OWTB: booting the application\n");
    __real_board_init_psram();
}
