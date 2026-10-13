/* SPDX-License-Identifier: MIT
 * macOS: cc tools/test-speaker.c -framework AudioToolbox -framework CoreAudio
 *          -framework CoreFoundation -o /tmp/openwalkietalkie-speaker-test
 * --list reports OpenWalkieTalkie's output. --tone plays three quiet channel checks
 * directly to it, without changing the Mac's default output or saving audio.
 * --loud plays the same three checks at full scale with the device's own
 * volume (the deck's AVRCP volume) turned up to maximum for the duration and
 * put back afterwards: the loudest the speaker can be driven from the Mac.
 * --silence briefly opens a silent stream to check early-suspend cleanup.
 * --play file.wav [--volume PCT] plays a 16-bit PCM WAV (for example one
 * recorded by tools/test-microphone.c) to the deck, at PCT percent of the
 * device's volume (default 50, restored afterwards): the round-trip check.
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
static CFStringRef output_uid(void)
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
        if (!match) continue;
        UInt32 out = channels(devices[i], kAudioDevicePropertyScopeOutput);
        UInt32 in = channels(devices[i], kAudioDevicePropertyScopeInput);
        Float64 rate = 0; length = sizeof(rate); a.mSelector = kAudioDevicePropertyNominalSampleRate;
        AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, &rate);
        printf("OpenWalkieTalkie: inputs=%u outputs=%u current_rate=%.0f\n", in, out, rate);
        if (out) {
            CFStringRef uid = NULL; length = sizeof(uid); a.mSelector = kAudioDevicePropertyDeviceUID;
            if (!AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, &uid)) return uid;
        }
    }
    return NULL;
}
static void completed(void *context, AudioQueueRef queue, AudioQueueBufferRef buffer)
{ (void)context; (void)queue; (void)buffer; }

/* The output device behind `uid`, for its volume control. */
static AudioDeviceID device_for(CFStringRef uid)
{
    AudioObjectPropertyAddress a = {kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    AudioDeviceID devices[64]; UInt32 size = sizeof(devices);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &a, 0, NULL, &size, devices)) return kAudioObjectUnknown;
    for (UInt32 i = 0; i < size / sizeof(devices[0]); i++) {
        CFStringRef other = NULL; UInt32 length = sizeof(other);
        a.mSelector = kAudioDevicePropertyDeviceUID;
        if (AudioObjectGetPropertyData(devices[i], &a, 0, NULL, &length, &other)) continue;
        bool match = CFEqual(other, uid);
        CFRelease(other);
        if (match) return devices[i];
    }
    return kAudioObjectUnknown;
}

/* Set the device's output volume (main element, else both channels); returns
 * the previous value or -1 when the device has no volume control. */
static float set_volume(AudioDeviceID device, float value)
{
    float previous = -1;
    for (UInt32 element = 0; element <= 2; element++) {
        AudioObjectPropertyAddress a = {kAudioDevicePropertyVolumeScalar, kAudioDevicePropertyScopeOutput, element};
        Boolean settable = false;
        if (!AudioObjectHasProperty(device, &a) || AudioObjectIsPropertySettable(device, &a, &settable) || !settable) continue;
        Float32 current = 0; UInt32 size = sizeof(current);
        if (!AudioObjectGetPropertyData(device, &a, 0, NULL, &size, &current) && previous < 0) previous = current;
        Float32 wanted = value;
        AudioObjectSetPropertyData(device, &a, 0, NULL, sizeof(wanted), &wanted);
        if (element == 0) break;
    }
    return previous;
}

