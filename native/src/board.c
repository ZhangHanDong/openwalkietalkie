/* SPDX-License-Identifier: MIT */
#include <rtthread.h>
#include <rtdevice.h>
#include "board.h"
#include "openwalkietalkie.h"
#include "diagnostics.h"
#include "bf0_pm.h"

/* Pins that differ between the boards. The full maps are in src/board.rs;
 * the C side is selected by OWT_BOARD_HW_A (Rev A and Rev B) and
 * OWT_BOARD_HW_B (Rev B's three moved pins), which tools/build-native.sh
 * sets from OWT_BOARD=hw-a / hw-b. */
#ifdef OWT_BOARD_HW_A
#ifdef OWT_BOARD_HW_B
#define OWT_CHG_PIN 33 /* BQ24073 CHG, open drain with R46 100k to IO_3V3: low while charging */
#endif
#define OWT_TOUCH_IRQ_PIN 43
#define OWT_TOUCH_RESET_PIN 44
#define OWT_PANEL_POWER_PIN 10  /* PANEL_ENABLE: U8 makes PANEL_4V */
#define OWT_PANEL_LOGIC_PIN 11  /* PANEL_VCI_ENABLE: U10 switches PANEL_LOGIC */
#define OWT_SYSTEM_EN_PIN OWT_PANEL_LOGIC_PIN /* stands in for the dev board's 3V3_EN in diagnostics */
#define OWT_DIAG_SECOND_PIN 39  /* encoder A stands in for the keypad IRQ slot */
#define OWT_SECOND_IRQ_NAME "input_irq"
/* PTT, ENTER, ESC, SHIFT, LEFT, RIGHT, encoder A, encoder B: the bit order of
 * src/input/direct.rs. Active low, 100k pull-ups on the board. */
static const uint8_t input_pins[8] = { 24, 25, 26, 27, 37, 38, 39, 40 };
static volatile bool inputs_pending;
static volatile uint32_t input_irqs, input_irqs_total;
#else
#define OWT_TOUCH_IRQ_PIN 1
#define OWT_TOUCH_RESET_PIN 0
#define OWT_KEYPAD_IRQ_PIN 11
#define OWT_SYSTEM_EN_PIN 41
#define OWT_PANEL_POWER_PIN 44
#define OWT_DIAG_SECOND_PIN OWT_KEYPAD_IRQ_PIN
#define OWT_SECOND_IRQ_NAME "keypad_irq"
#endif

#ifdef OWT_BOARD_HW_A
#define OWT_MIC_POWER_PIN 28 /* MIC_POWER_ENABLE: the load switch feeding the MP34DT05's 3.3 V */
void BSP_GPIO_Set(int pin, int value, int is_porta);
HAL_StatusTypeDef __real_HAL_PDM_Init(PDM_HandleTypeDef *hpdm);
HAL_StatusTypeDef __real_HAL_PDM_DeInit(PDM_HandleTypeDef *hpdm);
/* The microphone is powered only while the PDM block runs. The audio server
 * configures PDM1 (HAL_PDM_Init) when a voice link opens and tears it down
 * (HAL_PDM_DeInit) when the link closes; the supply must be up before the
 * clock starts, and 10 ms covers the load switch's slew and the part's own
 * start-up. The server also discards the first frames of every capture. */
HAL_StatusTypeDef __wrap_HAL_PDM_Init(PDM_HandleTypeDef *hpdm)
{
    BSP_GPIO_Set(OWT_MIC_POWER_PIN, 1, 1);
    rt_thread_mdelay(10);
    rt_kprintf("microphone: MP34DT05 powered (PA28), PDM1 on PA22/PA23\n");
    return __real_HAL_PDM_Init(hpdm);
}
HAL_StatusTypeDef __wrap_HAL_PDM_DeInit(PDM_HandleTypeDef *hpdm)
{
    HAL_StatusTypeDef result = __real_HAL_PDM_DeInit(hpdm);
    BSP_GPIO_Set(OWT_MIC_POWER_PIN, 0, 1);
    rt_kprintf("microphone: powered off\n");
    return result;
}
#endif

