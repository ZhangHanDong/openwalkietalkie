/* SPDX-License-Identifier: MIT */
#include <string.h>
#include <rtthread.h>
#include "mem_section.h"

/* BSP_Flash_hw2_init's aggregate initialization emits a memset before the NOR
 * clock divider has been restored. The SDK keeps newlib's *memset*.o in SRAM,
 * but Rust compiler_builtins supplies a weak memset from a differently named
 * object, which otherwise lands in XIP flash. Use RT-Thread's existing SRAM
 * implementation, and check both symbols in check-native-image.py. */
L1_RET_CODE_SECT(memset, void *memset(void *destination, int value, size_t length))
{
    return rt_memset(destination, value, length);
}
