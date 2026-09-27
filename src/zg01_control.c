/*
 * Yamaha ZG01 USB Audio Driver - Control Interface
 *
 * EP0 vendor request 7 is the device handshake.  Every mixer setting
 * (mic DSP, EQ, ...) travels on interface 4 as 512-byte bulk frames
 * (host -> EP 0x03, device -> EP 0x83); see docs/MIC_CONTROL_PROTOCOL.md.
 *
 * This file implements that transport and the mic controls (GATE, COMP,
 * EQ and LIMITER) as ALSA kcontrols.
 */

#include <linux/bitops.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/usb.h>
#include <sound/control.h>
#include <sound/tlv.h>

#include "zg01.h"
#include "zg01_control.h"

int zg01_init_control(struct zg01_dev *dev)
{
    int ret;
    unsigned char *buf;

    if (!dev || !dev->udev || !dev->interface)
        return -ENODEV;

    buf = kmalloc(256, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    /* Device initialization sequence based on USB capture. */
    ret = usb_control_msg(dev->udev,
                          usb_rcvctrlpipe(dev->udev, 0),
                          7,      /* bRequest */
                          0xc0,   /* bmRequestType: vendor, device-to-host */
                          0x0000, /* wValue */
                          0,      /* wIndex */
                          buf, 3, /* expect 3 bytes response (80bb00) */
                          1000);
    if (ret < 0) {
        dev_err(&dev->interface->dev,
                "ZG01 initialization request failed: %d\n", ret);
        kfree(buf);
        return ret;
    }

    if (ret == 3) {
        if (buf[0] == 0x80 && buf[1] == 0xbb && buf[2] == 0x00) {
            dev_info(&dev->interface->dev,
                     "ZG01 initialization successful\n");
        } else {
            dev_warn(&dev->interface->dev,
                     "unexpected ZG01 init response %02x%02x%02x\n",
                     buf[0], buf[1], buf[2]);
        }
    } else {
        dev_dbg(&dev->interface->dev, "short response (%d bytes)\n", ret);
    }

    kfree(buf);
    return 0;
}

/*
 * Frame helpers.  A frame is 128 four-byte words; the first byte of a
 * word is a type (0x04 data, 0x05/0x07 terminator, 0x00 padding).
 */
static void zg01_put_word(u8 *buf, unsigned int index, u8 a, u8 b, u8 c)
{
    u8 *w = buf + index * 4;

    w[0] = 0x04;
    w[1] = a;
    w[2] = b;
    w[3] = c;
}

static int zg01_bulk_out(struct zg01_dev *dev, const u8 *buf, unsigned int len)
{
    int actual = 0;
    int ret;

    mutex_lock(&dev->param_mutex);
    ret = usb_bulk_msg(dev->udev,
                       usb_sndbulkpipe(dev->udev, ZG01_PARAM_EP_OUT),
                       (void *)buf, len, &actual, ZG01_PARAM_TIMEOUT);
    mutex_unlock(&dev->param_mutex);

    if (ret)
        return ret;
    if (actual != len)
        return -EIO;
    return 0;
}

/* The app polls the channel; a keepalive precedes each write. */
static int zg01_param_keepalive(struct zg01_dev *dev)
{
    unsigned char *buf;
    int ret;

    buf = kzalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    zg01_put_word(buf, 0, 0xf0, 0x43, 0x10);
    zg01_put_word(buf, 1, 0x3e, 0x14, 0x00);
    buf[8] = 0x07;
    buf[9] = 0x04;
    buf[10] = 0x00;
    buf[11] = 0xf7;

    ret = zg01_bulk_out(dev, buf, ZG01_PARAM_FRAME);
    kfree(buf);
    return ret;
}

/*
 * Write one parameter.  Layout (docs/MIC_CONTROL_PROTOCOL.md):
 *
 *   04 f0 43 10 | 04 3e 14 01 | 04 01 00 <flag> | 04 <id> 00 00
 *   | 04 00 00 00 | 04 <v2> <v1> <v0> | 05 f7 00 00 | 00...
 *
 * The terminator word's first byte is 0x05; a 0x04 there makes the
 * firmware ignore the whole frame.
 */
int zg01_param_write(struct zg01_dev *dev, u8 id, u8 flag, u32 value)
{
    unsigned char *buf;
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;

    ret = zg01_param_keepalive(dev);
    if (ret)
        return ret;

    buf = kzalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    zg01_put_word(buf, 0, 0xf0, 0x43, 0x10);
    zg01_put_word(buf, 1, 0x3e, 0x14, 0x01);
    zg01_put_word(buf, 2, 0x01, 0x00, flag);
    zg01_put_word(buf, 3, id, 0x00, 0x00);
    zg01_put_word(buf, 4, 0x00, 0x00, 0x00);
    zg01_put_word(buf, 5, (value >> 16) & 0xff,
                  (value >> 8) & 0xff, value & 0xff);
    buf[24] = 0x05;
    buf[25] = 0xf7;
    buf[26] = 0x00;
    buf[27] = 0x00;

    ret = zg01_bulk_out(dev, buf, ZG01_PARAM_FRAME);
    kfree(buf);

    if (ret)
        dev_dbg(&dev->interface->dev, "param 0x%02x write failed: %d\n",
                id, ret);
    return ret;
}

/*
 * Persist the current settings to the device's non-volatile memory
 * (`Save to ZG01`):
 *
 *   04 f0 43 30 | 04 3e 14 03 | 04 02 01 00 | 07 00 01 f7
 */
int zg01_param_save(struct zg01_dev *dev)
{
    unsigned char *buf;
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;

    ret = zg01_param_keepalive(dev);
    if (ret)
        return ret;

    buf = kzalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    zg01_put_word(buf, 0, 0xf0, 0x43, 0x30);
    zg01_put_word(buf, 1, 0x3e, 0x14, 0x03);
    zg01_put_word(buf, 2, 0x02, 0x01, 0x00);
    buf[12] = 0x07;
    buf[13] = 0x00;
    buf[14] = 0x01;
    buf[15] = 0xf7;

    ret = zg01_bulk_out(dev, buf, ZG01_PARAM_FRAME);
    kfree(buf);

    if (ret)
        dev_dbg(&dev->interface->dev, "save failed: %d\n", ret);
    return ret;
}

/*
 * Recall the persisted settings (`RESET` / reload):
 *
 *   04 f0 43 30 | 04 3e 14 03 | 04 02 04 00 | 07 00 01 f7
 *
 * Same shape as save, with code 0x04 instead of 0x01.
 */
int zg01_param_reset(struct zg01_dev *dev)
{
    unsigned char *buf;
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;

    ret = zg01_param_keepalive(dev);
    if (ret)
        return ret;

    buf = kzalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    zg01_put_word(buf, 0, 0xf0, 0x43, 0x30);
    zg01_put_word(buf, 1, 0x3e, 0x14, 0x03);
    zg01_put_word(buf, 2, 0x02, 0x04, 0x00);
    buf[12] = 0x07;
    buf[13] = 0x00;
    buf[14] = 0x01;
    buf[15] = 0xf7;

    ret = zg01_bulk_out(dev, buf, ZG01_PARAM_FRAME);
    kfree(buf);

    if (ret)
        dev_dbg(&dev->interface->dev, "reset failed: %d\n", ret);
    return ret;
}

/*
 * Read a parameter through the EP 0x83 flag-0x02 space.  This returns
 * the value for EQ-space ids; mic-space ids (GATE/COMP/LIMITER) answer
 * with a fixed status instead, and their values come from the device
 * state dump, which is not decoded yet.
 */
static int zg01_param_parse(const u8 *buf, unsigned int len, u8 id,
                            u32 *value)
{
    const u8 prefix[16] = {
        0x04, 0xf0, 0x43, 0x10,
        0x04, 0x3e, 0x14, 0x01,
        0x04, 0x01, 0x00, 0x02,
        0x04, id, 0x00, 0x00,
    };
    unsigned int i;

    for (i = 0; i + 28 <= len; i++) {
        if (memcmp(buf + i, prefix, sizeof(prefix)) != 0)
            continue;
        if (buf[i + 16] != 0x04 || buf[i + 24] != 0x05 ||
            buf[i + 25] != 0xf7)
            continue;
        *value = ((u32)buf[i + 21] << 16) |
                 ((u32)buf[i + 22] << 8) |
                 (u32)buf[i + 23];
        return 0;
    }

    return -ENOENT;
}

int zg01_param_read(struct zg01_dev *dev, u8 id, u32 *value)
{
    unsigned char *req;
    unsigned char *buf;
    int ret = -ETIMEDOUT;
    int actual = 0;
    unsigned int attempt;

    if (!dev || !dev->udev || !value)
        return -ENODEV;

    req = kzalloc(20, GFP_KERNEL);
    buf = kmalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!req || !buf) {
        ret = -ENOMEM;
        goto out;
    }

    zg01_put_word(req, 0, 0xf0, 0x43, 0x30);
    zg01_put_word(req, 1, 0x3e, 0x14, 0x01);
    zg01_put_word(req, 2, 0x01, 0x00, 0x02);
    zg01_put_word(req, 3, id, 0x00, 0x00);
    req[16] = 0x06;
    req[17] = 0x00;
    req[18] = 0xf7;
    req[19] = 0x00;

    mutex_lock(&dev->param_mutex);

    ret = usb_bulk_msg(dev->udev,
                       usb_sndbulkpipe(dev->udev, ZG01_PARAM_EP_OUT),
                       req, 20, &actual, ZG01_PARAM_TIMEOUT);
    if (ret)
        goto out_unlock;

    ret = -ETIMEDOUT;
    for (attempt = 0; attempt < 20; attempt++) {
        actual = 0;
        if (usb_bulk_msg(dev->udev,
                         usb_rcvbulkpipe(dev->udev, ZG01_PARAM_EP_IN),
                         buf, ZG01_PARAM_FRAME, &actual, 100))
            continue;
        if (actual > 0 &&
            zg01_param_parse(buf, actual, id, value) == 0) {
            ret = 0;
            break;
        }
    }

out_unlock:
    mutex_unlock(&dev->param_mutex);
out:
    kfree(req);
    kfree(buf);
    return ret;
}