static struct rt_i2c_bus_device *i2c;
static rt_adc_device_t adc;
static LCDC_HandleTypeDef lcd;
static volatile bool display_active;
static volatile bool display_done, display_error;
static volatile uint32_t display_started, display_transfer_ms;
static uint32_t display_pixels;
static volatile bool touch_pending;
static volatile bool keypad_pending;
static uint32_t i2c_count, band_count, watchdog_count, watchdog_errors;
static volatile uint32_t touch_irqs, keypad_irqs;
static volatile uint32_t touch_irqs_total, keypad_irqs_total;
static uint32_t touch_reads, touch_errors, keypad_errors;
static uint16_t battery_mv;
static uint8_t last_touch_report[7];
static uint8_t boot_charger[3], boot_charger_errors;

static void touch_interrupt(void *arg)
{
    (void)arg;
    touch_pending = true;
    touch_irqs++;
    touch_irqs_total++;
    owt_wake();
}

void owt_keypad_retry(void)
{
    rt_base_t level = rt_hw_interrupt_disable();
    keypad_pending = true;
    rt_hw_interrupt_enable(level);
    owt_wake();
}

static void keypad_interrupt(void *arg)
{
    (void)arg;
    keypad_irqs++;
    keypad_irqs_total++;
    owt_keypad_retry();
}

bool owt_keypad_asserted(void)
{
    rt_base_t level = rt_hw_interrupt_disable();
    bool pending = keypad_pending;
    keypad_pending = false;
    rt_hw_interrupt_enable(level);
#ifdef OWT_BOARD_HW_A
    return pending;
#else
    return pending || rt_pin_read(OWT_KEYPAD_IRQ_PIN) == PIN_LOW;
#endif
}

#ifdef OWT_BOARD_HW_A
static void input_interrupt(void *arg)
{
    (void)arg;
    inputs_pending = true;
    input_irqs++;
    input_irqs_total++;
    owt_wake();
}

bool owt_inputs_asserted(void)
{
    rt_base_t level = rt_hw_interrupt_disable();
    bool pending = inputs_pending;
    inputs_pending = false;
    rt_hw_interrupt_enable(level);
    return pending;
}

#ifdef OWT_BOARD_HW_B
bool owt_charger_charging(void) { return rt_pin_read(OWT_CHG_PIN) == PIN_LOW; }
#endif

uint8_t owt_inputs_read(void)
{
    uint8_t raw = 0;
    for (unsigned i = 0; i < 8; i++)
        if (rt_pin_read(input_pins[i]) == PIN_HIGH) raw |= 1U << i;
    return raw;
}

/* The second "input" word of the status lines: the raw level mask here, the
 * keypad interrupt level on the dev board. */
static unsigned input_word(void) { return owt_inputs_read(); }
#else
bool owt_inputs_asserted(void) { return false; }
uint8_t owt_inputs_read(void) { return 0xff; }
static unsigned input_word(void) { return rt_pin_read(OWT_KEYPAD_IRQ_PIN); }
#endif

void owt_watchdog_result(bool ok)
{
    if (ok) watchdog_count++;
    else { watchdog_errors++; rt_kprintf("charger: watchdog feed failed\n"); }
}

void owt_power_stats(uint32_t now, uint8_t screen)
{
    static uint32_t last;
    if (now - last < 30000) return;
    uint32_t elapsed = now - last;
    last = now;
    rt_kprintf("power: window=%ums screen=%u i2c=%u bands=%u watchdog=%u errors=%u vbat_monitor=%u\n",
               elapsed, screen, i2c_count, band_count, watchdog_count, watchdog_errors,
               !!(hwp_hpsys_cfg->ANAU_CR & HPSYS_CFG_ANAU_CR_EN_VBAT_MON));
    rt_base_t level = rt_hw_interrupt_disable();
    uint32_t pixels = display_pixels, dma_ms = display_transfer_ms;
#ifdef OWT_BOARD_HW_A
    uint32_t touch_edges = touch_irqs, key_edges = input_irqs;
    touch_irqs = input_irqs = 0;
#else
    uint32_t touch_edges = touch_irqs, key_edges = keypad_irqs;
    touch_irqs = keypad_irqs = 0;
#endif
    display_pixels = display_transfer_ms = 0;
    rt_hw_interrupt_enable(level);
    rt_kprintf("display: pixels=%u dma_ms=%u\n", pixels, dma_ms);
    rt_kprintf("input: touch_irq=%u " OWT_SECOND_IRQ_NAME "=%u touch_reads=%u touch_errors=%u keypad_errors=%u pins=%u/%02x battery_mv=%u uptime_ms=%u\n",
               touch_edges, key_edges, touch_reads, touch_errors, keypad_errors,
               rt_pin_read(OWT_TOUCH_IRQ_PIN), input_word(), battery_mv, now);
    touch_reads = touch_errors = keypad_errors = 0;
    i2c_count = band_count = watchdog_count = watchdog_errors = 0;
    owt_idle_power_stats();
}