/* Play a 16-bit PCM WAV file to the deck at `percent` of the device volume. */
static int play_file(CFStringRef uid, const char *path, int percent)
{
    FILE *f = fopen(path, "rb");
    if (!f) { perror(path); return 4; }
    uint8_t header[12]; if (fread(header, 1, 12, f) != 12 || memcmp(header, "RIFF", 4) || memcmp(header + 8, "WAVE", 4)) { fprintf(stderr, "%s: not a WAV file\n", path); return 4; }
    uint16_t channels = 0, bits = 0; uint32_t rate = 0; long data_start = 0; uint32_t data_bytes = 0;
    for (;;) {
        uint8_t chunk[8]; if (fread(chunk, 1, 8, f) != 8) break;
        uint32_t size = chunk[4] | chunk[5] << 8 | chunk[6] << 16 | (uint32_t)chunk[7] << 24;
        if (!memcmp(chunk, "fmt ", 4)) {
            uint8_t fmt[16]; if (fread(fmt, 1, 16, f) != 16) break;
            channels = fmt[2] | fmt[3] << 8; rate = fmt[4] | fmt[5] << 8 | fmt[6] << 16 | (uint32_t)fmt[7] << 24; bits = fmt[14] | fmt[15] << 8;
            fseek(f, size - 16, SEEK_CUR);
        } else if (!memcmp(chunk, "data", 4)) { data_start = ftell(f); data_bytes = size; break; }
        else fseek(f, size, SEEK_CUR);
    }
    if (!rate || bits != 16 || !channels || !data_bytes) { fprintf(stderr, "%s: need 16-bit PCM with a data chunk (rate=%u bits=%u channels=%u)\n", path, rate, bits, channels); fclose(f); return 4; }
    AudioDeviceID device = device_for(uid);
    float previous = device != kAudioObjectUnknown ? set_volume(device, percent / 100.0f) : -1;
    if (previous >= 0) printf("OpenWalkieTalkie volume %.0f%% -> %d%% for the playback\n", previous * 100, percent);
    AudioStreamBasicDescription format = {.mSampleRate=rate, .mFormatID=kAudioFormatLinearPCM,
        .mFormatFlags=kAudioFormatFlagIsSignedInteger|kAudioFormatFlagIsPacked,
        .mBytesPerPacket=(UInt32)(2*channels), .mFramesPerPacket=1, .mBytesPerFrame=(UInt32)(2*channels),
        .mChannelsPerFrame=channels, .mBitsPerChannel=16};
    AudioQueueRef queue = NULL;
    OSStatus status = AudioQueueNewOutput(&format, completed, NULL, NULL, NULL, 0, &queue);
    if (!status) status = AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uid, sizeof(uid));
    if (!status) status = AudioQueueSetParameter(queue, kAudioQueueParam_Volume, 1.0f);
    fseek(f, data_start, SEEK_SET);
    uint32_t remaining = data_bytes, chunk_bytes = rate * 2 * channels / 2; /* half a second per buffer */
    while (!status && remaining) {
        AudioQueueBufferRef buffer = NULL;
        uint32_t want = remaining < chunk_bytes ? remaining : chunk_bytes;
        status = AudioQueueAllocateBuffer(queue, want, &buffer);
        if (status) break;
        size_t got = fread(buffer->mAudioData, 1, want, f);
        if (!got) break;
        buffer->mAudioDataByteSize = (UInt32)got; remaining -= (uint32_t)got;
        status = AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
    }
    fclose(f);
    double seconds = (double)data_bytes / (rate * 2.0 * channels);
    if (!status) status = AudioQueueStart(queue, NULL);
    if (!status) {
        printf("Playing %s (%.1f s, %u Hz, %u channel%s) on OpenWalkieTalkie\n", path, seconds, rate, channels, channels == 1 ? "" : "s"); fflush(stdout);
        CFAbsoluteTime end = CFAbsoluteTimeGetCurrent() + seconds + 1.0;
        while (CFAbsoluteTimeGetCurrent() < end) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
    }
    if (queue) { AudioQueueStop(queue, true); AudioQueueDispose(queue, true); }
    if (previous >= 0) { set_volume(device, previous); printf("OpenWalkieTalkie volume restored to %.0f%%\n", previous * 100); }
    if (status) fprintf(stderr, "Core Audio playback failed: %d\n", (int)status);
    return status ? 4 : 0;
}

