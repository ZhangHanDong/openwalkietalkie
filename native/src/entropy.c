/* SPDX-License-Identifier: MIT */
#include <rtthread.h>
#include <limits.h>
#include "bf0_hal.h"
#include "bf0_sibles_nvds.h"
#include "bf0_ble_gap.h"

static RNG_HandleTypeDef rng;
static struct rt_mutex mutex;

static int entropy_init(void)
{
    uint32_t seed;
    rt_mutex_init(&mutex, "entropy", RT_IPC_FLAG_PRIO);
    HAL_RCC_EnableModule(RCC_MOD_TRNG);
    rng.Instance = hwp_trng;
    RT_ASSERT(HAL_RNG_Init(&rng) == HAL_OK);
    RT_ASSERT(HAL_RNG_GenerateRandomSeed(&rng, &seed) == HAL_OK);
    return RT_EOK;
}
INIT_PREV_EXPORT(entropy_init);

/* The SDK's legacy SMP key generation calls rand() once per key byte.
 * Resolve those calls to hardware entropy rather than the C library PRNG. */
int __wrap_rand(void)
{
    uint32_t value;
    RT_ASSERT(rt_mutex_take(&mutex, RT_WAITING_FOREVER) == RT_EOK);
    HAL_StatusTypeDef result = HAL_RNG_GenerateRandomNumber(&rng, &value);
    rt_mutex_release(&mutex);
    RT_ASSERT(result == HAL_OK);
    return value & INT_MAX;
}

ble_common_update_type_t ble_request_local_irk(ble_gap_sec_key_t *key)
{
    for (unsigned i = 0; i < sizeof(key->key); i++) key->key[i] = (uint8_t)__wrap_rand();
    return BLE_UPDATE_ONCE;
}
