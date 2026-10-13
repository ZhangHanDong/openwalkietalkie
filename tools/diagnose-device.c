/* SPDX-License-Identifier: MIT
 * macOS: cc -Wall -Wextra -Werror tools/diagnose-device.c -framework IOKit
 *          -framework CoreFoundation -o /tmp/owt-diagnose
 * /tmp/owt-diagnose [--battery] [--watch SECONDS] | --wake
 * Uses Bluetooth vendor HID only: never opens the reset-wired USB console.
 */
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/hid/IOHIDManager.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../native/src/diagnostics.h"

static bool received;
static uint8_t sequence, operation;
static void battery_reply(const uint8_t *r)
{
    const uint8_t *registers = r + OWT_DIAG_BATTERY_REGS;
    const uint16_t failed = owt_diag_get16(r + OWT_DIAG_BATTERY_ERRORS);
    static const char *charge_names[] = { "unknown", "battery", "usb", "charging", "full" };
    const unsigned charge = r[OWT_DIAG_BATTERY_CHARGE];
    printf("host_time=%.3f uptime_ms=%u battery_mv=%u soc=",
           CFAbsoluteTimeGetCurrent() + kCFAbsoluteTimeIntervalSince1970,
           owt_diag_get32(r + 4), owt_diag_get16(r + OWT_DIAG_BATTERY_MV));
    if (r[OWT_DIAG_BATTERY_SOC] == 255) printf("unknown");
    else printf("%u", r[OWT_DIAG_BATTERY_SOC]);
    printf(" charge=%u(%s) full_mv=%u charger=", charge,
           charge < sizeof(charge_names) / sizeof(charge_names[0]) ? charge_names[charge] : "invalid",
           owt_diag_get16(r + OWT_DIAG_BATTERY_FULL_MV));
    for (unsigned reg = 0; reg <= 0x0c; reg++) {
        printf("%s%02x:", reg ? "," : "", reg);
        if (failed & (1U << reg)) printf("??");
        else printf("%02x", registers[reg]);
    }
    printf(" read_errors=%04x completion_pending=%u candidate_age_ms=%u", failed,
           r[OWT_DIAG_BATTERY_FLAGS] & 1,
           owt_diag_get32(r + OWT_DIAG_BATTERY_CANDIDATE_AGE));
    /* These encodings apply only to the verified SGM41562S identity. */
    if (!(failed & (1U << 0x0b)) && registers[0x0b] == 0x09) {
        if (!(failed & (1U << 0x04)))
            printf(" regulation_mv=%u", 3500U + 10U * (registers[0x04] >> 1));
        if (!(failed & (1U << 0x02)))
            printf(" fast_charge_ma=%u", 8U + 8U * (registers[0x02] & 0x7f));
        if (!(failed & (1U << 0x03)))
            printf(" termination_code=%u", registers[0x03] & 0x0f);
        if (!(failed & (1U << 0x0c)))
            printf(" input_limit_ma=%u", 50U + 30U * (registers[0x0c] >> 3));
        if (!(failed & (1U << 0x08)))
            printf(" power_good=%u charger_state=%u", !!(registers[0x08] & 0x02),
                   (registers[0x08] >> 3) & 3);
    }
    putchar('\n');
}
static void input(void *context, IOReturn result, void *sender, IOHIDReportType type,
                  uint32_t report_id, uint8_t *bytes, CFIndex length)
{
    (void)context; (void)sender; (void)type; (void)report_id;
    if (result || length < 0) return;
    const uint8_t *r = owt_diag_body(bytes, (size_t)length);
    if (!r || r[1] != OWT_DIAG_VERSION || r[2] != (operation | 0x80) || r[3] != sequence) return;
    if (operation == OWT_DIAG_BATTERY) {
        battery_reply(r);
        fflush(stdout);
        received = true;
        return;
    }
    printf("host_time=%.3f uptime_ms=%u HCLK_MHz=%u screen=%u pins=%02x touch=",
           CFAbsoluteTimeGetCurrent() + kCFAbsoluteTimeIntervalSince1970,
           owt_diag_get32(r + 4), owt_diag_get32(r + 8) / 1000000, r[12], r[13]);
    for (unsigned i = 14; i < 21; i++) printf("%02x", r[i]);
    printf(" mode=%02x%02x%02x%02x keypad_cfg=%02x expander_out=%02x%02x dir=%02x%02x",
           r[21], r[22], r[23], r[24], r[25], r[27], r[26], r[29], r[28]);
    printf(" charger=01:%02x,06:%02x,07:%02x,08:%02x,09:%02x,0a:%02x,0c:%02x",
           r[30], r[31], r[32], r[33], r[34], r[35], r[36]);
    printf(" errors=%x touch_irqs=%u key_irqs=%u battery_mv=%u dma=%u audio_clock=%u",
           owt_diag_get32(r + 40), owt_diag_get32(r + 44), owt_diag_get32(r + 48),
           r[52] | (unsigned)r[53] << 8, r[54], r[55]);
    printf(" boot_charger=08:%02x,09:%02x,0a:%02x boot_src=%x boot_mode=%u deep_wfi=%u boot_read_errors=%u\n",
           r[37], r[38], r[39], owt_diag_get32(r + 56), r[60], r[61], r[62]);
    fflush(stdout);
    received = true;
}
static void number(CFMutableDictionaryRef dict, CFStringRef name, int value)
{
    CFNumberRef n = CFNumberCreate(NULL, kCFNumberIntType, &value);
    CFDictionarySetValue(dict, name, n); CFRelease(n);
}
static int sample(void)
{
    int status = 2;
    received = false;
    IOHIDManagerRef manager = IOHIDManagerCreate(NULL, kIOHIDOptionsTypeNone);
    CFMutableDictionaryRef match = CFDictionaryCreateMutable(NULL, 0,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    number(match, CFSTR(kIOHIDVendorIDKey), 0x1209);
    number(match, CFSTR(kIOHIDProductIDKey), 0x5754);
    number(match, CFSTR(kIOHIDPrimaryUsagePageKey), 0xff00);
    IOHIDManagerSetDeviceMatching(manager, match); CFRelease(match);
    IOHIDManagerOpen(manager, kIOHIDOptionsTypeNone);
    CFSetRef devices = IOHIDManagerCopyDevices(manager);
    CFIndex count = devices ? CFSetGetCount(devices) : 0;
    if (count != 1) { fprintf(stderr, "Expected one OpenWalkieTalkie vendor interface, found %ld\n", count); goto done; }
    IOHIDDeviceRef device;
    CFSetGetValues(devices, (const void **)&device);
    IOReturn error = IOHIDDeviceOpen(device, kIOHIDOptionsTypeNone);
    if (error) { fprintf(stderr, "HID open: 0x%x\n", error); status = 3; goto done; }
    uint8_t buffer[256];
    IOHIDDeviceRegisterInputReportCallback(device, buffer, sizeof(buffer), input, NULL);
    IOHIDDeviceScheduleWithRunLoop(device, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
    uint8_t request[64] = {6, OWT_DIAG_CHANNEL, OWT_DIAG_VERSION, operation, ++sequence};
    error = IOHIDDeviceSetReport(device, kIOHIDReportTypeOutput, 6, request, sizeof(request));
    CFAbsoluteTime end = CFAbsoluteTimeGetCurrent() + 4;
    while (!error && !received && CFAbsoluteTimeGetCurrent() < end)
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, .05, false);
    if (!received) fprintf(stderr, "No diagnostic reply, write result=0x%x\n", error);
    IOHIDDeviceUnscheduleFromRunLoop(device, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
    IOHIDDeviceClose(device, kIOHIDOptionsTypeNone);
    status = received ? 0 : 4;
done:
    if (devices) CFRelease(devices);
    IOHIDManagerClose(manager, kIOHIDOptionsTypeNone); CFRelease(manager);
    return status;
}
int main(int argc, char **argv)
{
    double watch = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--battery") && operation == OWT_DIAG_STATUS)
            operation = OWT_DIAG_BATTERY;
        else if (!strcmp(argv[i], "--wake") && argc == 2)
            operation = OWT_DIAG_WAKE;
        else if (!strcmp(argv[i], "--watch") && watch == 0 && i + 1 < argc) {
            char *end;
            watch = strtod(argv[++i], &end);
            if (*end || !(watch > 0 && watch <= 3600)) goto usage;
        } else goto usage;
    }
    CFAbsoluteTime deadline = CFAbsoluteTimeGetCurrent() + watch;
    int status;
    do {
        /* Reacquire HID each time so watch follows a reboot/reconnect instead
         * of repeatedly sending to a stale OS handle. No USB port is opened. */
        status = sample();
        if (status) {
            fprintf(stderr, "host_time=%.3f diagnostic_status=%d\n",
                    CFAbsoluteTimeGetCurrent() + kCFAbsoluteTimeIntervalSince1970, status);
            fflush(stderr);
        }
        if (watch > 0) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 2, false);
    } while (CFAbsoluteTimeGetCurrent() < deadline);
    return status;
usage:
    fprintf(stderr, "Usage: %s [--battery] [--watch SECONDS] | --wake\n", argv[0]);
    return 1;
}
