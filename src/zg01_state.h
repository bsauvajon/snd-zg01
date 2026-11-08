/*
 * Yamaha ZG01 USB Audio Driver - MIC state dump parser
 *
 * The type-20 request (see docs/STATE_DUMP_PROTOCOL.md) makes the device
 * push eight 512-byte USB-MIDI frames.  Each four-byte packet is
 * [CIN][d0][d1][d2]; the concatenated d0..d2 bytes form a 3072-byte
 * payload whose fixed offsets hold the MIC state.
 *
 * Only fields whose payload byte is the value the ALSA control expects
 * are listed here.  The DSP-transformed fields (attack/release/out gain,
 * the gate threshold, the EQ biquad coefficients) are deliberately
 * absent: writing a transformed byte into a UI cache slot would display
 * a plausible but wrong value.
 */

#ifndef ZG01_STATE_H
#define ZG01_STATE_H

#ifdef __KERNEL__
#include <linux/types.h>
#else
#include <stdint.h>
typedef uint8_t u8;
typedef uint16_t u16;
typedef uint32_t u32;
#endif

#define ZG01_STATE_FRAME_SIZE 512
#define ZG01_STATE_FRAME_COUNT 8
#define ZG01_STATE_DUMP_SIZE (ZG01_STATE_FRAME_SIZE * ZG01_STATE_FRAME_COUNT)
#define ZG01_STATE_PAYLOAD_SIZE 3072

/* Direct fields, in payload order.  Keep the enum below the 32-bit mask. */
enum zg01_state_field {
    ZG01_SF_GATE_SWITCH,
    ZG01_SF_GATE_VALUE,
    ZG01_SF_GATE_RANGE,
    ZG01_SF_GATE_ATTACK,
    ZG01_SF_GATE_HOLD,
    ZG01_SF_GATE_DECAY,
    ZG01_SF_COMP_SWITCH,
    ZG01_SF_COMP_VALUE,
    ZG01_SF_COMP_THRESHOLD,
    ZG01_SF_COMP_RATIO,
    ZG01_SF_COMP_KNEE,
    ZG01_SF_LIM_SWITCH,
    ZG01_SF_LIM_VALUE,
    ZG01_SF_LIM_THRESHOLD,
    ZG01_SF_LIM_KNEE,
    ZG01_SF_EQ_SWITCH,
    ZG01_SF_EQ_LOW_SHAPE,
    ZG01_SF_EQ_HIGH_SHAPE,
    ZG01_SF_EQ_Q1,
    ZG01_SF_EQ_Q2,
    ZG01_SF_EQ_Q3,
    ZG01_SF_EQ_Q4,
    ZG01_SF_COUNT,
};

#define ZG01_STATE_KNOWN(field) (1u << (field))

struct zg01_state_values {
    u8 value[ZG01_SF_COUNT];
    u32 known; /* ZG01_STATE_KNOWN() bits for the fields actually parsed */
};

/*
 * Parse one complete dump.  Returns 0 on success, -EINVAL for a short or
 * unknown frame set.  The caller must pass exactly ZG01_STATE_DUMP_SIZE
 * bytes; a partial read is rejected rather than parsed with stale bytes.
 */
int zg01_state_parse(const u8 *frames, unsigned int len,
                     struct zg01_state_values *out);

#endif /* ZG01_STATE_H */