/*
 * Value conversions between the user-facing kcontrol units and the raw
 * device parameter.  See docs/MIC_CONTROL_PROTOCOL.md.
 */
enum zg01_conv {
    CONV_ID,       /* raw == user (switches, 0..100 levels, shapes)  */
    CONV_GAIN,     /* user in 0.1 dB, raw = 308 * (user + 180) / 180  */
    CONV_FREQ,     /* user in Hz, raw piecewise (f / 2f / 2f+32768)   */
    CONV_Q,        /* user in 0.01 Q, raw = 12 * log2(Q / 0.5)        */
};

/* log2(x) in 1/256 units, for x >= 1. */
static unsigned int zg01_log2_fp(unsigned int x)
{
    unsigned int e = fls(x) - 1;
    unsigned long long m = (unsigned long long)x << (31 - e);
    unsigned int frac =
        (unsigned int)(((m - 0x80000000ULL) << 8) >> 31);

    return e * 256 + frac;
}

/* 2^(v / 256) * 256. */
static unsigned int zg01_exp2_fp(unsigned int v)
{
    unsigned int i = v >> 8;
    unsigned int f = v & 0xff;
    unsigned int base = 256 + (f * 177) / 256;

    return base << i;
}

static u32 zg01_q100_to_raw(unsigned int q100)
{
    unsigned int l = zg01_log2_fp(q100);
    unsigned int l50 = zg01_log2_fp(50);

    return (12 * (l - l50)) / 256;
}