void owt_delay_ms(uint32_t ms) { rt_thread_mdelay(ms); }
bool owt_touch_asserted(void)
{
    rt_base_t level = rt_hw_interrupt_disable();
    bool pending = touch_pending;
    touch_pending = false;
    rt_hw_interrupt_enable(level);
    return pending || rt_pin_read(OWT_TOUCH_IRQ_PIN) == PIN_LOW;
}

void owt_touch_trace(uint8_t phase, uint16_t x, uint16_t y, bool ready)
{
    static const char *names[] = {"down", "move", "up", "cancel"};
    if (phase < 4 && phase != 1)
        rt_kprintf("touch: %s x=%u y=%u hid=%u\n", names[phase], x, y, ready);
}

void owt_input_wake_trace(uint8_t source, uint32_t now)
{
    static const char *names[] = { "touch", "keypad", "debug", "notice" };
    if (source < 4) rt_kprintf("input: wake source=%s at %ums\n", names[source], now);
}

int owt_i2c_transfer(uint8_t address, const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_len)
{
    i2c_count++;
    if (!i2c || tx_len > UINT16_MAX || rx_len > UINT16_MAX) return -1;
    struct rt_i2c_msg messages[2];
    unsigned count = 0;
    if (tx_len) messages[count++] = (struct rt_i2c_msg) {
        .addr = address, .flags = RT_I2C_WR, .len = tx_len, .buf = (uint8_t *)tx
    };
    if (rx_len) messages[count++] = (struct rt_i2c_msg) {
        .addr = address, .flags = RT_I2C_RD, .len = rx_len, .buf = rx
    };
    int result = count && rt_i2c_transfer(i2c, messages, count) == count ? 0 : -1;
    if (address == 0x5a) {
        touch_reads += rx_len != 0;
        touch_errors += result != 0;
        if (result == 0 && tx_len == 2 && tx[0] == 0xd0 && tx[1] == 0 && rx_len == 15)
            rt_memcpy(last_touch_report, rx, sizeof(last_touch_report));
    }
    if (address == 0x34) keypad_errors += result != 0;
    /* Bounded startup diagnostics include idle frames: successful identity
     * reads alone do not prove that the normal touch report path works. */
    static unsigned touch_samples;
    if (address == 0x5a && tx_len == 2 && tx[0] == 0xd0 && tx[1] == 0 && rx_len == 15
        && touch_samples < 4) {
        touch_samples++;
        if (result == 0)
            rt_kprintf("touch: report %02x %02x %02x %02x %02x %02x %02x irq=%u\n",
                       rx[0], rx[1], rx[2], rx[3], rx[4], rx[5], rx[6], rt_pin_read(OWT_TOUCH_IRQ_PIN));
        else rt_kprintf("touch: report I2C error\n");
    }
    return result;
}

uint16_t owt_battery_mv(void)
{
    /* The SDK returns calibrated VBAT in 0.1mV units, including its divider. */
    if (!adc) return 0;
    /* The ADC driver blocks AON sleep, but does not guard idle clock scaling
     * during its delayed sampling sequence. Cover the complete conversion. */
    rt_pm_hw_device_start();
    /* rt_adc_read discards convert's error code and can return an uninitialized
     * value after a timeout. Never allow that to qualify charge completion. */
    rt_uint32_t value = 0;
    rt_err_t result = adc->ops->convert(adc, 7, &value);
    rt_pm_hw_device_stop();
    if (result != RT_EOK) {
        rt_kprintf("battery: ADC conversion failed (%d)\n", result);
        battery_mv = 0;
    } else {
        battery_mv = value >= 25000 && value <= 45000 ? (uint16_t)(value / 10) : 0;
    }
    return battery_mv;
}

