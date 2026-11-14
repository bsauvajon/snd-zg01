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

#define ZG01_METER_GROUP_CHANNELS 8
#define ZG01_METER_CHANNELS       34

#define ZG01_METER_BASE_MIC_IN    0
#define ZG01_METER_BASE_MIC_OUT   2
#define ZG01_METER_BASE_PC_IN     4
#define ZG01_METER_BASE_HDMI_IN   20
#define ZG01_METER_BASE_GAME_MIX  36
#define ZG01_METER_BASE_STREAM_MIX 52

#define ZG01_METER_ZERO_CODE      16384
#define ZG01_METER_CODES_PER_DB   10

struct zg01_meter_values {
    int mic_in;
    int mic_out;
    int pc_in[ZG01_METER_GROUP_CHANNELS];
    int hdmi_in[ZG01_METER_GROUP_CHANNELS];
    int game_mix[ZG01_METER_GROUP_CHANNELS];
    int stream_mix[ZG01_METER_GROUP_CHANNELS];
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

/* Map a SysEx payload index to its raw frame offset. */
static inline unsigned int zg01_meter_raw(unsigned int payload)
{
    return 4 * (payload / 3) + 1 + payload % 3;
}

/*
 * One channel current value. A channel has a current and a peak field; the
 * current is the even field of the pair.
 */
static inline int zg01_meter_channel(const unsigned char *buf,
                                     unsigned int base, unsigned int channel)
{
    unsigned int field = base + 2 * channel;
    unsigned int payload = 7 + 2 * field;

    return zg01_meter_level(buf, zg01_meter_raw(payload),
                            zg01_meter_raw(payload + 1));
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

    v.mic_in = zg01_meter_channel(buf, ZG01_METER_BASE_MIC_IN, 0);
    v.mic_out = zg01_meter_channel(buf, ZG01_METER_BASE_MIC_OUT, 0);
    for (i = 0; i < ZG01_METER_GROUP_CHANNELS; i++) {
        v.pc_in[i] = zg01_meter_channel(buf, ZG01_METER_BASE_PC_IN, i);
        v.hdmi_in[i] = zg01_meter_channel(buf, ZG01_METER_BASE_HDMI_IN, i);
        v.game_mix[i] = zg01_meter_channel(buf, ZG01_METER_BASE_GAME_MIX, i);
        v.stream_mix[i] =
            zg01_meter_channel(buf, ZG01_METER_BASE_STREAM_MIX, i);
    }
    *out = v;
    return 0;
}

#endif /* ZG01_METERS_H */
