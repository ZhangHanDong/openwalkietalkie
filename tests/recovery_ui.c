/* SPDX-License-Identifier: MIT */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "../native/bootloader/owt/recovery_ui.h"

static void progress(void)
{
    struct owt_recovery_ui ui;
    owt_recovery_ui_init(&ui, true);
    assert(ui.phase == OWT_RECOVERY_READY);
    owt_recovery_ui_init(&ui, false);
    assert(ui.phase == OWT_RECOVERY_MISSING);
    assert(owt_recovery_ui_begin(&ui, 4200));
    assert(!owt_recovery_ui_received(&ui));
    assert(owt_recovery_ui_percent(&ui) == 0);
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE, 42));
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE + 4096, 4096));
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE + 8192, 1));
    /* The final short sector can arrive first; sector0 is only held in SRAM. */
    owt_recovery_ui_committed(&ui, OWT_APP_BASE + 4096, 104);
    assert(ui.received == 104 && owt_recovery_ui_percent(&ui) == 2);
    owt_recovery_ui_committed(&ui, OWT_APP_BASE + 4096, 104);
    assert(ui.received == 104);
    owt_recovery_ui_error(&ui, 2);
    assert(ui.phase == OWT_RECOVERY_ERROR && ui.error == 2);
    owt_recovery_ui_committed(&ui, OWT_APP_BASE, 4096);
    assert(ui.phase == OWT_RECOVERY_WRITING && !ui.error);
    assert(owt_recovery_ui_received(&ui));
    assert(owt_recovery_ui_percent(&ui) == 99);
    ui.phase = OWT_RECOVERY_COMPLETE;
    assert(owt_recovery_ui_percent(&ui) == 100);
    assert(owt_recovery_ui_begin(&ui, 8)); /* restart resets acknowledgments */
    assert(!ui.received && !ui.sectors[0]);

    const uint32_t bad[] = {1, 7, 0xffffffff, OWT_FLASH_END - OWT_APP_BASE + 1,
        OWT_BONDS_START - OWT_APP_BASE + 1, OWT_BONDS_END - OWT_APP_BASE};
    struct owt_recovery_ui before = ui;
    for (unsigned i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
        assert(!owt_recovery_ui_begin(&ui, bad[i]));
        assert(!memcmp(&ui, &before, sizeof(ui)));
    }
    assert(owt_recovery_ui_begin(&ui, OWT_FLASH_END - OWT_APP_BASE));
    assert(ui.total == 16u * 1024 * 1024 - 256u * 1024);
    for (uint32_t addr = OWT_APP_BASE; addr < OWT_FLASH_END; addr += 4096) {
        bool valid = addr < OWT_BONDS_START || addr >= OWT_BONDS_END;
        assert(owt_recovery_ui_accepts(&ui, addr, 4096) == valid);
        owt_recovery_ui_committed(&ui, addr, 4096);
        owt_recovery_ui_committed(&ui, addr, 4096);
    }
    assert(ui.received == ui.total && owt_recovery_ui_received(&ui));
    assert(owt_recovery_ui_percent(&ui) == 99);
    assert(!owt_recovery_ui_accepts(&ui, OWT_FLASH_END, 4096));
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE + 1, 4096));
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE, 0));
    assert(!owt_recovery_ui_accepts(&ui, OWT_APP_BASE, 4097));

    /* Legacy client: show receiving data, never a fabricated percentage. */
    assert(owt_recovery_ui_begin(&ui, 0));
    owt_recovery_ui_committed(&ui, OWT_APP_BASE, 4096);
    assert(!ui.total && ui.received == 4096);
    assert(owt_recovery_ui_percent(&ui) == 0);
    assert(owt_recovery_ui_received(&ui));
}

static void rendering(void)
{
    enum { N = OWT_SCREEN_SIZE * OWT_SCREEN_BAND };
    uint16_t normal[N + 2], rotated[N + 2];
    struct owt_recovery_ui ui;
    owt_recovery_ui_begin(&ui, 100000);
    for (unsigned phase = OWT_RECOVERY_READY; phase <= OWT_RECOVERY_ERROR; phase++) {
        ui.phase = phase;
        for (unsigned percent = 0; percent <= 100; percent++) {
            ui.received = percent * 1000;
            ui.error = percent % 6;
            for (unsigned y = 0; y < OWT_SCREEN_SIZE; y += OWT_SCREEN_BAND) {
                normal[0] = rotated[0] = normal[N + 1] = rotated[N + 1] = 0xdead;
                owt_recovery_ui_render(&ui, y, false, normal + 1);
                owt_recovery_ui_render(&ui, y, true, rotated + 1);
                assert(normal[0] == 0xdead && rotated[0] == 0xdead);
                assert(normal[N + 1] == 0xdead && rotated[N + 1] == 0xdead);
                for (unsigned i = 0; i < N; i++) assert(normal[i + 1] == rotated[N - i]);
                /* The 480px round display must not clip any foreground. */
                for (unsigned yy = 0; yy < OWT_SCREEN_BAND; yy++)
                    for (unsigned x = 0; x < OWT_SCREEN_SIZE; x++) {
                        int dx = (int)x - 240, dy = (int)(y + yy) - 240;
                        if (dx * dx + dy * dy > 236 * 236)
                            assert(normal[1 + yy * OWT_SCREEN_SIZE + x] == 0x0842);
                    }
            }
        }
    }
}

/* Optional contact sheet of the actual bootloader renderer, upright. */
static void preview(const char *path)
{
    FILE *f = fopen(path, "wb"); assert(f);
    fprintf(f, "P6\n1920 480\n255\n");
    uint16_t pixels[4][OWT_SCREEN_SIZE * OWT_SCREEN_BAND];
    struct owt_recovery_ui ui;
    owt_recovery_ui_begin(&ui, 100000);
    ui.received = 63000;
    for (unsigned y = 0; y < OWT_SCREEN_SIZE; y += OWT_SCREEN_BAND) {
        for (unsigned scene = 0; scene < 4; scene++) {
            ui.phase = scene == 0 ? OWT_RECOVERY_READY : scene + 1;
            ui.error = 2;
            owt_recovery_ui_render(&ui, y, false, pixels[scene]);
        }
        for (unsigned row = 0; row < OWT_SCREEN_BAND; row++)
            for (unsigned scene = 0; scene < 4; scene++)
                for (unsigned x = 0; x < OWT_SCREEN_SIZE; x++) {
                    uint16_t p = pixels[scene][row * OWT_SCREEN_SIZE + x];
                    int dx = (int)x - 240, dy = (int)(y + row) - 240;
                    if (dx * dx + dy * dy >= 240 * 240) p = 0;
                    fputc(((p >> 11) & 31) * 255 / 31, f);
                    fputc(((p >> 5) & 63) * 255 / 63, f);
                    fputc((p & 31) * 255 / 31, f);
                }
    }
    assert(!fclose(f));
}

int main(int argc, char **argv)
{
    progress(); rendering();
    if (argc == 2) preview(argv[1]);
    puts("recovery UI: full flash, retry, incomplete/legacy transfers, rotation and round-screen bounds passed");
}
