/* SPDX-License-Identifier: MIT */
#include "recovery_ui.h"
#include <string.h>

void owt_recovery_ui_init(struct owt_recovery_ui *ui, bool valid)
{
    memset(ui, 0, sizeof(*ui));
    ui->phase = valid ? OWT_RECOVERY_READY : OWT_RECOVERY_MISSING;
}

bool owt_recovery_ui_begin(struct owt_recovery_ui *ui, uint32_t size)
{
    uint32_t gap_start = OWT_BONDS_START - OWT_APP_BASE;
    uint32_t gap_end = OWT_BONDS_END - OWT_APP_BASE;
    if (size && (size < 8 || size > OWT_FLASH_END - OWT_APP_BASE
                 || (size > gap_start && size <= gap_end)))
        return false;
    memset(ui, 0, sizeof(*ui));
    ui->phase = OWT_RECOVERY_WRITING;
    ui->image_size = size;
    ui->total = size - (size > gap_end ? gap_end - gap_start : 0);
    return true;
}

bool owt_recovery_ui_accepts(const struct owt_recovery_ui *ui, uint32_t addr, uint32_t len)
{
    if (!len || len > OWT_RECOVERY_SECTOR || (addr & (OWT_RECOVERY_SECTOR - 1))
        || !owt_app_range(addr, OWT_RECOVERY_SECTOR))
        return false;
    if (!ui->image_size) return true;
    uint32_t offset = addr - OWT_APP_BASE;
    if (offset >= ui->image_size) return false;
    uint32_t expected = ui->image_size - offset;
    if (expected > OWT_RECOVERY_SECTOR) expected = OWT_RECOVERY_SECTOR;
    return len == expected;
}

void owt_recovery_ui_committed(struct owt_recovery_ui *ui, uint32_t addr, uint32_t len)
{
    if (!owt_recovery_ui_accepts(ui, addr, len)) return;
    unsigned sector = (addr - OWT_APP_BASE) / OWT_RECOVERY_SECTOR;
    uint8_t mask = 1u << (sector % 8);
    if (!(ui->sectors[sector / 8] & mask)) {
        ui->sectors[sector / 8] |= mask;
        ui->received += len;
    }
    ui->phase = OWT_RECOVERY_WRITING;
    ui->error = 0;
}

bool owt_recovery_ui_received(const struct owt_recovery_ui *ui)
{
    return !ui->total || ui->received == ui->total;
}

void owt_recovery_ui_error(struct owt_recovery_ui *ui, uint8_t error)
{
    ui->phase = OWT_RECOVERY_ERROR;
    ui->error = error;
}

unsigned owt_recovery_ui_percent(const struct owt_recovery_ui *ui)
{
    if (ui->phase == OWT_RECOVERY_COMPLETE) return 100;
    if (!ui->total) return 0;
    unsigned p = ui->received * 100u / ui->total;
    return p < 100 ? p : 99; /* FINISH must write and validate the vectors. */
}

/* Small, self-contained 5x7 uppercase alphabet. No application font assets
 * are readable assumptions while the application is being erased. */
static const uint8_t letters[][5] = {
    {0x7e,0x11,0x11,0x11,0x7e}, {0x7f,0x49,0x49,0x49,0x36},
    {0x3e,0x41,0x41,0x41,0x22}, {0x7f,0x41,0x41,0x22,0x1c},
    {0x7f,0x49,0x49,0x49,0x41}, {0x7f,0x09,0x09,0x09,0x01},
    {0x3e,0x41,0x49,0x49,0x7a}, {0x7f,0x08,0x08,0x08,0x7f},
    {0x00,0x41,0x7f,0x41,0x00}, {0x20,0x40,0x41,0x3f,0x01},
    {0x7f,0x08,0x14,0x22,0x41}, {0x7f,0x40,0x40,0x40,0x40},
    {0x7f,0x02,0x0c,0x02,0x7f}, {0x7f,0x04,0x08,0x10,0x7f},
    {0x3e,0x41,0x41,0x41,0x3e}, {0x7f,0x09,0x09,0x09,0x06},
    {0x3e,0x41,0x51,0x21,0x5e}, {0x7f,0x09,0x19,0x29,0x46},
    {0x46,0x49,0x49,0x49,0x31}, {0x01,0x01,0x7f,0x01,0x01},
    {0x3f,0x40,0x40,0x40,0x3f}, {0x1f,0x20,0x40,0x20,0x1f},
    {0x3f,0x40,0x38,0x40,0x3f}, {0x63,0x14,0x08,0x14,0x63},
    {0x07,0x08,0x70,0x08,0x07}, {0x61,0x51,0x49,0x45,0x43},
};
static const uint8_t digits[][5] = {
    {0x3e,0x51,0x49,0x45,0x3e}, {0x00,0x42,0x7f,0x40,0x00},
    {0x42,0x61,0x51,0x49,0x46}, {0x21,0x41,0x45,0x4b,0x31},
    {0x18,0x14,0x12,0x7f,0x10}, {0x27,0x45,0x45,0x45,0x39},
    {0x3c,0x4a,0x49,0x49,0x30}, {0x01,0x71,0x09,0x05,0x03},
    {0x36,0x49,0x49,0x49,0x36}, {0x06,0x49,0x49,0x29,0x1e},
};
static const uint8_t percent[] = {0x63,0x13,0x08,0x64,0x63};
static const uint8_t dash[] = {0x08,0x08,0x08,0x08,0x08};

