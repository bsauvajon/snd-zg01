/*
 * Yamaha ZG01 USB Audio Driver - Control Interface
 *
 * EP0 vendor request 7 is the device handshake.  Every mixer setting
 * (mic DSP, EQ, ...) travels on interface 4 as 512-byte bulk frames
 * (host -> EP 0x03, device -> EP 0x83).  Those frames are USB-MIDI
 * event packets carrying Yamaha system exclusive messages
 * (F0 43 10 3E 14 ... F7), so values are sent as big-endian base-128
 * digits, not as raw bytes; see docs/MIC_CONTROL_PROTOCOL.md and the
 * value codec in zg01_codec.h.
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
#include "zg01_codec.h"
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
 * Frame helpers.  A frame is 128 four-byte USB-MIDI event packets; the
 * first byte of a packet is its code index number (CIN): 0x04 while a
 * system exclusive message continues, 0x05/0x06/0x07 when it ends and
 * the packet carries 1, 2 or 3 data bytes.  0x00 marks padding.
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
 * The two bytes the frame carries are the base-128 digits of the address
 * (docs/DEVICE_PARAMETER_MAP.md): page 0 offsets stay below 0x80 so their
 * high digit is 0x00, page 1 offsets land on 0x02 because 0x100 = 2 * 128.
 */
#define ZG01_ADDR_FLAG(addr)  ((u8)((addr) / 128))
#define ZG01_ADDR_ID(addr)    ((u8)((addr) % 128))

/*
 * Write one parameter.  Layout (docs/MIC_CONTROL_PROTOCOL.md):
 *
 *   04 f0 43 10 | 04 3e 14 01 | 04 01 00 <flag> | 04 <id> 00 00
 *   | 04 00 00 00 | 04 <d0> <d1> <d2> | 05 f7 00 00 | 00...
 *
 * <flag> <id> are the address digits and <d0> <d1> <d2> the big-endian
 * base-128 digits of the value (see zg01_codec.h): 1 kHz is written as
 * 00 07 68, not as 0x07 0x68 of a hexadecimal 16-bit value.  The
 * terminator packet's CIN is 0x05 (the sys exclusive message ends with
 * one byte); a 0x04 there makes the firmware ignore the whole frame.
 */
int zg01_param_write(struct zg01_dev *dev, u16 addr, u32 value)
{
    unsigned char digits[ZG01_VALUE_DIGITS];
    unsigned char *buf;
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;

    if (value > ZG01_VALUE_MAX)
        return -ERANGE;

    ret = zg01_param_keepalive(dev);
    if (ret)
        return ret;

    buf = kzalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!buf)
        return -ENOMEM;

    zg01_value_encode(value, digits);

    zg01_put_word(buf, 0, 0xf0, 0x43, 0x10);
    zg01_put_word(buf, 1, 0x3e, 0x14, 0x01);
    zg01_put_word(buf, 2, 0x01, 0x00, ZG01_ADDR_FLAG(addr));
    zg01_put_word(buf, 3, ZG01_ADDR_ID(addr), 0x00, 0x00);
    zg01_put_word(buf, 4, 0x00, 0x00, 0x00);
    zg01_put_word(buf, 5, digits[0], digits[1], digits[2]);
    buf[24] = 0x05;
    buf[25] = 0xf7;
    buf[26] = 0x00;
    buf[27] = 0x00;

    ret = zg01_bulk_out(dev, buf, ZG01_PARAM_FRAME);
    kfree(buf);

    if (ret)
        dev_dbg(&dev->interface->dev, "param 0x%03x write failed: %d\n",
                addr, ret);
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
 * Read a parameter back through EP 0x83.  The device only answers
 * meaningfully for page 1 addresses (what the application polls there
 * are the dirty_* flags, see docs/DEVICE_PARAMETER_MAP.md); a page 0
 * address comes back as a fixed status, so the mic values are taken
 * from the driver cache instead.  Reply payloads are base-128 digit
 * sequences like the writes.
 */
