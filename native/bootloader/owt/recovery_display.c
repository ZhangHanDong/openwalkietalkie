/* SPDX-License-Identifier: MIT */
#include "bf0_hal.h"
#include "recovery_ui.h"

/* Only the CO5300 on Rev A/B. HAL initializes LCDC; bounded polled register
 * and DMA transfers below avoid the SDK's unbounded EOF wait and tick-based
 * command waits (this bare-metal bootloader does not run a SysTick ISR).
 * Same QSPI framing, RGB565 format and reset sequence as src/display.rs. */
static uint16_t band[OWT_SCREEN_SIZE * OWT_SCREEN_BAND]
    __attribute__((section(".recovery_scratch"), aligned(4)));
static bool enabled;
static int last_phase = -1, last_percent = -1, last_error = -1;
static bool last_known;

static bool idle(void)
{
    for (unsigned n = 0; n < 10000; n++) {
        if (!(hwp_lcdc1->STATUS & LCD_IF_STATUS_LCD_BUSY)
            && !(hwp_lcdc1->LCD_SINGLE & LCD_IF_LCD_SINGLE_LCD_BUSY)) return true;
        HAL_Delay_us(10);
    }
    return false;
}

static bool send(uint32_t word, unsigned bytes, bool data, bool end)
{
    if (!idle()) return false;
    uint32_t cfg = hwp_lcdc1->SPI_IF_CONF;
    cfg &= ~(LCD_IF_SPI_IF_CONF_SPI_RD_MODE_Msk | LCD_IF_SPI_IF_CONF_WR_LEN_Msk
             | LCD_IF_SPI_IF_CONF_SPI_CS_AUTO_DIS);
    cfg |= (bytes - 1) << LCD_IF_SPI_IF_CONF_WR_LEN_Pos;
    if (end) cfg |= LCD_IF_SPI_IF_CONF_SPI_CS_AUTO_DIS;
    hwp_lcdc1->SPI_IF_CONF = cfg;
    hwp_lcdc1->LCD_WR = word;
    hwp_lcdc1->LCD_SINGLE = LCD_IF_LCD_SINGLE_WR_TRIG | (data ? LCD_IF_LCD_SINGLE_TYPE : 0);
    return idle();
}

static bool command(uint8_t cmd, const uint8_t *data, unsigned size)
{
    if (!send((0x02u << 24) | ((uint32_t)cmd << 8), 4, false, !size)) return false;
    for (unsigned i = 0; i < size; i++)
        if (!send(data[i], 1, true, i + 1 == size)) return false;
    return true;
}

static bool write_band(unsigned y)
{
    unsigned bottom = y + OWT_SCREEN_BAND - 1;
    const uint8_t x_range[] = {0, 0, 1, 0xdf};
    const uint8_t y_range[] = {y >> 8, y, bottom >> 8, bottom};
    if (!command(0x2a, x_range, 4) || !command(0x2b, y_range, 4)
        || !send(0x32002c00, 4, false, false)) return false;
    hwp_lcdc1->CANVAS_TL_POS = y << LCD_IF_CANVAS_TL_POS_Y0_Pos;
    hwp_lcdc1->CANVAS_BR_POS = (479u << LCD_IF_CANVAS_BR_POS_X1_Pos)
        | (bottom << LCD_IF_CANVAS_BR_POS_Y1_Pos);
    hwp_lcdc1->LAYER0_TL_POS = y << LCD_IF_LAYER0_TL_POS_Y0_Pos;
    hwp_lcdc1->LAYER0_BR_POS = (479u << LCD_IF_LAYER0_BR_POS_X1_Pos)
        | (bottom << LCD_IF_LAYER0_BR_POS_Y1_Pos);
    hwp_lcdc1->LAYER0_CONFIG = LCD_IF_LAYER0_CONFIG_FORMAT_RGB565
        | LCD_IF_LAYER0_CONFIG_ALPHA_SEL | (255u << LCD_IF_LAYER0_CONFIG_ALPHA_Pos)
        | (960u << LCD_IF_LAYER0_CONFIG_WIDTH_Pos) | LCD_IF_LAYER0_CONFIG_PREFETCH_EN
        | LCD_IF_LAYER0_CONFIG_ACTIVE;
    hwp_lcdc1->LAYER1_CONFIG = 0;
    hwp_lcdc1->LAYER0_SRC = (uint32_t)(uintptr_t)band;
    hwp_lcdc1->SPI_IF_CONF |= LCD_IF_SPI_IF_CONF_SPI_CS_AUTO_DIS;
    hwp_lcdc1->IRQ = LCD_IF_IRQ_EOF_STAT | LCD_IF_IRQ_DPI_UDR_STAT | LCD_IF_IRQ_ICB_OF_STAT;
    __DSB(); /* SRAM writes must be visible to DMA; bootloader caches are off. */
    hwp_lcdc1->COMMAND = LCD_IF_COMMAND_START;
    for (unsigned n = 0; n < 10000; n++) {
        uint32_t irq = hwp_lcdc1->IRQ;
        if (irq & (LCD_IF_IRQ_DPI_UDR_RAW_STAT | LCD_IF_IRQ_ICB_OF_RAW_STAT)) return false;
        if (irq & LCD_IF_IRQ_EOF_RAW_STAT) return idle();
        HAL_Delay_us(10);
    }
    return false;
}