static unsigned int zg01_raw_to_q100(u32 raw)
{
    unsigned int v = (raw * 256) / 12;

    return (50 * zg01_exp2_fp(v)) >> 8;
}

/* Measured Hz -> raw points (docs/MIC_CONTROL_PROTOCOL.md). */
static const struct { u32 hz; u32 raw; } zg01_freq_table[] = {
    { 20, 20 }, { 50, 50 }, { 100, 100 }, { 200, 328 }, { 500, 884 },
    { 1000, 1896 }, { 2000, 3920 }, { 5000, 9992 }, { 10000, 19984 },
    { 15000, 29976 }, { 16383, 32766 }, { 16384, 65536 },
    { 17000, 66664 }, { 20000, 72736 },
};

static u32 zg01_freq_to_raw(long hz)
{
    unsigned int i;

    if (hz <= (long)zg01_freq_table[0].hz)
        return zg01_freq_table[0].raw;

    for (i = 1; i < ARRAY_SIZE(zg01_freq_table); i++) {
        u32 h0 = zg01_freq_table[i - 1].hz;
        u32 h1 = zg01_freq_table[i].hz;

        if (hz <= (long)h1) {
            u32 r0 = zg01_freq_table[i - 1].raw;
            u32 r1 = zg01_freq_table[i].raw;

            return r0 + (u32)(((u64)(hz - h0) * (r1 - r0)) / (h1 - h0));
        }
    }

    return zg01_freq_table[ARRAY_SIZE(zg01_freq_table) - 1].raw;
}

static long zg01_raw_to_freq(u32 raw)
{
    unsigned int i;

    if (raw <= zg01_freq_table[0].raw)
        return zg01_freq_table[0].hz;

    for (i = 1; i < ARRAY_SIZE(zg01_freq_table); i++) {
        u32 r0 = zg01_freq_table[i - 1].raw;
        u32 r1 = zg01_freq_table[i].raw;

        if (raw <= r1) {
            u32 h0 = zg01_freq_table[i - 1].hz;
            u32 h1 = zg01_freq_table[i].hz;

            return h0 + (long)(((u64)(raw - r0) * (h1 - h0)) / (r1 - r0));
        }
    }

    return zg01_freq_table[ARRAY_SIZE(zg01_freq_table) - 1].hz;
}