int main(int argc, char **argv)
{
    if (argc >= 3 && !strcmp(argv[1], "--play")) {
        int percent = 50;
        if (argc == 5 && !strcmp(argv[3], "--volume")) percent = atoi(argv[4]);
        else if (argc != 3) { fprintf(stderr, "Usage: %s --play file.wav [--volume PCT]\n", argv[0]); return 2; }
        if (percent < 1 || percent > 100) { fprintf(stderr, "volume must be 1..100\n"); return 2; }
        CFStringRef uid = output_uid();
        if (!uid) { fprintf(stderr, "OpenWalkieTalkie has no connected audio output\n"); return 3; }
        int result = play_file(uid, argv[2], percent);
        CFRelease(uid);
        return result;
    }
    if (argc != 2 || (strcmp(argv[1], "--list") && strcmp(argv[1], "--tone") && strcmp(argv[1], "--loud") && strcmp(argv[1], "--silence"))) {
        fprintf(stderr, "Usage: %s --list | --tone | --loud | --silence | --play file.wav [--volume PCT]\n", argv[0]); return 2;
    }
    CFStringRef uid = output_uid();
    if (!uid) { fprintf(stderr, "OpenWalkieTalkie has no connected audio output\n"); return 3; }
    if (!strcmp(argv[1], "--list")) { CFRelease(uid); return 0; }
    bool silence = !strcmp(argv[1], "--silence");
    bool loud = !strcmp(argv[1], "--loud");
    AudioDeviceID device = loud ? device_for(uid) : kAudioObjectUnknown;
    float previous_volume = -1;
    if (loud && device != kAudioObjectUnknown) {
        previous_volume = set_volume(device, 1.0f);
        if (previous_volume >= 0) printf("OpenWalkieTalkie volume %.0f%% -> 100%% for this test\n", previous_volume * 100);
        else puts("OpenWalkieTalkie exposes no volume control; playing full-scale at the deck's current volume");
    }
    AudioStreamBasicDescription format = {.mSampleRate=48000, .mFormatID=kAudioFormatLinearPCM,
        .mFormatFlags=kAudioFormatFlagIsSignedInteger|kAudioFormatFlagIsPacked,
        .mBytesPerPacket=4, .mFramesPerPacket=1, .mBytesPerFrame=4,
        .mChannelsPerFrame=2, .mBitsPerChannel=16};
    AudioQueueRef queue = NULL;
    OSStatus status = AudioQueueNewOutput(&format, completed, NULL, NULL, NULL, 0, &queue);
    if (!status) status = AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uid, sizeof(uid));
    if (!status) status = AudioQueueSetParameter(queue, kAudioQueueParam_Volume, loud ? 1.0f : 0.25f);
    for (unsigned part = 0; !status && part < 3; part++) {
        AudioQueueBufferRef buffer = NULL;
        status = AudioQueueAllocateBuffer(queue, 48000 * 4 * 2, &buffer);
        if (status) break;
        int16_t *pcm = buffer->mAudioData;
        for (unsigned frame = 0; frame < 96000; frame++) {
            // One-second tone followed by one second of silence. Short ramps
            // avoid clicks; 660/880/440Hz distinguish left, right and both.
            double envelope = frame < 48000 ? fmin(1.0, fmin(frame / 2400.0, (48000-frame) / 2400.0)) : 0;
            double hz = part == 0 ? 660 : part == 1 ? 880 : 440;
            double amplitude = loud ? 32000 : 8192;
            int16_t sample = silence ? 0 : (int16_t)(amplitude * envelope * sin(2 * 3.141592653589793 * hz * frame / 48000));
            pcm[frame * 2] = part == 1 ? 0 : sample;
            pcm[frame * 2 + 1] = part == 0 ? 0 : sample;
        }
        buffer->mAudioDataByteSize = 96000 * 4;
        status = AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
    }
    if (!status) status = AudioQueueStart(queue, NULL);
    if (!status) {
        puts(silence ? "Opening a short silent stream on OpenWalkieTalkie" :
             loud ? "Playing FULL-SCALE left, right, then both-channel tones on OpenWalkieTalkie at maximum volume" :
                    "Playing quiet left, right, then both-channel tones on OpenWalkieTalkie"); fflush(stdout);
        CFAbsoluteTime end = CFAbsoluteTimeGetCurrent() + (silence ? 1 : 7);
        while (CFAbsoluteTimeGetCurrent() < end) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
    }
    if (queue) { AudioQueueStop(queue, true); AudioQueueDispose(queue, true); }
    if (loud && previous_volume >= 0) { set_volume(device, previous_volume); printf("OpenWalkieTalkie volume restored to %.0f%%\n", previous_volume * 100); }
    CFRelease(uid);
    if (status) fprintf(stderr, "Core Audio playback failed: %d\n", (int)status);
    return status ? 4 : 0;
}