void owt_battery_trace(uint16_t mv, uint8_t voltage_percent, uint8_t estimated_percent, uint8_t charge)
{
    rt_kprintf("battery: uptime_ms=%u mv=%u voltage_pct=%u soc_pct=%u charge=%u\n",
               rt_tick_get_millisecond(), mv, voltage_percent, estimated_percent, charge);
}

static void command(uint8_t cmd, const uint8_t *bytes, unsigned len)
{
    RT_ASSERT(!owt_display_busy());
    rt_pm_hw_device_start();
    HAL_StatusTypeDef result = HAL_LCDC_WriteU32Reg(&lcd, (0x02U << 24) | (cmd << 8),
                                                  (uint8_t *)bytes, len);
    rt_pm_hw_device_stop();
    RT_ASSERT(result == HAL_OK);
}

static void region(unsigned x0, unsigned y0, unsigned x1, unsigned y1)
{
    const uint8_t x[] = { x0 >> 8, x0, x1 >> 8, x1 };
    const uint8_t y[] = { y0 >> 8, y0, y1 >> 8, y1 };
    command(0x2a, x, sizeof(x));
    command(0x2b, y, sizeof(y));
    HAL_LCDC_SetROIArea(&lcd, x0, y0, x1, y1);
}

static void panel_initialize(void)
{
    /* Cold initialization is also used after panel-only reset on wake.
     * Restore the interface and brightness registers instead of relying on
     * their retention through a long panel sleep. Touch PA0 is untouched. */
    static const uint8_t init[][2] = {
        {0xfe, 0x00}, {0xc4, 0x80}, {0x3a, 0x55}, {0x35, 0x00},
        {0x53, 0x20}, {0x51, 0xff}, {0x63, 0xff},
    };
    RT_ASSERT(HAL_LCDC_SetFreq(&lcd, 48000000) == HAL_OK);
    for (unsigned i = 0; i < sizeof(init) / sizeof(init[0]); i++)
        command(init[i][0], &init[i][1], 1);
    region(0, 0, 479, 479);
    command(0x11, NULL, 0);
}

void owt_board_diagnose(void)
{
    /* Application thread only, with no in-flight frame. RDDPM (0Ah) is
     * readable in Sleep In too: the command must not itself wake the UI.
     * Use LilyGO's 2MHz read protocol and restore the normal QSPI divider. */
    RT_ASSERT(!owt_display_busy());
    uint32_t id = 0;
    uint8_t power = 0, brightness = 0;
    rt_pm_hw_device_start();
    HAL_LCDC_SetFreq(&lcd, 2000000);
    HAL_StatusTypeDef id_result = HAL_LCDC_ReadU32Reg(&lcd, 0x03000400, (uint8_t *)&id, 3);
    HAL_StatusTypeDef power_result = HAL_LCDC_ReadU32Reg(&lcd, 0x03000a00, &power, 1);
    HAL_StatusTypeDef brightness_result = HAL_LCDC_ReadU32Reg(&lcd, 0x03005200, &brightness, 1);
    HAL_LCDC_SetFreq(&lcd, 48000000);
    rt_pm_hw_device_stop();
    rt_kprintf("diagnostic: panel id=%06x power=%02x brightness=%u read_status=%u/%u/%u reset=%u rail=%u\n",
               id, power, brightness, id_result, power_result, brightness_result,
               rt_pin_read(9), rt_pin_read(OWT_PANEL_POWER_PIN));
    uint8_t reg = 1, keypad_config = 0;
    uint8_t outputs[2] = {0};
    static const uint8_t charger_regs[] = { 0x01, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0c };
    uint8_t charger[sizeof(charger_regs)] = {0};
    unsigned failed = 0;
#ifdef OWT_BOARD_HW_A
    /* Rev A has no keypad, no expander and no I2C charger: nothing to read,
     * and a probe of an absent address costs a bus reset and error lines. */
    int keypad_result = -1, expander_result = -1;
    failed = (1U << sizeof(charger_regs)) - 1;
#else
    int keypad_result = owt_i2c_transfer(0x34, &reg, 1, &keypad_config, 1);
    reg = 2;
    int expander_result = owt_i2c_transfer(0x22, &reg, 1, outputs, sizeof(outputs));
    /* Read individually: do not assume charger register auto-increment. */
    for (unsigned i = 0; i < sizeof(charger_regs); i++) {
        reg = charger_regs[i];
        if (owt_i2c_transfer(0x03, &reg, 1, &charger[i], 1)) failed |= 1U << i;
    }
#endif
    rt_kprintf("diagnostic: keypad_cfg=%02x read_status=%d expander=%02x/%02x read_status=%d input_irq=%u/%02x touch=%02x %02x %02x %02x %02x %02x %02x\n",
               keypad_config, keypad_result, outputs[0], outputs[1], expander_result,
               rt_pin_read(OWT_TOUCH_IRQ_PIN), input_word(), last_touch_report[0], last_touch_report[1],
               last_touch_report[2], last_touch_report[3], last_touch_report[4],
               last_touch_report[5], last_touch_report[6]);
    rt_kprintf("diagnostic: charger 01=%02x 06=%02x 07=%02x 08=%02x 09=%02x 0a=%02x 0c=%02x failed=%02x system_en=%u touch_reset=%u battery_mv=%u\n",
               charger[0], charger[1], charger[2], charger[3], charger[4], charger[5],
               charger[6], failed, rt_pin_read(OWT_SYSTEM_EN_PIN), rt_pin_read(OWT_TOUCH_RESET_PIN), battery_mv);
#ifdef OWT_BOARD_HW_B
    rt_kprintf("diagnostic: charger CHG pin PA%u=%u (low = charging)\n", OWT_CHG_PIN, rt_pin_read(OWT_CHG_PIN));
#endif
}