static u32 zg01_user_to_dev(int conv, long user)
{
    switch (conv) {
    case CONV_GAIN:
        return (u32)((308 * (user + 180) + 90) / 180);
    case CONV_FREQ:
        return zg01_freq_to_raw(user);
    case CONV_Q:
        return zg01_q100_to_raw((unsigned int)user);
    default:
        return (u32)user;
    }
}

static long zg01_dev_to_user(int conv, u32 dev)
{
    switch (conv) {
    case CONV_GAIN:
        return (180L * dev + 154) / 308 - 180;
    case CONV_FREQ:
        return zg01_raw_to_freq(dev);
    case CONV_Q:
        return zg01_raw_to_q100(dev);
    default:
        return dev;
    }
}

struct zg01_param_ctl {
    const char *name;
    u8 id;
    u8 flag;
    int type;
    long min;
    long max;
    int conv;
    u32 def;       /* raw default used to seed the cache */
};

static const struct zg01_param_ctl zg01_param_ctls[] = {
    { "Gate Capture Switch", ZG01_PARAM_GATE_ENABLE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "Gate", ZG01_PARAM_GATE_VALUE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 0 },
    { "Compressor Capture Switch", ZG01_PARAM_COMP_ENABLE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "Compressor", ZG01_PARAM_COMP_VALUE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 0 },
    { "Limiter Capture Switch", ZG01_PARAM_LIMITER_ENABLE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "Limiter", ZG01_PARAM_LIMITER_VALUE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 0 },

    { "EQ Capture Switch", ZG01_PARAM_EQ_ENABLE, ZG01_PARAM_FLAG_MIC,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "EQ Low Shape", ZG01_PARAM_LOW_SHAPE, ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 1, CONV_ID, 0 },
    { "EQ High Shape", ZG01_PARAM_HIGH_SHAPE, ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 1, CONV_ID, 0 },

    { "EQ Band 1 Gain", ZG01_PARAM_EQ_GAIN(0), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, -180, 180, CONV_GAIN, 308 },
    { "EQ Band 2 Gain", ZG01_PARAM_EQ_GAIN(1), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, -180, 180, CONV_GAIN, 308 },
    { "EQ Band 3 Gain", ZG01_PARAM_EQ_GAIN(2), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, -180, 180, CONV_GAIN, 308 },
    { "EQ Band 4 Gain", ZG01_PARAM_EQ_GAIN(3), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, -180, 180, CONV_GAIN, 308 },

    { "EQ Band 1 Frequency", ZG01_PARAM_EQ_FREQ(0), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 1000, CONV_FREQ, 80 },
    { "EQ Band 2 Frequency", ZG01_PARAM_EQ_FREQ(1), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 20000, CONV_FREQ, 260 },
    { "EQ Band 3 Frequency", ZG01_PARAM_EQ_FREQ(2), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 20000, CONV_FREQ, 6248 },
    { "EQ Band 4 Frequency", ZG01_PARAM_EQ_FREQ(3), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 500, 20000, CONV_FREQ, 14988 },

    { "EQ Band 1 Q", ZG01_PARAM_EQ_Q(0), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 50, 800, CONV_Q, 12 },
    { "EQ Band 2 Q", ZG01_PARAM_EQ_Q(1), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 50, 800, CONV_Q, 12 },
    { "EQ Band 3 Q", ZG01_PARAM_EQ_Q(2), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 50, 800, CONV_Q, 12 },
    { "EQ Band 4 Q", ZG01_PARAM_EQ_Q(3), ZG01_PARAM_FLAG_EQ,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 50, 800, CONV_Q, 12 },
};

static const struct zg01_param_ctl *zg01_ctl_of(struct snd_kcontrol *kcontrol)
{
    return (const struct zg01_param_ctl *)kcontrol->private_value;
}

static int zg01_ctl_info(struct snd_kcontrol *kcontrol,
                         struct snd_ctl_elem_info *uinfo)
{
    const struct zg01_param_ctl *c = zg01_ctl_of(kcontrol);

    uinfo->type = c->type;
    uinfo->count = 1;
    uinfo->value.integer.min = c->min;
    uinfo->value.integer.max = c->max;
    return 0;
}

