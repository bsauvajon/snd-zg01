/*
 * ZG01 meter frame parser (bulk IN 0x83), shared by the kernel module and
 * the userspace tests.  Mirrors tools/decode-meters.py; see
 * docs/METER_PROTOCOL.md.
 */

#ifndef ZG01_METERS_H
#define ZG01_METERS_H

#ifdef __KERNEL__
#include <linux/errno.h>
#else
#include <errno.h>
#endif

#define ZG01_METER_FRAME_LEN      192

#define ZG01_METER_MIC_IN_HI      10
#define ZG01_METER_MIC_IN_LO      11
#define ZG01_METER_MIC_OUT_HI     15
#define ZG01_METER_MIC_OUT_LO     17
#define ZG01_METER_PC_IN_HI       159
#define ZG01_METER_PC_IN_LO       161
#define ZG01_METER_STREAM_MIX_HI  181
#define ZG01_METER_STREAM_MIX_LO  182

#define ZG01_METER_ZERO_CODE      16384
#define ZG01_METER_CODES_PER_DB   10

struct zg01_meter_values {
    int mic_in;
    int mic_out;
    int pc_in;
    int stream_mix;
};

/* Round codes / 10 to the nearest dB, ties to even (as Python round()). */
static inline int zg01_meter_round_db(int codes)
{
    int db = codes / ZG01_METER_CODES_PER_DB;
    int rem = codes % ZG01_METER_CODES_PER_DB;

    if (rem < 0) {
        db--;
        rem += ZG01_METER_CODES_PER_DB;
    }
    if (rem > ZG01_METER_CODES_PER_DB / 2 ||
        (rem == ZG01_METER_CODES_PER_DB / 2 && db % 2 != 0))
        db++;
    return db;
}

static inline int zg01_meter_level(const unsigned char *buf,
                                   unsigned int hi, unsigned int lo)
{
    int code = buf[hi] * 128 + buf[lo];
    int level = zg01_meter_round_db(code - ZG01_METER_ZERO_CODE) + 127;

    if (level < 0)
        return 0;
    if (level > 127)
        return 127;
    return level;
}

static inline int zg01_meter_parse(const unsigned char *buf, unsigned int len,
                                   struct zg01_meter_values *out)
{
    static const unsigned char header[8] = {
        0x04, 0xf0, 0x43, 0x10, 0x04, 0x3e, 0x14, 0x02
    };
    struct zg01_meter_values v;
    unsigned int i;

    if (len != ZG01_METER_FRAME_LEN)
        return -EINVAL;
    for (i = 0; i < sizeof(header); i++) {
        if (buf[i] != header[i])
            return -EINVAL;
    }
    for (i = 0; i < ZG01_METER_FRAME_LEN - 4; i += 4) {
        if (buf[i] != 0x04)
            return -EINVAL;
    }
    if (buf[ZG01_METER_FRAME_LEN - 4] != 0x07 ||
        buf[ZG01_METER_FRAME_LEN - 1] != 0xf7)
        return -EINVAL;

    v.mic_in = zg01_meter_level(buf, ZG01_METER_MIC_IN_HI,
                                ZG01_METER_MIC_IN_LO);
    v.mic_out = zg01_meter_level(buf, ZG01_METER_MIC_OUT_HI,
                                 ZG01_METER_MIC_OUT_LO);
    v.pc_in = zg01_meter_level(buf, ZG01_METER_PC_IN_HI,
                               ZG01_METER_PC_IN_LO);
    v.stream_mix = zg01_meter_level(buf, ZG01_METER_STREAM_MIX_HI,
                                    ZG01_METER_STREAM_MIX_LO);
    *out = v;
    return 0;
}

#endif /* ZG01_METERS_H */