bool owt_diagnostic_report(const uint8_t *data, size_t len)
{
    const uint8_t *body = owt_diag_body(data, len);
    if (!body) return false;
    if (body[1] != OWT_DIAG_VERSION || body[2] > OWT_DIAG_BATTERY) return true;
    /* Called only by the application thread on an encrypted HID connection.
     * Status never wakes or resets anything and works without USB attached. */
    if (body[2] == OWT_DIAG_WAKE) owt_core_wake(rt_tick_get_millisecond());
    uint8_t reply[63] = { OWT_DIAG_CHANNEL, OWT_DIAG_VERSION, body[2] | 0x80, body[3] };
    owt_diag_put32(reply + 4, rt_tick_get_millisecond());
    if (body[2] == OWT_DIAG_BATTERY) {
        /* Individual register reads; no charger writes and no new ADC
         * conversion. REG09 reads can clear latched fault indications. */
        uint16_t failed = 0;
#ifdef OWT_BOARD_HW_A
        failed = 0x1fff; /* no I2C charger on Rev A: every register "fails" */
#else
        for (uint8_t reg = 0; reg <= 0x0c; reg++) {
            if (owt_i2c_transfer(0x03, &reg, 1,
                                 reply + OWT_DIAG_BATTERY_REGS + reg, 1))
                failed |= 1U << reg;
        }
#endif
        owt_diag_put16(reply + OWT_DIAG_BATTERY_ERRORS, failed);
        owt_diag_put16(reply + OWT_DIAG_BATTERY_MV, battery_mv);
        owt_core_battery_diagnostic(reply + OWT_DIAG_BATTERY_CORE);
        owt_hid_notify(reply, sizeof(reply));
        return true;
    }
    owt_diag_put32(reply + 8, HAL_RCC_GetHCLKFreq(CORE_ID_HCPU));
    reply[12] = owt_core_screen();
    /* Protocol slots: touch IRQ, keypad IRQ (encoder A on Rev A), touch reset,
     * panel reset, system enable (PANEL_LOGIC on Rev A), panel supply. */
    static const uint8_t pins[] = {OWT_TOUCH_IRQ_PIN, OWT_DIAG_SECOND_PIN, OWT_TOUCH_RESET_PIN, 9,
                                   OWT_SYSTEM_EN_PIN, OWT_PANEL_POWER_PIN};
    for (unsigned i = 0; i < sizeof(pins); i++) reply[13] |= !!rt_pin_read(pins[i]) << i;
    rt_memcpy(reply + 14, last_touch_report, 7);
    uint8_t mode[] = {0, 2};
    uint32_t failed = owt_i2c_transfer(0x5a, mode, 2, reply + 21, 4) != 0;
    static const uint8_t reads[][4] = {
        /* address, register, reply offset, length */
        {0x34, 1, 25, 1}, {0x22, 2, 26, 2}, {0x22, 6, 28, 2},
        {0x03, 1, 30, 1}, {0x03, 6, 31, 1}, {0x03, 7, 32, 1},
        {0x03, 8, 33, 1}, {0x03, 9, 34, 1}, {0x03, 0xa, 35, 1}, {0x03, 0xc, 36, 1},
    };
    for (unsigned i = 0; i < sizeof(reads) / sizeof(reads[0]); i++) {
        const uint8_t *r = reads[i];
#ifdef OWT_BOARD_HW_A
        /* Only the touch controller exists on Rev A's bus. */
        if (r[0] != 0x5a) { failed |= 1U << (i + 1); continue; }
#endif
        if (owt_i2c_transfer(r[0], r + 1, 1, reply + r[2], r[3])) failed |= 1U << (i + 1);
    }
    owt_diag_put32(reply + 40, failed);
    rt_base_t level = rt_hw_interrupt_disable();
    owt_diag_put32(reply + 44, touch_irqs_total);
#ifdef OWT_BOARD_HW_A
    owt_diag_put32(reply + 48, input_irqs_total);
#else
    owt_diag_put32(reply + 48, keypad_irqs_total);
#endif
    rt_hw_interrupt_enable(level);
    reply[52] = battery_mv;
    reply[53] = battery_mv >> 8;
    reply[54] = display_active;
    reply[55] = !!(hwp_hpsys_rcc->DBGR & HPSYS_RCC_DBGR_FORCE_HP);
    rt_memcpy(reply + 37, boot_charger, 3);
    owt_diag_put32(reply + 56, pm_get_pwron_wakeup_src());
    reply[60] = SystemPowerOnModeGet();
#ifdef BSP_PM_FREQ_SCALING
    reply[61] = 1;
#endif
    reply[62] = boot_charger_errors;
    owt_hid_notify(reply, sizeof(reply));
    return true;
}

