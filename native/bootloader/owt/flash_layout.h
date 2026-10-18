#ifndef OWT_FLASH_LAYOUT_H
#define OWT_FLASH_LAYOUT_H
#include <stdint.h>

#define OWT_APP_BASE     0x12020000u
#define OWT_BONDS_START  0x12700000u
#define OWT_BONDS_END    0x12720000u
#define OWT_FLASH_END    0x13000000u
#define OWT_PROTOCOL     2

/* Overflow-safe bounds shared by vector, erase/program and read checks. */
static inline int owt_app_range(uint32_t address, uint32_t length)
{
    return length && address >= OWT_APP_BASE && address < OWT_FLASH_END
        && length <= OWT_FLASH_END - address
        && (address + length <= OWT_BONDS_START || address >= OWT_BONDS_END);
}
#endif
