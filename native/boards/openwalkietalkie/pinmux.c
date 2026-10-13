/*
 * SPDX-FileCopyrightText: 2019-2022 SiFli Technologies(Nanjing) Co., Ltd
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include "bsp_board.h"

#ifdef BSP_USING_PSRAM1
/* APS 128p*/
static void board_pinmux_psram_func0()
{
    HAL_PIN_Set(PAD_SA01, MPI1_DIO0, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA02, MPI1_DIO1, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA03, MPI1_DIO2, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA04, MPI1_DIO3, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA05, MPI1_DIO4, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA06, MPI1_DIO5, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA07, MPI1_DIO6, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA08, MPI1_DIO7, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA09, MPI1_DQSDM, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA10, MPI1_CLK,  PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_SA11, MPI1_CS,   PIN_NOPULL, 1);

    HAL_PIN_Set_Analog(PAD_SA00, 1);
    HAL_PIN_Set_Analog(PAD_SA12, 1);
}

/* APS 1:64p 2:32P, 4:Winbond 32/64/128p*/
static void board_pinmux_psram_func1_2_4(int func)
{
    HAL_PIN_Set(PAD_SA01, MPI1_DIO0, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA02, MPI1_DIO1, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA03, MPI1_DIO2, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA04, MPI1_DIO3, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA08, MPI1_DIO4, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA09, MPI1_DIO5, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA10, MPI1_DIO6, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA11, MPI1_DIO7, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA07, MPI1_CLK,  PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_SA05, MPI1_CS,   PIN_NOPULL, 1);

#ifdef FPGA
    HAL_PIN_Set(PAD_SA00, MPI1_DM, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA06, MPI1_CLKB, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_SA12, MPI1_DQSDM, PIN_PULLDOWN, 1);
#else
    switch (func)
    {
    case 1:             // APS 64P XCELLA
        HAL_PIN_Set(PAD_SA12, MPI1_DQSDM, PIN_PULLDOWN, 1);
        HAL_PIN_Set_Analog(PAD_SA00, 1);
        HAL_PIN_Set_Analog(PAD_SA06, 1);
        break;
    case 2:             // APS 32P LEGACY
        HAL_PIN_Set(PAD_SA00, MPI1_DM, PIN_PULLDOWN, 1);
        HAL_PIN_Set(PAD_SA12, MPI1_DQS, PIN_PULLDOWN, 1);
        HAL_PIN_Set(PAD_SA06, MPI1_CLKB, PIN_NOPULL, 1);
        break;
    case 4:             // Winbond 32/64/128p
        //HAL_PIN_Set(PAD_SA06, MPI1_CLKB, PIN_NOPULL, 1);
        HAL_PIN_Set(PAD_SA12, MPI1_DQSDM, PIN_NOPULL, 1);
        HAL_PIN_Set_Analog(PAD_SA00, 1);
        HAL_PIN_Set_Analog(PAD_SA06, 1);
        break;
    }
#endif
}

/* APS 16p*/
static void board_pinmux_psram_func3()
{
    HAL_PIN_Set(PAD_SA09, MPI1_CLK, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_SA08, MPI1_CS,  PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_SA05, MPI1_DIO0, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA07, MPI1_DIO1, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_SA06, MPI1_DIO2, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_SA10, MPI1_DIO3, PIN_PULLUP, 1);

    HAL_PIN_Set_Analog(PAD_SA00, 1);
    HAL_PIN_Set_Analog(PAD_SA01, 1);
    HAL_PIN_Set_Analog(PAD_SA02, 1);
    HAL_PIN_Set_Analog(PAD_SA03, 1);
    HAL_PIN_Set_Analog(PAD_SA04, 1);
    HAL_PIN_Set_Analog(PAD_SA11, 1);
    HAL_PIN_Set_Analog(PAD_SA12, 1);
}

static void board_pinmux_mpi1_none(void)
{
    uint32_t i;

    for (i = 0; i <= 12; i++)
    {
        HAL_PIN_Set_Analog(PAD_SA00 + i, 1);
    }
}
#endif