static void display_complete(LCDC_HandleTypeDef *handle)
{
    (void)handle;
    if (!display_active) return;
    display_transfer_ms += rt_tick_get_millisecond() - display_started;
    display_done = true;
    owt_wake();
}

static void display_failed(LCDC_HandleTypeDef *handle)
{
    (void)handle;
    if (!display_active) return;
    display_error = true;
    owt_wake();
}

void LCDC1_IRQHandler(void)
{
    rt_interrupt_enter();
    HAL_LCDC_IRQHandler(&lcd);
    rt_interrupt_leave();
}

bool owt_display_busy(void)
{
    if (!display_active) return false;
    if (display_error || (!display_done && rt_tick_get_millisecond() - display_started > 1000)) {
        rt_kprintf("display: DMA %s; restarting\n", display_error ? "error" : "timeout");
        HAL_NVIC_SystemReset();
        while (1) {}
    }
    if (!display_done) return true;
    display_active = false;
    /* Keep the PM clock guard until the application observes DMA completion. */
    rt_pm_hw_device_stop();
    return false;
}

void owt_board_start(void)
{
#ifdef OWT_BOARD_HW_A
    /* TOUCH_IRQ has its own 100k pull-up (R41); the direct inputs have theirs. */
    rt_pin_mode(OWT_TOUCH_IRQ_PIN, PIN_MODE_INPUT);
#else
    rt_pin_mode(OWT_TOUCH_IRQ_PIN, PIN_MODE_INPUT_PULLUP);
    rt_pin_mode(OWT_KEYPAD_IRQ_PIN, PIN_MODE_INPUT_PULLUP);
#endif
    rt_pin_mode(OWT_TOUCH_RESET_PIN, PIN_MODE_OUTPUT);
    rt_pin_write(OWT_TOUCH_RESET_PIN, PIN_LOW);
    rt_thread_mdelay(3);
    rt_pin_write(OWT_TOUCH_RESET_PIN, PIN_HIGH);
    rt_thread_mdelay(50);
    rt_err_t touch_irq = rt_pin_attach_irq(OWT_TOUCH_IRQ_PIN, PIN_IRQ_MODE_FALLING, touch_interrupt, RT_NULL);
    if (touch_irq == RT_EOK) touch_irq = rt_pin_irq_enable(OWT_TOUCH_IRQ_PIN, PIN_IRQ_ENABLE);
#ifdef OWT_BOARD_HW_A
    unsigned input_irqs_ok = 0;
    for (unsigned i = 0; i < 8; i++) {
        rt_pin_mode(input_pins[i], PIN_MODE_INPUT);
        rt_err_t irq = rt_pin_attach_irq(input_pins[i], PIN_IRQ_MODE_RISING_FALLING, input_interrupt, RT_NULL);
        if (irq == RT_EOK) irq = rt_pin_irq_enable(input_pins[i], PIN_IRQ_ENABLE);
        input_irqs_ok += irq == RT_EOK;
    }
    rt_kprintf("input: touch IRQ=%u (PA%u), %u/8 direct input IRQs (PTT PA24 ENTER PA25 ESC PA26 SHIFT PA27 LEFT PA37 RIGHT PA38 wheel PA39/PA40), levels=%02x, idle fallback=50ms\n",
               touch_irq == RT_EOK, OWT_TOUCH_IRQ_PIN, input_irqs_ok, owt_inputs_read());
#ifdef OWT_BOARD_HW_B
    rt_pin_mode(OWT_CHG_PIN, PIN_MODE_INPUT);
    rt_kprintf("charger: BQ24073 CHG on PA%u reads %s\n", OWT_CHG_PIN,
               owt_charger_charging() ? "charging" : "not charging");
#endif
#else
    rt_err_t keypad_irq = rt_pin_attach_irq(OWT_KEYPAD_IRQ_PIN, PIN_IRQ_MODE_FALLING, keypad_interrupt, RT_NULL);
    if (keypad_irq == RT_EOK) keypad_irq = rt_pin_irq_enable(OWT_KEYPAD_IRQ_PIN, PIN_IRQ_ENABLE);
    rt_kprintf("input: touch IRQ=%u keypad IRQ=%u, idle fallback=50ms\n",
               touch_irq == RT_EOK, keypad_irq == RT_EOK);
#endif
    i2c = rt_i2c_bus_device_find("i2c1");
    if (i2c) {
        struct rt_i2c_configuration config = {.mode = 0, .addr = 0, .timeout = 100, .max_hz = 400000};
        RT_ASSERT(rt_i2c_configure(i2c, &config) == RT_EOK);
    }
#ifndef OWT_BOARD_HW_A
    /* Preserve faults before Rust charger initialization or status reads can
     * clear them. These are observations, never watchdog/charge settings.
     * Rev A's BQ24073 has no bus: nothing to preserve, and nothing at 0x03. */
    static const uint8_t regs[] = {0x08, 0x09, 0x0a};
    for (unsigned n = 0; n < sizeof(regs); n++) {
        if (owt_i2c_transfer(0x03, &regs[n], 1, &boot_charger[n], 1))
            boot_charger_errors |= 1U << n;
    }
#endif
    adc = (rt_adc_device_t)rt_device_find("bat1");
    if (adc && rt_adc_enable(adc, 7) != RT_EOK) adc = NULL;

    lcd.Instance = hwp_lcdc1;
    lcd.Init = (LCDC_InitTypeDef) {
        .lcd_itf = LCDC_INTF_SPI_DCX_4DATA, .freq = 48000000,
        .color_mode = LCDC_PIXEL_FORMAT_RGB565,
        .cfg.spi = { .dummy_clock = 0, .syn_mode = HAL_LCDC_SYNC_DISABLE,
                     .vsyn_polarity = 1, .readback_from_Dx = 0 },
    };
    RT_ASSERT(HAL_LCDC_Init(&lcd) == HAL_OK);
    lcd.XferCpltCallback = display_complete;
    lcd.XferErrorCallback = display_failed;
    HAL_NVIC_SetPriority(LCDC1_IRQn, 6, 0);
    HAL_NVIC_EnableIRQ(LCDC1_IRQn);
    /* HAL_LCDC_Init initializes the controller, not the layer defaults.
     * A zero-initialized layer has zero row stride and alpha, so each band
     * sends a repeated transparent first row instead of the rendered UI. */
    RT_ASSERT(HAL_LCDC_LayerReset(&lcd, HAL_LCDC_LAYER_DEFAULT) == HAL_OK);
    RT_ASSERT(HAL_LCDC_LayerEnableAlpha(&lcd, HAL_LCDC_LAYER_DEFAULT, 255) == HAL_OK);
    rt_pin_mode(9, PIN_MODE_OUTPUT);
    rt_pin_write(9, PIN_LOW);
    rt_thread_mdelay(10);
    rt_pin_write(9, PIN_HIGH);
    rt_thread_mdelay(10);
    rt_pin_write(9, PIN_LOW);
    rt_thread_mdelay(10);
    rt_pin_write(9, PIN_HIGH);
    rt_thread_mdelay(50);

    panel_initialize();
    rt_thread_mdelay(120);
    /* Keep the panel off until the complete splash replaces undefined GRAM. */
    HAL_LCDC_LayerSetFormat(&lcd, HAL_LCDC_LAYER_DEFAULT, LCDC_PIXEL_FORMAT_RGB565);
    rt_kprintf("board: CO5300 ready, I2C=%u ADC=%u; QSPI=%uHz\n", !!i2c, !!adc,
               HAL_RCC_GetHCLKFreq(CORE_ID_HCPU) /
               ((lcd.Instance->SPI_IF_CONF & LCD_IF_SPI_IF_CONF_CLK_DIV_Msk) >> LCD_IF_SPI_IF_CONF_CLK_DIV_Pos));
}