static void rect(uint16_t *pixels, unsigned band_y, unsigned x, unsigned y,
                 unsigned width, unsigned height, uint16_t color)
{
    for (unsigned yy = y; yy < y + height && yy < band_y + OWT_SCREEN_BAND; yy++) {
        if (yy < band_y) continue;
        for (unsigned xx = x; xx < x + width && xx < OWT_SCREEN_SIZE; xx++)
            pixels[(yy - band_y) * OWT_SCREEN_SIZE + xx] = color;
    }
}

static void text(uint16_t *pixels, unsigned band_y, unsigned y, unsigned scale,
                 const char *s, uint16_t color)
{
    unsigned width = (unsigned)strlen(s) * 6u * scale - scale;
    unsigned x = (OWT_SCREEN_SIZE - width) / 2;
    if (y >= band_y + OWT_SCREEN_BAND || y + 7 * scale <= band_y) return;
    for (; *s; s++, x += 6 * scale) {
        const uint8_t *glyph = *s >= 'A' && *s <= 'Z' ? letters[*s - 'A']
            : *s >= '0' && *s <= '9' ? digits[*s - '0']
            : *s == '%' ? percent : *s == '-' ? dash : NULL;
        if (!glyph) continue;
        for (unsigned col = 0; col < 5; col++)
            for (unsigned row = 0; row < 7; row++)
                if (glyph[col] & (1u << row))
                    rect(pixels, band_y, x + col * scale, y + row * scale, scale, scale, color);
    }
}

void owt_recovery_ui_render(const struct owt_recovery_ui *ui, unsigned y,
                            bool rotate, uint16_t *pixels)
{
    const uint16_t bg = 0x0842, white = 0xef7d, muted = 0x94b3;
    uint16_t accent = ui->phase == OWT_RECOVERY_ERROR ? 0xfa89
        : ui->phase == OWT_RECOVERY_COMPLETE ? 0x47b3 : 0x4e9f;
    for (unsigned i = 0; i < OWT_SCREEN_SIZE * OWT_SCREEN_BAND; i++) pixels[i] = bg;
    text(pixels, y, 96, 2, "OPENWALKIETALKIE", muted);
    rect(pixels, y, 212, 132, 56, 3, accent);
    const char *title = "RECOVERY MODE", *detail = "OPEN THE DESKTOP APP";
    switch (ui->phase) {
    case OWT_RECOVERY_MISSING: detail = "FIRMWARE NEEDS REPAIR"; break;
    case OWT_RECOVERY_WRITING: title = "UPDATING FIRMWARE"; detail = "DO NOT DISCONNECT"; break;
    case OWT_RECOVERY_COMPLETE: title = "UPDATE COMPLETE"; detail = "RESTARTING DEVICE"; break;
    case OWT_RECOVERY_ERROR: title = "UPDATE FAILED"; detail = "RETRY IN THE DESKTOP APP"; break;
    default: break;
    }
    text(pixels, y, 162, 3, title, white);
    if (ui->phase == OWT_RECOVERY_WRITING || ui->phase == OWT_RECOVERY_COMPLETE) {
        unsigned p = owt_recovery_ui_percent(ui);
        char label[5];
        unsigned n = 0;
        if (p >= 100) label[n++] = '1';
        if (p >= 10) label[n++] = '0' + (p / 10) % 10;
        label[n++] = '0' + p % 10;
        label[n++] = '%'; label[n] = 0;
        if (ui->total || ui->phase == OWT_RECOVERY_COMPLETE)
            text(pixels, y, 218, 4, label, accent);
        else
            text(pixels, y, 226, 2, "RECEIVING DATA", accent);
        rect(pixels, y, 96, 274, 288, 12, 0x2126);
        rect(pixels, y, 96, 274, 288 * p / 100, 12, accent);
    } else {
        const char *status = "USB-C RECOVERY";
        if (ui->phase == OWT_RECOVERY_ERROR)
            status = ui->error == 2 ? "DATA CHECK FAILED"
                : ui->error == 3 ? "FLASH WRITE FAILED"
                : ui->error == 4 ? "VERIFY FAILED"
                : ui->error == 5 ? "USB CONNECTION LOST" : "INCOMPLETE TRANSFER";
        text(pixels, y, 236, 2, status, accent);
    }
    text(pixels, y, 322, 2, detail, white);
    text(pixels, y, 356, 2, "KEEP USB-C CONNECTED", muted);
    if (rotate) {
        unsigned count = OWT_SCREEN_SIZE * OWT_SCREEN_BAND;
        for (unsigned i = 0; i < count / 2; i++) {
            uint16_t p = pixels[i]; pixels[i] = pixels[count - i - 1]; pixels[count - i - 1] = p;
        }
    }
}