/*
 * The getters return the driver cache: the per-id read reply is a
 * status, not the value, and the device state dump that carries real
 * values is not decoded yet.  The cache reflects the last write.
 */
static int zg01_ctl_get(struct snd_kcontrol *kcontrol,
                        struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);
    const struct zg01_param_ctl *c = zg01_ctl_of(kcontrol);

    ucontrol->value.integer.value[0] =
        zg01_dev_to_user(c->conv, dev->param_cache[c->id]);
    return 0;
}

static int zg01_ctl_put(struct snd_kcontrol *kcontrol,
                        struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);
    const struct zg01_param_ctl *c = zg01_ctl_of(kcontrol);
    long user = ucontrol->value.integer.value[0];
    u32 raw;
    int ret;

    if (user < c->min || user > c->max)
        return -EINVAL;

    raw = zg01_user_to_dev(c->conv, user);
    if (dev->param_cache[c->id] == raw)
        return 0;

    ret = zg01_param_write(dev, c->id, c->flag, raw);
    if (ret)
        return ret;

    dev->param_cache[c->id] = raw;
    return 1;
}

/* Write-only action control: writing 1 persists the current settings. */
static int zg01_save_info(struct snd_kcontrol *kcontrol,
                          struct snd_ctl_elem_info *uinfo)
{
    uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
    uinfo->count = 1;
    uinfo->value.integer.min = 0;
    uinfo->value.integer.max = 1;
    return 0;
}

static int zg01_save_get(struct snd_kcontrol *kcontrol,
                         struct snd_ctl_elem_value *ucontrol)
{
    ucontrol->value.integer.value[0] = 0;
    return 0;
}

static int zg01_save_put(struct snd_kcontrol *kcontrol,
                         struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);

    if (!ucontrol->value.integer.value[0])
        return 0;

    return zg01_param_save(dev);
}

/* Write-only action control: writing 1 reloads the persisted settings. */
static int zg01_reset_info(struct snd_kcontrol *kcontrol,
                           struct snd_ctl_elem_info *uinfo)
{
    uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
    uinfo->count = 1;
    uinfo->value.integer.min = 0;
    uinfo->value.integer.max = 1;
    return 0;
}

static int zg01_reset_get(struct snd_kcontrol *kcontrol,
                          struct snd_ctl_elem_value *ucontrol)
{
    ucontrol->value.integer.value[0] = 0;
    return 0;
}

static int zg01_reset_put(struct snd_kcontrol *kcontrol,
                          struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);

    if (!ucontrol->value.integer.value[0])
        return 0;

    return zg01_param_reset(dev);
}

int zg01_create_controls(struct zg01_dev *dev)
{
    unsigned int i;

    if (!dev || !dev->card)
        return -ENODEV;

    for (i = 0; i < ARRAY_SIZE(zg01_param_ctls); i++) {
        const struct zg01_param_ctl *c = &zg01_param_ctls[i];
        struct snd_kcontrol_new tmpl = {
            .iface = SNDRV_CTL_ELEM_IFACE_MIXER,
            .name = c->name,
            .info = zg01_ctl_info,
            .get = zg01_ctl_get,
            .put = zg01_ctl_put,
            .private_value = (unsigned long)c,
        };
        struct snd_kcontrol *kctl;
        int ret;

        dev->param_cache[c->id] = c->def;

        kctl = snd_ctl_new1(&tmpl, dev);
        if (!kctl)
            return -ENOMEM;
        ret = snd_ctl_add(dev->card, kctl);
        if (ret)
            return ret;
    }

    {
        struct snd_kcontrol_new tmpl = {
            .iface = SNDRV_CTL_ELEM_IFACE_MIXER,
            .name = "Save to ZG01",
            .info = zg01_save_info,
            .get = zg01_save_get,
            .put = zg01_save_put,
        };
        struct snd_kcontrol *kctl = snd_ctl_new1(&tmpl, dev);
        int ret;

        if (!kctl)
            return -ENOMEM;
        ret = snd_ctl_add(dev->card, kctl);
        if (ret)
            return ret;
    }

    {
        struct snd_kcontrol_new tmpl = {
            .iface = SNDRV_CTL_ELEM_IFACE_MIXER,
            .name = "Reset to ZG01",
            .info = zg01_reset_info,
            .get = zg01_reset_get,
            .put = zg01_reset_put,
        };
        struct snd_kcontrol *kctl = snd_ctl_new1(&tmpl, dev);
        int ret;

        if (!kctl)
            return -ENOMEM;
        ret = snd_ctl_add(dev->card, kctl);
        if (ret)
            return ret;
    }

    return 0;
}