void owt_display_on(void)
{
    owt_display_enable();
    rt_thread_mdelay(70);
}

void owt_display_enable(void) { command(0x29, NULL, 0); }
void owt_display_brightness(uint8_t level) { command(0x51, &level, 1); }
/* The application keeps servicing input/audio through the 120ms sleep/wake
 * settling intervals, and refreshes GRAM before making the panel visible. */
void owt_display_sleep(void)
{
    command(0x28, NULL, 0);
    command(0x10, NULL, 0);
}
void owt_display_reset(bool asserted)
{
    RT_ASSERT(!owt_display_busy());
    rt_pin_write(9, asserted ? PIN_LOW : PIN_HIGH);
}
void owt_display_wake(void) { panel_initialize(); }

void owt_display_band(uint16_t index, const uint8_t *pixels)
{
    /* This direct HAL path bypasses drv_lcd's PM guard. Keep clocks stable
     * through the entire synchronous transfer, even if our thread blocks. */
    rt_pm_hw_device_start();
    band_count++;
    unsigned y0 = index * 48, y1 = y0 + 47;
    region(0, y0, 479, y1);
    HAL_LCDC_LayerSetDataExt(&lcd, HAL_LCDC_LAYER_DEFAULT, (uint8_t *)pixels,
                           0, y0, 479, y1, 480);
    HAL_StatusTypeDef result = HAL_LCDC_SendLayerData2Reg(&lcd, (0x32U << 24) | (0x2c << 8), 4);
    rt_pm_hw_device_stop();
    RT_ASSERT(result == HAL_OK);
}

