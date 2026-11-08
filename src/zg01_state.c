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
};

/*
 * Payload offsets, confirmed by single-control captures against the
 * persisted state.  The transformed fields (gate threshold 1462, the
 * compressor/limiter attack/release/out gain and the EQ gain/frequency
 * biquads) are intentionally not in this table; see the state dump
 * protocol document for their offsets and why they cannot be applied to
 * the UI cache directly.
 */
static const struct zg01_state_field_entry zg01_state_fields[] = {
    { ZG01_SF_GATE_SWITCH, 1460 },
    { ZG01_SF_GATE_VALUE, 1461 },
    { ZG01_SF_GATE_RANGE, 1464 },
    { ZG01_SF_GATE_ATTACK, 1465 },
    { ZG01_SF_GATE_HOLD, 1467 },
    { ZG01_SF_GATE_DECAY, 1468 },
    { ZG01_SF_COMP_SWITCH, 1470 },
    { ZG01_SF_COMP_VALUE, 1472 },
    { ZG01_SF_COMP_THRESHOLD, 1473 },
    { ZG01_SF_COMP_RATIO, 1474 },
    { ZG01_SF_COMP_KNEE, 1480 },
    { ZG01_SF_LIM_SWITCH, 1661 },
    { ZG01_SF_LIM_VALUE, 1662 },
    { ZG01_SF_LIM_THRESHOLD, 1664 },
    { ZG01_SF_LIM_KNEE, 1669 },
    { ZG01_SF_EQ_SWITCH, 1628 },
    { ZG01_SF_EQ_LOW_SHAPE, 1634 },
    { ZG01_SF_EQ_HIGH_SHAPE, 1637 },
    { ZG01_SF_EQ_Q1, 1657 },
    { ZG01_SF_EQ_Q2, 1658 },
    { ZG01_SF_EQ_Q3, 1659 },
    { ZG01_SF_EQ_Q4, 1660 },
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

int zg01_state_parse(const u8 *frames, unsigned int len,
                     struct zg01_state_values *out)
{
    unsigned int i;

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

    memset(out, 0, sizeof(*out));
    for (i = 0; i < ARRAY_SIZE(zg01_state_fields); i++) {
        const struct zg01_state_field_entry *f = &zg01_state_fields[i];
        u8 value = zg01_state_payload_byte(frames, f->offset);

        if (value > 0x7f)
            return -EINVAL;
        out->value[f->field] = value;
        out->known |= ZG01_STATE_KNOWN(f->field);
    }

    return 0;
}
