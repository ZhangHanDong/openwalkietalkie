#include <assert.h>
#include <stdio.h>
#include "../native/bootloader/owt/flash_layout.h"

int main(void)
{
    unsigned allowed = 0;
    for (uint32_t address = 0x12000000; address < OWT_FLASH_END; address += 4096) {
        int expected = address >= OWT_APP_BASE
            && (address < OWT_BONDS_START || address >= OWT_BONDS_END);
        assert(owt_app_range(address, 4096) == expected);
        allowed += expected;
    }
    assert(allowed * 4096u == 16u * 1024u * 1024u - 256u * 1024u);
    assert(!owt_app_range(OWT_BONDS_START - 1, 2));
    assert(!owt_app_range(OWT_BONDS_END - 1, 2));
    assert(!owt_app_range(OWT_APP_BASE, 0));
    assert(!owt_app_range(OWT_FLASH_END - 1, 2));
    assert(!owt_app_range(OWT_FLASH_END, 1));
    assert(!owt_app_range(0xfffff000u, 8192));
    assert(!owt_app_range(OWT_APP_BASE, 0xffffffffu));
    assert(owt_app_range(OWT_APP_BASE, OWT_BONDS_START - OWT_APP_BASE));
    assert(owt_app_range(OWT_BONDS_END, OWT_FLASH_END - OWT_BONDS_END));
    puts("flash bounds: all 4096 sectors checked; both bond stores preserved");
}
