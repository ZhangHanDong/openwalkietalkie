/* SPDX-License-Identifier: MIT */
/* Compile the production report handler with simulated flash/USB boundaries.
 * tools/tests/test_recovery_ui.py extracts the named functions unchanged. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "recovery_ui.h"

static unsigned erases, writes, replies;
static uint8_t flash_status, last_op, response[64];
static uint32_t response_size, ticks_ms = 20, reboot_at_ms;
static bool valid = true;
static int talking;
static int app_valid(void) { return valid; }
static void say(const char *s) { (void)s; }
static void reply(uint8_t op, const uint8_t *p, uint32_t n)
{
    assert(n <= sizeof(response));
    memcpy(response, p, n); last_op = op; response_size = n; replies++;
}
static int g_flash_erase(uint32_t addr, uint32_t n)
{
    assert(addr == OWT_APP_BASE && n == OWT_RECOVERY_SECTOR);
    erases++; valid = false; return flash_status ? -1 : 0;
}
static uint8_t program_sector(uint32_t addr, const uint8_t *data, uint32_t len);
#include "recovery_handler.inc"

static uint8_t program_sector(uint32_t addr, const uint8_t *data, uint32_t len)
{
    assert(owt_app_range(addr, OWT_RECOVERY_SECTOR));
    assert(len && len <= OWT_RECOVERY_SECTOR && data);
    writes++;
    if (addr == OWT_APP_BASE && !flash_status) valid = true;
    return flash_status;
}

static void report(uint8_t op, const uint8_t *payload, unsigned n)
{
    uint8_t rep[64] = {REPORT_ID, CHANNEL, n + 1, op};
    assert(n <= 60);
    if (n) memcpy(rep + 4, payload, n);
    handle_report(rep, n + 4);
}
static void begin(uint32_t size)
{
    uint8_t p[4]; wr32(p, size); report(OP_BEGIN, p, 4);
    assert(last_op == OP_BEGIN && response_size == 1);
}
static void commit(uint32_t addr, unsigned size, bool good_crc)
{
    uint8_t p[10]; wr32(p, addr); p[4] = size; p[5] = size >> 8;
    wr32(p + 6, crc32_of(sector_buf, size) ^ !good_crc);
    report(OP_COMMIT, p, sizeof(p));
    assert(last_op == OP_COMMIT && response_size == 5 && rd32(response + 1) == addr);
}
static void fill(unsigned size)
{
    for (unsigned off = 0; off < size;) {
        uint8_t p[60]; unsigned n = size - off; if (n > 57) n = 57;
        p[0] = off; p[1] = off >> 8; p[2] = n;
        memset(p + 3, 0xa5, n);
        report(OP_DATA, p, n + 3); off += n;
    }
}

static uint8_t bulk_packet[BULK_FRAME + 32], bulk_reply[24];
static uint32_t make_bulk(uint8_t op, uint32_t addr, uint32_t seq, uint32_t token, unsigned count)
{
    memset(bulk_packet, 0, sizeof(bulk_packet));
    memcpy(bulk_packet, "OWTF", 4);
    bulk_packet[4] = op; bulk_packet[5] = 1;
    bulk_packet[6] = count; bulk_packet[7] = count >> 8;
    wr32(bulk_packet + 8, addr); wr32(bulk_packet + 12, seq); wr32(bulk_packet + 16, token);
    memset(bulk_packet + 32, op == OP_ERASE ? 0xff : 0xa5, SECTOR);
    wr32(bulk_packet + 20, crc32_of(bulk_packet + 32, count));
    wr32(bulk_packet + 24, crc32_of(bulk_packet, 24));
    return 32 + (op == OP_COMMIT ? ((count + 63u) & ~63u) : 0);
}
static void bulk(uint8_t op, uint32_t addr, uint32_t seq, uint32_t token, unsigned count)
{
    uint32_t n = make_bulk(op, addr, seq, token, count);
    handle_bulk(bulk_packet, n, bulk_reply);
    assert(!memcmp(bulk_reply, "OWTA", 4) && bulk_reply[4] == op);
    assert(rd32(bulk_reply + 8) == seq && rd32(bulk_reply + 12) == token);
    assert(rd32(bulk_reply + 16) == addr);
    assert(rd32(bulk_reply + 20) == crc32_of(bulk_reply, 20));
}

static void test_bulk(void)
{
    for (unsigned tail = 1; tail <= 4096; tail++) {
        bulk(OP_BEGIN, 4096 + tail, 0, 7, 0); assert(!bulk_reply[5] && !valid);
        bulk(OP_COMMIT, APP_BASE, 1, 7, 4096); assert(!bulk_reply[5] && sector0_held && !valid);
        /* Empty tail requests verify erasure; they still count toward coverage. */
        bulk(tail & 1 ? OP_ERASE : OP_COMMIT, APP_BASE + 4096, 2, 7, tail);
        assert(!bulk_reply[5] && !valid && recovery_ui.received == 4096 + tail);
        bulk(OP_FINISH, 0, 3, 7, 0); assert(!bulk_reply[5] && valid);
    }
    unsigned before = erases;
    uint32_t n = make_bulk(OP_BEGIN, 4200, 0, 8, 0);
    bulk_packet[24] ^= 1;
    handle_bulk(bulk_packet, n, bulk_reply);
    assert(bulk_reply[5] == ST_ARGS && before == erases);
    bulk(OP_BEGIN, OWT_BONDS_START - APP_BASE + 1, 0, 8, 0);
    assert(bulk_reply[5] == ST_ARGS && before == erases);
    bulk(OP_BEGIN, 4200, 0, 0, 0); assert(bulk_reply[5] == ST_ARGS && before == erases);
    bulk(OP_BEGIN, 4200, 0, 8, 0);
    bulk(OP_COMMIT, APP_BASE, 1, 8, 4096);
    bulk(OP_FINISH, 0, 2, 8, 0); assert(bulk_reply[5] == ST_ARGS && !valid);
    for (unsigned fault = 0; fault < 7; fault++) {
        bulk(OP_BEGIN, 4200, 0, 9, 0);
        n = make_bulk(OP_COMMIT, APP_BASE, 1, 9, 4096);
        if (fault == 0) bulk_packet[32] ^= 1; /* payload CRC */
        if (fault == 1) { wr32(bulk_packet + 8, OWT_BONDS_START); }
        if (fault == 2) { wr32(bulk_packet + 12, 2); }
        if (fault == 3) { wr32(bulk_packet + 16, 8); }
        if (fault == 4) n--; /* truncated/short packet */
        if (fault == 5) { bulk_packet[6] = 0xff; bulk_packet[7] = 0xff; }
        if (fault == 6) flash_status = ST_VERIFY;
        wr32(bulk_packet + 24, crc32_of(bulk_packet, 24));
        if (fault == 6) { /* fail on a physical sector, after vectors are held */
            handle_bulk(bulk_packet, n, bulk_reply);
            n = make_bulk(OP_COMMIT, APP_BASE + 4096, 2, 9, 104);
        }
        handle_bulk(bulk_packet, n, bulk_reply);
        assert(bulk_reply[5] && !session && !valid && !sector0_held);
        bulk(OP_FINISH, 0, 3, 9, 0); assert(bulk_reply[5] && !valid && !reboot_at_ms);
        flash_status = 0;
    }
    /* A fresh session works after failure. HID cannot finish a bulk session. */
    bulk(OP_BEGIN, 4096, 0, 10, 0);
    bulk(OP_COMMIT, APP_BASE, 1, 10, 4096);
    report(OP_FINISH, NULL, 0); assert(response[0] == ST_ARGS && !valid);
    bulk(OP_FINISH, 0, 2, 10, 0); assert(!bulk_reply[5] && valid);
    /* Legacy HID can start again, and sparse HID has the same CRC/bounds checks. */
    begin(4200); fill(4096); commit(APP_BASE, 4096, true);
    uint8_t p[10]; wr32(p, APP_BASE + 4096); p[4] = 104; p[5] = 0;
    memset(sector_buf, 0xff, SECTOR); wr32(p + 6, crc32_of(sector_buf, 104));
    report(OP_ERASE, p, sizeof(p)); assert(response[0] == ST_OK);
    report(OP_FINISH, NULL, 0); assert(response[0] == ST_OK && valid);
}

