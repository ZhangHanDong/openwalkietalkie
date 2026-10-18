/* SPDX-License-Identifier: MIT */
#ifndef OWT_RECOVERY_UI_H
#define OWT_RECOVERY_UI_H
#include <stdbool.h>
#include <stdint.h>
#include "flash_layout.h"

#define OWT_SCREEN_SIZE 480u
#define OWT_SCREEN_BAND 16u
#define OWT_RECOVERY_SECTOR 4096u

enum owt_recovery_phase { OWT_RECOVERY_READY, OWT_RECOVERY_MISSING,
    OWT_RECOVERY_WRITING, OWT_RECOVERY_COMPLETE, OWT_RECOVERY_ERROR };
struct owt_recovery_ui {
    enum owt_recovery_phase phase;
    uint32_t image_size, total, received;
    uint8_t error;
    /* Acknowledged sectors, including the vector sector held until FINISH.
     * Retransmitting a sector must not advance the progress bar twice. */
    uint8_t sectors[(OWT_FLASH_END - OWT_APP_BASE) / OWT_RECOVERY_SECTOR / 8];
};

void owt_recovery_ui_init(struct owt_recovery_ui *ui, bool valid);
/* size=0 is an old client without a declared image size. */
bool owt_recovery_ui_begin(struct owt_recovery_ui *ui, uint32_t size);
bool owt_recovery_ui_accepts(const struct owt_recovery_ui *ui, uint32_t addr, uint32_t len);
void owt_recovery_ui_committed(struct owt_recovery_ui *ui, uint32_t addr, uint32_t len);
bool owt_recovery_ui_received(const struct owt_recovery_ui *ui);
void owt_recovery_ui_error(struct owt_recovery_ui *ui, uint8_t error);
unsigned owt_recovery_ui_percent(const struct owt_recovery_ui *ui);
/* Draw one complete logical band; rotate the pixels for Rev A/B's mounting. */
void owt_recovery_ui_render(const struct owt_recovery_ui *ui, unsigned y,
                            bool rotate, uint16_t *pixels);

/* The hardware driver never reads the application or uses PSRAM. Display
 * faults disable drawing for this boot; USB recovery stays available. */
void owt_recovery_display_init(const struct owt_recovery_ui *ui);
void owt_recovery_display_update(const struct owt_recovery_ui *ui);
#endif