static void disable(void)
{
    enabled = false;
    /* Stop a stuck DMA before its buffer is reused. No application jump is
     * made from recovery; all exits use the existing PMU reset. */
    HAL_RCC_ResetModule(RCC_MOD_LCDC1);
    HAL_GPIO_WritePin(hwp_gpio1, 9, GPIO_PIN_RESET);
}

void owt_recovery_display_update(const struct owt_recovery_ui *ui)
{
    int percent = (int)owt_recovery_ui_percent(ui);
    bool known = ui->total != 0;
    if (!enabled || (last_phase == (int)ui->phase && last_percent == percent && known == last_known
                     && last_error == ui->error)) return;
    /* Only the percentage/bar band changes during a transfer. */
    unsigned first = last_phase == (int)ui->phase && last_error == ui->error ? 208 : 0;
    unsigned end = first ? 288 : OWT_SCREEN_SIZE;
    for (unsigned y = first; y < end; y += OWT_SCREEN_BAND) {
        owt_recovery_ui_render(ui, y, true, band);
        if (!write_band(OWT_SCREEN_SIZE - y - OWT_SCREEN_BAND)) {
            disable();
            return;
        }
    }
    last_phase = ui->phase; last_percent = percent; last_error = ui->error; last_known = known;
}

static void output(unsigned pin, GPIO_PinState level)
{
    GPIO_InitTypeDef cfg = {.Pin = pin, .Mode = GPIO_MODE_OUTPUT, .Pull = GPIO_NOPULL};
    HAL_GPIO_WritePin(hwp_gpio1, pin, level);
    HAL_GPIO_Init(hwp_gpio1, &cfg);
}

void owt_recovery_display_init(const struct owt_recovery_ui *ui)
{
    HAL_RCC_EnableModule(RCC_MOD_GPIO1);
    HAL_PIN_Set(PAD_PA09, GPIO_A9, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA10, GPIO_A10, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA11, GPIO_A11, PIN_NOPULL, 1);
    output(9, GPIO_PIN_RESET);
    output(11, GPIO_PIN_SET); /* logic first, then AMOLED supply */
    HAL_Delay_us(5000);
    output(10, GPIO_PIN_SET);
    HAL_PIN_Set(PAD_PA02, LCDC1_SPI_TE, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA03, LCDC1_SPI_CS, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA04, LCDC1_SPI_CLK, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA05, LCDC1_SPI_DIO0, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA06, LCDC1_SPI_DIO1, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA07, LCDC1_SPI_DIO2, PIN_NOPULL, 1);
    HAL_PIN_Set(PAD_PA08, LCDC1_SPI_DIO3, PIN_NOPULL, 1);
    LCDC_HandleTypeDef lcd = {0};
    lcd.Instance = hwp_lcdc1;
    lcd.Init = (LCDC_InitTypeDef){
        .lcd_itf = LCDC_INTF_SPI_DCX_4DATA, .freq = 48000000,
        .color_mode = LCDC_PIXEL_FORMAT_RGB565,
        .cfg.spi = {.syn_mode = HAL_LCDC_SYNC_DISABLE, .vsyn_polarity = 1},
    };
    if (HAL_LCDC_Init(&lcd) != HAL_OK) { disable(); return; }
    HAL_NVIC_DisableIRQ(LCDC1_IRQn); /* all transfers are bounded and polled */
    HAL_Delay_us(10000);
    HAL_GPIO_WritePin(hwp_gpio1, 9, GPIO_PIN_SET);
    HAL_Delay_us(10000);
    HAL_GPIO_WritePin(hwp_gpio1, 9, GPIO_PIN_RESET);
    HAL_Delay_us(10000);
    HAL_GPIO_WritePin(hwp_gpio1, 9, GPIO_PIN_SET);
    HAL_Delay_us(50000);
    static const uint8_t init[][2] = {
        {0xfe, 0x00}, {0xc4, 0x80}, {0x3a, 0x55}, {0x35, 0x00},
        {0x53, 0x20}, {0x51, 0x80}, {0x63, 0x80},
    };
    for (unsigned i = 0; i < sizeof(init) / sizeof(init[0]); i++)
        if (!command(init[i][0], &init[i][1], 1)) { disable(); return; }
    if (!command(0x11, NULL, 0)) { disable(); return; }
    HAL_Delay_us(120000);
    enabled = true;
    owt_recovery_display_update(ui); /* replace all GRAM before display-on */
    if (enabled && !command(0x29, NULL, 0)) disable();
}