int main(void)
{
    uint8_t stored[SECTOR], incoming[SECTOR];
    memset(stored, 0xa5, sizeof(stored)); memset(incoming, 0xa5, sizeof(incoming));
    /* Independent zlib fixtures for the table-driven CRC implementation. */
    assert(crc32_of((const uint8_t *)"123456789", 9) == 0xcbf43926);
    assert(crc32_of(incoming, 0) == 0);
    assert(crc32_of(incoming, 32) == 0x775f2f85);
    assert(crc32_of(incoming, 104) == 0x497ecff0);
    assert(crc32_of(incoming, 4096) == 0x4a9d36c6);
    assert(sector_matches(stored, incoming, SECTOR));
    assert(!sector_matches(stored, incoming, 104)); /* dirty unused tail */
    memset(stored + 104, 0xff, SECTOR - 104);
    assert(sector_matches(stored, incoming, 104));
    stored[0] ^= 1; assert(!sector_matches(stored, incoming, 104));
    assert(!sector_matches(stored, incoming, 0));
    assert(!sector_matches(stored, incoming, SECTOR + 1));
    memset(stored, 0xff, SECTOR); memset(incoming, 0xff, SECTOR);
    assert(sector_matches(stored, incoming, 104));
    stored[SECTOR - 1] = 0; assert(!sector_matches(stored, incoming, 104));
    owt_recovery_ui_init(&recovery_ui, true);
    unsigned count = replies;
    uint8_t empty[] = {REPORT_ID, CHANNEL, 0, OP_BEGIN};
    handle_report(empty, sizeof(empty));
    assert(!erases && replies == count);
    report(OP_BEGIN, (const uint8_t *)"x", 1);
    assert(response[0] == ST_ARGS && !erases);
    begin(0); assert(response[0] == ST_ARGS && !erases);
    begin(0xffffffff); assert(response[0] == ST_ARGS && !erases);
    begin(OWT_BONDS_START - OWT_APP_BASE + 1);
    assert(response[0] == ST_ARGS && !erases);
    begin(4200); assert(response[0] == ST_OK && erases == 1 && !valid);
    fill(4096); commit(OWT_APP_BASE, 4096, true);
    assert(response[0] == ST_OK && !writes && sector0_held);
    assert(owt_recovery_ui_percent(&recovery_ui) == 97);
    report(OP_FINISH, NULL, 0);
    assert(response[0] == ST_ARGS && !valid && !writes && !reboot_at_ms);
    assert(recovery_ui.phase == OWT_RECOVERY_ERROR);

    begin(4200);
    fill(104); commit(OWT_APP_BASE + 4096, 104, true);
    assert(response[0] == ST_OK && writes == 1);
    commit(OWT_APP_BASE + 4096, 104, true);
    assert(recovery_ui.received == 104);
    fill(4096); commit(OWT_APP_BASE, 4096, false);
    assert(response[0] == ST_CRC && recovery_ui.error == ST_CRC && !sector0_held);
    commit(OWT_BONDS_START, 4096, true);
    assert(response[0] == ST_ARGS && writes == 2);
    commit(OWT_APP_BASE, 4096, true);
    assert(owt_recovery_ui_percent(&recovery_ui) == 99 && !valid);
    count = replies;
    report(OP_COMMIT, NULL, 0);
    assert(replies == count); /* truncated command is never parsed */
    report(OP_FINISH, NULL, 0);
    assert(response[0] == ST_OK && response[1] && valid && writes == 3);
    assert(recovery_ui.phase == OWT_RECOVERY_COMPLETE);
    assert(owt_recovery_ui_percent(&recovery_ui) == 100 && reboot_at_ms == ticks_ms + 500);

    /* A flash failure during a duplicate write invalidates the whole session. */
    begin(4200); assert(!reboot_at_ms);
    fill(4096); commit(OWT_APP_BASE, 4096, true);
    fill(104); commit(OWT_APP_BASE + 4096, 104, true);
    flash_status = ST_VERIFY;
    commit(OWT_APP_BASE + 4096, 104, true);
    assert(response[0] == ST_VERIFY && !session);
    report(OP_FINISH, NULL, 0);
    assert(response[0] == ST_ARGS && !valid && !reboot_at_ms);
    flash_status = ST_FLASH;
    begin(4200); assert(response[0] == ST_FLASH && !session);
    assert(recovery_ui.phase == OWT_RECOVERY_ERROR);
    flash_status = 0;

    report(OP_BEGIN, NULL, 0); /* old client */
    assert(response[0] == ST_OK && !recovery_ui.total);
    fill(4096); commit(OWT_APP_BASE, 4096, true);
    assert(owt_recovery_ui_percent(&recovery_ui) == 0);
    report(OP_FINISH, NULL, 0);
    assert(response[0] == ST_OK && valid);
    test_bulk();
    puts("recovery protocol: invalid/legacy BEGIN, CRC, retries, incomplete FINISH and flash failures passed");
}