static int zg01_param_parse(const u8 *buf, unsigned int len, u16 addr,
                            u32 *value)
{
    const u8 prefix[16] = {
        0x04, 0xf0, 0x43, 0x10,
        0x04, 0x3e, 0x14, 0x01,
        0x04, 0x01, 0x00, ZG01_ADDR_FLAG(addr),
        0x04, ZG01_ADDR_ID(addr), 0x00, 0x00,
    };
    unsigned int i;

    for (i = 0; i + 28 <= len; i++) {
        if (memcmp(buf + i, prefix, sizeof(prefix)) != 0)
            continue;
        if (buf[i + 16] != 0x04 || buf[i + 24] != 0x05 ||
            buf[i + 25] != 0xf7)
            continue;
        *value = (u32)zg01_value_decode(buf + i + 21);
        return 0;
    }

    return -ENOENT;
}

int zg01_param_read(struct zg01_dev *dev, u16 addr, u32 *value)
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
    zg01_put_word(req, 2, 0x01, 0x00, ZG01_ADDR_FLAG(addr));
    zg01_put_word(req, 3, ZG01_ADDR_ID(addr), 0x00, 0x00);
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
            zg01_param_parse(buf, actual, addr, value) == 0) {
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
 * Value conversions between the user-facing kcontrol units and the
 * device parameter.  See docs/MIC_CONTROL_PROTOCOL.md and zg01_codec.h.
 */
enum zg01_conv {
    CONV_ID,       /* raw == user (switches, 0..100 mixer levels)    */
    CONV_GAIN,     /* user in 0.1 dB, device = 180 + user            */
    CONV_FREQ,     /* user in Hz, device == user                     */
    CONV_Q,        /* user in 0.01 Q, device = 12 * log2(Q / 0.5)    */
};

static u32 zg01_user_to_dev(int conv, long user)
{
    switch (conv) {
    case CONV_GAIN:
        return (u32)zg01_gain_user_to_dev(user);
    case CONV_FREQ:
        return (u32)zg01_freq_user_to_dev(user);
    case CONV_Q:
        return (u32)zg01_q_user_to_dev(user);
    default:
        return (u32)user;
    }
}

static long zg01_dev_to_user(int conv, u32 dev)
{
    switch (conv) {
    case CONV_GAIN:
        return zg01_gain_dev_to_user(dev);
    case CONV_FREQ:
        return zg01_freq_dev_to_user(dev);
    case CONV_Q:
        return zg01_q_dev_to_user(dev);
    default:
        return dev;
    }
}

struct zg01_param_ctl {
    const char *name;
    u16 addr;      /* device address: page * 256 + offset                  */
    int type;
    long min;
    long max;
    int conv;
    u32 def;       /* raw device default used to seed the cache            */
};

/*
 * The cache is seeded with the device defaults (docs/VENDOR_PACKAGE.md):
 * GATE, COMP and LIMITER levels default to 50, their switches to off
 * except the limiter switch, the microphone EQ to the flat vendor state
 * (0 dB gains, Q 1.00, shelf shapes on) and the sub-parameters of each
 * dynamics block to the vendor model's own defaults.
 *
 * A control exposes the device steps of the address it names
 * (CONV_ID) unless its encoding is *known*: EQ gain in 0.1 dB around
 * 0 dB = 180, frequency in Hz, Q in 0.01 with the vendor's 0.5 floor
 * (see zg01_codec.h and docs/DEVICE_PARAMETER_MAP.md).  The physical unit
 * of the threshold, time and out-gain steps is not established, so
 * naming them in dB or ms would be a guess: the vendor model's bounds
 * are published as they are and a capture can pin each one later.
 */
static const struct zg01_param_ctl zg01_param_ctls[] = {
    { "P48V Capture Switch", ZG01_PARAM_P48V_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    /* The device parameter is mic_mute_onf: 1 mutes the microphone. */
    { "Mute Capture Switch", ZG01_PARAM_MUTE_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },

    { "Gate Capture Switch", ZG01_PARAM_GATE_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "Gate", ZG01_PARAM_GATE_VALUE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 50 },
    { "Gate Threshold", ZG01_PARAM_GATE_THRESHOLD,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 1, 73, CONV_ID, 23 },
    { "Gate Range", ZG01_PARAM_GATE_RANGE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 73, CONV_ID, 17 },
    { "Gate Attack", ZG01_PARAM_GATE_ATTACK,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 57, 283, CONV_ID, 237 },
    { "Gate Hold", ZG01_PARAM_GATE_HOLD,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 213, CONV_ID, 101 },
    { "Gate Decay", ZG01_PARAM_GATE_DECAY,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 24, 300, CONV_ID, 188 },

    { "Compressor Capture Switch", ZG01_PARAM_COMP_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "Compressor", ZG01_PARAM_COMP_VALUE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 50 },
    { "Compressor Threshold", ZG01_PARAM_COMP_THRESHOLD,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 19, 73, CONV_ID, 52 },
    { "Compressor Ratio", ZG01_PARAM_COMP_RATIO,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 120, CONV_ID, 14 },
    { "Compressor Attack", ZG01_PARAM_COMP_ATTACK,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 57, 283, CONV_ID, 237 },
    { "Compressor Release", ZG01_PARAM_COMP_RELEASE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 24, 300, CONV_ID, 228 },
    { "Compressor Knee", ZG01_PARAM_COMP_KNEE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 2, CONV_ID, 1 },
    { "Compressor Out Gain", ZG01_PARAM_COMP_OUTGAIN,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 180, CONV_ID, 50 },

    { "Limiter Capture Switch", ZG01_PARAM_LIMITER_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 1 },
    { "Limiter", ZG01_PARAM_LIMITER_VALUE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 100, CONV_ID, 50 },
    { "Limiter Threshold", ZG01_PARAM_LIMITER_THRESHOLD,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 19, 73, CONV_ID, 65 },
    { "Limiter Attack", ZG01_PARAM_LIMITER_ATTACK,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 57, 283, CONV_ID, 237 },
    { "Limiter Release", ZG01_PARAM_LIMITER_RELEASE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 24, 300, CONV_ID, 228 },
    { "Limiter Knee", ZG01_PARAM_LIMITER_KNEE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 2, CONV_ID, 1 },
    { "Limiter Out Gain", ZG01_PARAM_LIMITER_OUTGAIN,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 180, CONV_ID, 0 },

    { "EQ Capture Switch", ZG01_PARAM_EQ_ENABLE,
      SNDRV_CTL_ELEM_TYPE_BOOLEAN, 0, 1, CONV_ID, 0 },
    { "EQ Low Shape", ZG01_PARAM_LOW_SHAPE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 1, CONV_ID, 1 },
    { "EQ High Shape", ZG01_PARAM_HIGH_SHAPE,
      SNDRV_CTL_ELEM_TYPE_INTEGER, 0, 1, CONV_ID, 1 },

    /*
     * EQ defaults are the vendor ones (docs/VENDOR_PACKAGE.md): gains at
     * 0 dB (device 180), band frequencies at 100 Hz / 500 Hz / 2.5 kHz /
     * 10 kHz, Q at 1.00 (device 12).
     */
    { "EQ Band 1 Gain", ZG01_PARAM_EQ_GAIN(0),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_GAIN_USER_MIN, ZG01_GAIN_USER_MAX,
      CONV_GAIN, ZG01_GAIN_DEV_ZERO },
    { "EQ Band 2 Gain", ZG01_PARAM_EQ_GAIN(1),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_GAIN_USER_MIN, ZG01_GAIN_USER_MAX,
      CONV_GAIN, ZG01_GAIN_DEV_ZERO },
    { "EQ Band 3 Gain", ZG01_PARAM_EQ_GAIN(2),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_GAIN_USER_MIN, ZG01_GAIN_USER_MAX,
      CONV_GAIN, ZG01_GAIN_DEV_ZERO },
    { "EQ Band 4 Gain", ZG01_PARAM_EQ_GAIN(3),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_GAIN_USER_MIN, ZG01_GAIN_USER_MAX,
      CONV_GAIN, ZG01_GAIN_DEV_ZERO },

    { "EQ Band 1 Frequency", ZG01_PARAM_EQ_FREQ(0),
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 1000, CONV_FREQ, 100 },
    { "EQ Band 2 Frequency", ZG01_PARAM_EQ_FREQ(1),
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 20000, CONV_FREQ, 500 },
    { "EQ Band 3 Frequency", ZG01_PARAM_EQ_FREQ(2),
      SNDRV_CTL_ELEM_TYPE_INTEGER, 20, 20000, CONV_FREQ, 2500 },
    { "EQ Band 4 Frequency", ZG01_PARAM_EQ_FREQ(3),
      SNDRV_CTL_ELEM_TYPE_INTEGER, 500, 20000, CONV_FREQ, 10000 },

    { "EQ Band 1 Q", ZG01_PARAM_EQ_Q(0),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_Q_USER_MIN, ZG01_Q_USER_MAX,
      CONV_Q, 12 },
    { "EQ Band 2 Q", ZG01_PARAM_EQ_Q(1),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_Q_USER_MIN, ZG01_Q_USER_MAX,
      CONV_Q, 12 },
    { "EQ Band 3 Q", ZG01_PARAM_EQ_Q(2),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_Q_USER_MIN, ZG01_Q_USER_MAX,
      CONV_Q, 12 },
    { "EQ Band 4 Q", ZG01_PARAM_EQ_Q(3),
      SNDRV_CTL_ELEM_TYPE_INTEGER, ZG01_Q_USER_MIN, ZG01_Q_USER_MAX,
      CONV_Q, 12 },
};

static const struct zg01_param_ctl *zg01_ctl_of(struct snd_kcontrol *kcontrol)
{
    return (const struct zg01_param_ctl *)kcontrol->private_value;
}

/* One cache slot per address, so the two pages cannot alias. */
static u32 *zg01_cache(struct zg01_dev *dev, u16 addr)
{
    return &dev->param_cache[addr / ZG01_PARAM_PAGE_SIZE]
                            [addr % ZG01_PARAM_PAGE_SIZE];
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
 * The getters return the driver cache: the device exposes no usable
 * per-parameter read (its replies are statuses) and the state dump is
 * not decoded, so the cache is the source of truth.  It is seeded with
 * the built-in defaults at load and updated on every successful write;
 * settings changed outside the driver are not reflected.
 */
static int zg01_ctl_get(struct snd_kcontrol *kcontrol,
                        struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);
    const struct zg01_param_ctl *c = zg01_ctl_of(kcontrol);

    ucontrol->value.integer.value[0] =
        zg01_dev_to_user(c->conv, *zg01_cache(dev, c->addr));
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

    /*
     * Always write.  The cache is seeded with the model defaults, not with
     * what the device holds (its persisted settings can come from the
     * Windows application or an earlier session), and nothing can refresh
     * it: page 0 answers reads with a status rather than a value, and a
     * Reset to ZG01 reloads the device without touching the cache.  A
     * "the cached value is already this" shortcut therefore swallowed real
     * writes - asking for P48V off while the cache said off did nothing
     * even though phantom power was on.
     */
    ret = zg01_param_write(dev, c->addr, raw);
    if (ret)
        return ret;

    *zg01_cache(dev, c->addr) = raw;
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

    /* The cache is indexed by address: every address must fit a page. */
    BUILD_BUG_ON(ZG01_PARAM_EQ_Q(3) >= ZG01_PARAM_CACHE);

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

        *zg01_cache(dev, c->addr) = c->def;

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
