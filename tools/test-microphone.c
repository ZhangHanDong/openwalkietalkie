/* SPDX-License-Identifier: MIT
 * macOS: cc tools/test-microphone.c -framework AudioToolbox -framework CoreAudio
 *          -framework CoreFoundation -o /tmp/openwalkietalkie-microphone-test
 * Records from the deck's Bluetooth voice input ("OpenWalkieTalkie", 16 kHz mono)
 * for N seconds, straight from that device without changing the Mac's default
 * input, and prints the peak and RMS of every second so a tap or a word near
 * the microphone shows up as numbers. Recording from the HFP input is what
 * makes macOS open the voice link, so the board's own `microphone:` log line
 * appears at the same time. Optionally writes the samples to a WAV file.
 *
 *   test-microphone [--seconds N] [--out file.wav]
 */
#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static UInt32 channels(AudioDeviceID device, AudioObjectPropertyScope scope)
{
    AudioObjectPropertyAddress a = {kAudioDevicePropertyStreamConfiguration, scope, kAudioObjectPropertyElementMain};
    UInt32 size = 0, count = 0;
    if (AudioObjectGetPropertyDataSize(device, &a, 0, NULL, &size)) return 0;
    AudioBufferList *list = malloc(size);
    if (list && !AudioObjectGetPropertyData(device, &a, 0, NULL, &size, list))
        for (UInt32 i = 0; i < list->mNumberBuffers; i++) count += list->mBuffers[i].mNumberChannels;
    free(list);
    return count;
}

/* The deck's input device UID and its nominal rate. */
static CFStringRef input_uid(double *rate)
{
    AudioObjectPropertyAddress a = {kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    AudioDeviceID devices[64]; UInt32 size = sizeof(devices);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &a, 0, NULL, &size, devices)) return NULL;
    for (UInt32 i = 0; i < size / sizeof(devices[0]); i++) {
        CFStringRef name = NULL; UInt32 length = sizeof(name);
        a.mSelector = kAudioObjectPropertyName;
        if (AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, &name)) continue;
        bool match = CFEqual(name, CFSTR("OpenWalkieTalkie"));
        CFRelease(name);
        if (!match || channels(devices[i], kAudioDevicePropertyScopeInput) == 0) continue;
        a.mSelector = kAudioDevicePropertyNominalSampleRate; length = sizeof(*rate);
        AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, rate);
        CFStringRef uid = NULL; length = sizeof(uid);
        a.mSelector = kAudioDevicePropertyDeviceUID;
        if (AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, &uid)) continue;
        return uid;
    }
    return NULL;
}

struct recorder {
    FILE *wav;
    uint32_t bytes;
    uint64_t frames_in_second, frames_total;
    double sum_squares;
    int16_t peak;
    unsigned second;
    double rate;
};

static void report(struct recorder *r)
{
    double rms = r->frames_in_second ? sqrt(r->sum_squares / (double)r->frames_in_second) : 0;
    printf("second %u: peak=%d rms=%.0f (%s)\n", r->second + 1, r->peak, rms,
           r->peak > 2000 ? "sound" : r->peak > 150 ? "quiet" : "silence");
    fflush(stdout);
    r->frames_in_second = 0; r->sum_squares = 0; r->peak = 0; r->second++;
}

static void input_callback(void *user, AudioQueueRef queue, AudioQueueBufferRef buffer,
                           const AudioTimeStamp *time, UInt32 packets, const AudioStreamPacketDescription *desc)
{
    struct recorder *r = user;
    const int16_t *samples = buffer->mAudioData;
    UInt32 count = buffer->mAudioDataByteSize / sizeof(int16_t);
    for (UInt32 i = 0; i < count; i++) {
        int16_t s = samples[i];
        int16_t m = s < 0 ? (int16_t)-s : s;
        if (m > r->peak) r->peak = m;
        r->sum_squares += (double)s * s;
        if (++r->frames_in_second >= (uint64_t)r->rate) report(r);
    }
    r->frames_total += count;
    if (r->wav && count) { fwrite(samples, sizeof(int16_t), count, r->wav); r->bytes += count * sizeof(int16_t); }
    AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
}

static void write_wav_header(FILE *f, uint32_t data_bytes, uint32_t rate)
{
    uint32_t byte_rate = rate * 2, chunk = 36 + data_bytes;
    uint16_t channels = 1, align = 2, bits = 16, fmt = 1; uint32_t fmt_len = 16;
    fseek(f, 0, SEEK_SET);
    fwrite("RIFF", 1, 4, f); fwrite(&chunk, 4, 1, f); fwrite("WAVEfmt ", 1, 8, f);
    fwrite(&fmt_len, 4, 1, f); fwrite(&fmt, 2, 1, f); fwrite(&channels, 2, 1, f);
    fwrite(&rate, 4, 1, f); fwrite(&byte_rate, 4, 1, f); fwrite(&align, 2, 1, f); fwrite(&bits, 2, 1, f);
    fwrite("data", 1, 4, f); fwrite(&data_bytes, 4, 1, f);
}

int main(int argc, char **argv)
{
    int seconds = 5; const char *out = NULL;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out = argv[++i];
        else { fprintf(stderr, "Usage: %s [--seconds N] [--out file.wav]\n", argv[0]); return 2; }
    }
    double rate = 16000;
    CFStringRef uid = input_uid(&rate);
    if (!uid) { fprintf(stderr, "OpenWalkieTalkie has no connected audio input (connect its Classic profiles first)\n"); return 3; }
    printf("Recording %d s from OpenWalkieTalkie at %.0f Hz; tap or speak near the microphone\n", seconds, rate);
    fflush(stdout);

    struct recorder r = {0}; r.rate = rate;
    if (out) { r.wav = fopen(out, "wb"); if (!r.wav) { perror(out); return 4; } write_wav_header(r.wav, 0, (uint32_t)rate); }
    AudioStreamBasicDescription format = {0};
    format.mSampleRate = rate; format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked;
    format.mBitsPerChannel = 16; format.mChannelsPerFrame = 1; format.mBytesPerFrame = 2;
    format.mFramesPerPacket = 1; format.mBytesPerPacket = 2;
    AudioQueueRef queue = NULL;
    OSStatus status = AudioQueueNewInput(&format, input_callback, &r, NULL, kCFRunLoopCommonModes, 0, &queue);
    if (status) { fprintf(stderr, "AudioQueueNewInput failed: %d\n", (int)status); return 5; }
    status = AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uid, sizeof(uid));
    if (status) { fprintf(stderr, "cannot select OpenWalkieTalkie: %d\n", (int)status); return 6; }
    for (int i = 0; i < 4; i++) {
        AudioQueueBufferRef buffer = NULL;
        AudioQueueAllocateBuffer(queue, (UInt32)(rate / 10) * 2, &buffer);
        AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
    }
    status = AudioQueueStart(queue, NULL);
    if (status) { fprintf(stderr, "AudioQueueStart failed: %d\n", (int)status); return 7; }
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds + 1.0, false);
    AudioQueueStop(queue, true);
    AudioQueueDispose(queue, true);
    if (r.frames_in_second) report(&r);
    if (r.wav) { write_wav_header(r.wav, r.bytes, (uint32_t)rate); fclose(r.wav); printf("wrote %s (%u bytes of PCM)\n", out, r.bytes); }
    printf("%llu frames in total\n", (unsigned long long)r.frames_total);
    return r.frames_total ? 0 : 8;
}