void BSP_PIN_Init(void)
{
#ifdef SOC_BF0_HCPU
#ifdef BSP_USING_PSRAM1
    unsigned pid = (hwp_hpsys_cfg->IDR & HPSYS_CFG_IDR_PID_Msk) >> HPSYS_CFG_IDR_PID_Pos;
    switch (pid & 7) {
    case 5: board_pinmux_psram_func3(); break;
    case 4: board_pinmux_psram_func1_2_4(2); break;
    case 6: board_pinmux_psram_func1_2_4(4); break;
    case 3: board_pinmux_psram_func1_2_4(1); break;
    case 2: board_pinmux_psram_func0(); break;
    default: board_pinmux_mpi1_none(); break;
    }
#endif
    HAL_PIN_Set(PAD_PA16, MPI2_CLK, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA12, MPI2_CS, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA15, MPI2_DIO0, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_PA13, MPI2_DIO1, PIN_PULLDOWN, 1);
    HAL_PIN_Set(PAD_PA14, MPI2_DIO2, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA17, MPI2_DIO3, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA18, USART1_RXD, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA19, USART1_TXD, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA20, GPIO_A20, PIN_PULLDOWN, 1); /* NS4150B speaker enable (Rev A; unconnected on Rev B) */
#ifdef OWT_BOARD_HW_A
    /* OpenWalkieTalkie Rev A (BodyV6.13 r6): the net list is
     * hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/source/main/src/main.cohdl.
     * The board carries every pull-up the inputs need (100k; I2C 10k). */
#ifdef OWT_BOARD_HW_B
    /* Rev B (hardware/rev-b/main/main-revb.kicad_pcb): the same map with three
     * pins moved. The amplifier enable is on PA01, the CAL test pad TP3 on
     * PA00 (parked), and the BQ24073's CHG output reaches PA33 (R46 100k
     * pull-up on the board; low while charging). */
    HAL_PIN_Set(PAD_PA01, GPIO_A1, PIN_PULLDOWN, 1);  /* AMP_ENABLE: NS4150B CTRL, off until driven */
    HAL_PIN_Set(PAD_PA00, GPIO_A0, PIN_PULLDOWN, 1);  /* CAL (TP3) */
    HAL_PIN_Set(PAD_PA33, GPIO_A33, PIN_NOPULL, 1);   /* CHG_STATUS */
#endif
    /* Both enables have 1M pull-downs on the board (R34, R36); no internal pull
     * to fight the high level BSP_Power_Up drives for the whole runtime. */
    HAL_PIN_Set(PAD_PA10, GPIO_A10, PIN_NOPULL, 1);   /* PANEL_ENABLE: U8 PANEL_4V */
    HAL_PIN_Set(PAD_PA11, GPIO_A11, PIN_NOPULL, 1);   /* PANEL_VCI_ENABLE: U10 PANEL_LOGIC */
    HAL_PIN_Set(PAD_PA09, GPIO_A9, PIN_NOPULL, 1);    /* PANEL_RESET, 100k pull-down on the board */
    HAL_PIN_Set(PAD_PA44, GPIO_A44, PIN_NOPULL, 1);   /* TOUCH_RESET */
    HAL_PIN_Set(PAD_PA43, GPIO_A43, PIN_NOPULL, 1);   /* TOUCH_IRQ */
    HAL_PIN_Set(PAD_PA42, I2C1_SCL, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA41, I2C1_SDA, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA24, GPIO_A24, PIN_NOPULL, 1);   /* PTT */
    HAL_PIN_Set(PAD_PA25, GPIO_A25, PIN_NOPULL, 1);   /* ENTER */
    HAL_PIN_Set(PAD_PA26, GPIO_A26, PIN_NOPULL, 1);   /* ESC */
    HAL_PIN_Set(PAD_PA27, GPIO_A27, PIN_NOPULL, 1);   /* SHIFT */
    HAL_PIN_Set(PAD_PA37, GPIO_A37, PIN_NOPULL, 1);   /* MACRO_LEFT */
    HAL_PIN_Set(PAD_PA38, GPIO_A38, PIN_NOPULL, 1);   /* MACRO_RIGHT */
    HAL_PIN_Set(PAD_PA39, GPIO_A39, PIN_NOPULL, 1);   /* ENC_A */
    HAL_PIN_Set(PAD_PA40, GPIO_A40, PIN_NOPULL, 1);   /* ENC_B */
    HAL_PIN_Set(PAD_PA28, GPIO_A28, PIN_PULLDOWN, 1); /* MIC_POWER_ENABLE: load switch for the MP34DT05, driven by board.c while the PDM block runs */
    /* PDM1 to the MP34DT05 (L/R tied low: left channel). PA22 doubles as
     * XTAL32K_XI, which this board does not fit (LXT_DISABLE). */
    HAL_PIN_Set(PAD_PA22, PDM1_CLK, PIN_NOPULL, 1);   /* MIC_PDM_CLK through 22 ohm */
    HAL_PIN_Set(PAD_PA23, PDM1_DATA, PIN_PULLDOWN, 1); /* MIC_PDM_DATA, 100k pull-down on the board too */
#else
    HAL_PIN_Set(PAD_PA41, GPIO_A41, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA44, GPIO_A44, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA09, GPIO_A9, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA00, GPIO_A0, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA01, GPIO_A1, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA11, GPIO_A11, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA38, I2C1_SCL, PIN_PULLUP, 1);
    HAL_PIN_Set(PAD_PA37, I2C1_SDA, PIN_PULLUP, 1);
#endif
    HAL_PIN_Set(PAD_PA02, LCDC1_SPI_TE, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA03, LCDC1_SPI_CS, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA04, LCDC1_SPI_CLK, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA05, LCDC1_SPI_DIO0, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA06, LCDC1_SPI_DIO1, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA07, LCDC1_SPI_DIO2, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA08, LCDC1_SPI_DIO3, PIN_NOPULL, 1);
#endif
}
