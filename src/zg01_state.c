#ifdef __KERNEL__
#include <linux/errno.h>
#include <linux/string.h>
#else
#include <errno.h>
#include <string.h>
#endif

#include "zg01_state.h"

#ifndef ARRAY_SIZE
#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))
#endif

struct zg01_state_field_entry {
    u8 field;
    u16 offset;
    u8 min;
    u8 max;
};

/*
 * Payload offsets, confirmed by single-control captures against the
 * persisted state.  The transformed fields (gate threshold, gate
 * attack/hold/decay, the compressor/limiter attack/release/out gain and
 * the EQ gain/frequency biquads) are intentionally not in this table;
 * see the state dump protocol document for their offsets and why they
 * cannot be applied to the UI cache directly.
 *
 * Gate Attack/Hold/Decay used to be treated as direct, but their ALSA
 * ranges are 57..283, 0..213 and 24..300, which a single 7-bit payload
 * byte cannot represent.  Until the multi-byte encoding is captured they
 * stay unknown rather than exposing a truncated value as known.
 */
static const struct zg01_state_field_entry zg01_state_fields[] = {
    { ZG01_SF_GATE_SWITCH, 1460, 0, 1 },
    { ZG01_SF_GATE_VALUE, 1461, 0, 100 },
    { ZG01_SF_GATE_RANGE, 1464, 0, 73 },
    { ZG01_SF_COMP_SWITCH, 1470, 0, 1 },
    { ZG01_SF_COMP_VALUE, 1472, 0, 120 },
    { ZG01_SF_COMP_THRESHOLD, 1473, 19, 73 },
    { ZG01_SF_COMP_RATIO, 1474, 0, 120 },
    { ZG01_SF_COMP_KNEE, 1480, 0, 2 },
    { ZG01_SF_LIM_SWITCH, 1661, 0, 1 },
    { ZG01_SF_LIM_VALUE, 1662, 0, 120 },
    { ZG01_SF_LIM_THRESHOLD, 1664, 19, 73 },
    { ZG01_SF_LIM_KNEE, 1669, 0, 2 },
    { ZG01_SF_EQ_SWITCH, 1628, 0, 1 },
    { ZG01_SF_EQ_LOW_SHAPE, 1634, 0, 1 },
    { ZG01_SF_EQ_HIGH_SHAPE, 1637, 0, 1 },
    { ZG01_SF_EQ_Q1, 1657, 0, 60 },
    { ZG01_SF_EQ_Q2, 1658, 0, 60 },
    { ZG01_SF_EQ_Q3, 1659, 0, 60 },
    { ZG01_SF_EQ_Q4, 1660, 0, 60 },
};

/*
 * The first data packet of each of the eight frames.  The first frame
 * carries the SysEx header; the other seven are stable descriptors in the
 * observed firmware.  Checking all eight rejects a duplicated, dropped
 * or interleaved frame before its bytes reach a field offset.
 */
static const u8 zg01_state_frame_prefix[ZG01_STATE_FRAME_COUNT][3] = {
    { 0xf0, 0x43, 0x00 },
    { 0x00, 0x40, 0x01 },
    { 0x01, 0x01, 0x00 },
    { 0x01, 0x5f, 0x01 },
    { 0x00, 0x00, 0x00 },
    { 0x00, 0x00, 0x00 },
    { 0x00, 0x00, 0x00 },
    { 0x01, 0x00, 0x01 },
};

/* The payload index is the d0..d2 byte of the corresponding packet. */
static u8 zg01_state_payload_byte(const u8 *frames, unsigned int index)
{
    unsigned int packet = index / 3;
    unsigned int byte = index % 3;

    return frames[packet * 4 + 1 + byte];
}

static const u8 zg01_state_header[] = {
    0xf0, 0x43, 0x00, 0x3e, 0x1b, 0x1f, 0x14, 0x01,
};

int zg01_state_collect(struct zg01_state_collector *collector,
                       const u8 *frame, unsigned int len)
{
    if (!collector || !frame || len != ZG01_STATE_FRAME_SIZE)
        return -EINVAL;
    if (collector->count >= ZG01_STATE_FRAME_COUNT)
        return -EINVAL;
    if (collector->count == 0 &&
        !(frame[0] == 0x04 && frame[1] == 0xf0 && frame[2] == 0x43 &&
          frame[3] == 0x00))
        return -EINVAL;

    memcpy(collector->frames +
               collector->count * ZG01_STATE_FRAME_SIZE,
           frame, ZG01_STATE_FRAME_SIZE);
    collector->count++;
    return 0;
}

int zg01_state_parse(const u8 *frames, unsigned int len,
                     struct zg01_state_values *out)
{
    unsigned int i;
    unsigned int frame;

    if (!frames || !out || len != ZG01_STATE_DUMP_SIZE)
        return -EINVAL;

    /*
     * The first packet is a USB-MIDI continuation carrying the Yamaha
     * system-exclusive header.  The descriptor/version bytes are part of
     * the contract: the fixed payload offsets below are only valid for
     * this layout.  A meter frame, a short read or an unknown firmware
     * must never be interpreted as state.
     */
    if (frames[0] != 0x04)
        return -EINVAL;
    for (i = 0; i < ARRAY_SIZE(zg01_state_header); i++) {
        if (zg01_state_payload_byte(frames, i) != zg01_state_header[i])
            return -EINVAL;
    }

    /*
     * Every USB-MIDI packet of the dump is a 0x04 continuation and every
     * data byte is 7-bit.  A 0xff CIN or a high-bit byte is a malformed
     * transfer, not a state frame.
     */
    for (i = 0; i < ZG01_STATE_DUMP_SIZE / 4; i++) {
        const u8 *packet = frames + i * 4;
        unsigned int j;

        if (packet[0] != 0x04)
            return -EINVAL;
        for (j = 1; j < 4; j++) {
            /*
             * The SysEx start/end bytes are the only full 8-bit values
             * in the USB-MIDI stream; every payload value is 7-bit.
             */
            if (packet[j] > 0x7f && packet[j] != 0xf0 &&
                packet[j] != 0xf7)
                return -EINVAL;
        }
    }

    /*
     * Each frame starts with the observed descriptor packet.  This is the
     * structural guard against a duplicated, dropped or interleaved frame
     * being spliced into the dump; individual field ranges below catch
     * value corruption.
     */
    for (frame = 0; frame < ZG01_STATE_FRAME_COUNT; frame++) {
        const u8 *first = frames + frame * ZG01_STATE_FRAME_SIZE;

        if (first[1] != zg01_state_frame_prefix[frame][0] ||
            first[2] != zg01_state_frame_prefix[frame][1] ||
            first[3] != zg01_state_frame_prefix[frame][2])
            return -EINVAL;
    }

    memset(out, 0, sizeof(*out));
    for (i = 0; i < ARRAY_SIZE(zg01_state_fields); i++) {
        const struct zg01_state_field_entry *f = &zg01_state_fields[i];
        u8 value = zg01_state_payload_byte(frames, f->offset);

        if (value < f->min || value > f->max)
            return -EINVAL;
        out->value[f->field] = value;
        out->known |= ZG01_STATE_KNOWN(f->field);
    }

    return 0;
}
