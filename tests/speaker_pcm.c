/* Host check: cc -std=c11 -Wall -Wextra -Werror tests/speaker_pcm.c -o /tmp/owt-speaker-test */
#include "../native/src/speaker_pcm.h"
#include <assert.h>
#include <string.h>

int main(void)
{
    const int16_t stereo[] = {32767, 32767, -32768, -32768, 12000, 0, 0, -12000, 32767, -32768};
    const int16_t expected[] = {32767, 32767, -32768, -32768, 6000, 6000, -6000, -6000, 0, 0};
    int16_t copy[10], mono[10];
    memcpy(copy, stereo, sizeof(copy));
    owt_speaker_mono((const uint8_t *)copy, (uint8_t *)mono, sizeof(copy));
    assert(memcmp(copy, stereo, sizeof(copy)) == 0);
    assert(memcmp(mono, expected, sizeof(mono)) == 0);
    assert(owt_speaker_volume(0, 15) == 0);
    assert(owt_speaker_volume(64, 15) == 8);
    assert(owt_speaker_volume(127, 15) == 15);
    assert(owt_speaker_volume(255, 15) == 15);
    for (unsigned n = 1; n <= 127; n++) {
        assert(owt_speaker_volume(n, 15) >= owt_speaker_volume(n - 1, 15));
        assert(owt_speaker_volume(n, 15) <= 15);
    }
    return 0;
}