void owt_display_region(const struct owt_region *rect, const uint8_t *pixels)
{
    RT_ASSERT(!owt_display_busy());
    RT_ASSERT(pixels && ((uintptr_t)pixels & 3) == 0);
    RT_ASSERT(rect->x0 < rect->x1 && rect->x1 <= 480 && rect->y0 < rect->y1 && rect->y1 <= 480);
    RT_ASSERT(((rect->x0 | rect->y0 | rect->x1 | rect->y1) & 1) == 0);
    rt_pm_hw_device_start();
    region(rect->x0, rect->y0, rect->x1 - 1, rect->y1 - 1);
    HAL_LCDC_LayerSetDataExt(&lcd, HAL_LCDC_LAYER_DEFAULT, (uint8_t *)pixels,
                           rect->x0, rect->y0, rect->x1 - 1, rect->y1 - 1, 480);
    /* Rust has rotated both the pixels and destination. Keep the original
     * source pointer/stride; no extra LCDC layer mirroring is needed. */
    display_done = display_error = false;
    display_started = rt_tick_get_millisecond();
    display_active = true;
    band_count++;
    display_pixels += (rect->x1 - rect->x0) * (rect->y1 - rect->y0);
    /* The caller retains this buffer until completion, while drawing into the other one. */
    RT_ASSERT(HAL_LCDC_SendLayerData2Reg_IT(&lcd, (0x32U << 24) | (0x2c << 8), 4) == HAL_OK);
}
