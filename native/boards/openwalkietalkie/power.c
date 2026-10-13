/* SPDX-License-Identifier: MIT */
#include "bsp_board.h"

void BSP_GPIO_Set(int pin, int value, int is_porta)
{
    GPIO_TypeDef *gpio = is_porta ? hwp_gpio1 : hwp_gpio2;
    GPIO_InitTypeDef config = { .Pin = pin, .Mode = GPIO_MODE_OUTPUT, .Pull = GPIO_NOPULL };
    HAL_GPIO_WritePin(gpio, pin, value ? GPIO_PIN_SET : GPIO_PIN_RESET);
    HAL_GPIO_Init(gpio, &config);
}

void BSP_Power_Up(bool is_deep_sleep)
{
    (void)is_deep_sleep;
#ifdef SOC_BF0_HCPU
#ifdef OWT_BOARD_HW_A
    /* Rev A: U10 switches PANEL_LOGIC (PA11) and U8 makes PANEL_4V (PA10);
     * both rails are off until driven. Logic first, then the 4 V rail, with
     * time for the load switch (about 2 ms at its 1 nF slew capacitor) to
     * finish before the AMOLED supply starts, as the Embassy board does. */
    BSP_GPIO_Set(11, 1, 1);
    HAL_Delay_us(5000);
    BSP_GPIO_Set(10, 1, 1);
#else
    BSP_GPIO_Set(41, 1, 1); /* Latch display-unit system power. */
    BSP_GPIO_Set(44, 1, 1); /* AMOLED supply. */
#endif
#endif
}

/* Keep board rails enabled until their resume paths have been verified. */
void BSP_IO_Power_Down(int coreid, bool is_deep_sleep)
{
    (void)coreid;
    (void)is_deep_sleep;
}
