/*
 * Yamaha ZG01 USB Audio Driver - Control Interface
 *
 * EP0 vendor request 7 is the device handshake.  Every mixer setting
 * (mic DSP, EQ, ...) travels on interface 4 as 512-byte bulk frames
 * (host -> EP 0x03, device -> EP 0x83); see docs/MIC_CONTROL_PROTOCOL.md.
 *
 * This file implements that transport and the first consumer: the mic
 * LIMITER block (enable + level).
 */

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

/*
 * Write one parameter.  Layout (docs/MIC_CONTROL_PROTOCOL.md):
 *
 *   04 f0 43 10 | 04 3e 14 01 | 04 01 00 <flag> | 04 <id> 00 00
 *   | 04 00 00 00 | 04 <v2> <v1> <v0> | 05 f7 00 00 | 00...
 *
 * The firmware ignores parameter writes when the host has not been
 * polling the channel, so a keepalive frame is sent first.
 */
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
    /* Terminator word: type 0x05, not a 0x04 data word. */
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
 * A read response echoes the request and carries the value:
 *
 *   04 f0 43 10 | 04 3e 14 01 | 04 01 00 02 | 04 <id> 00 00
 *   | 04 00 00 00 | 04 <v2> <v1> <v0> | 05 f7 00 00
 *
 * The 0x83 stream also carries 192-byte level-meter frames, so scan the
 * buffer for the matching response rather than assuming an offset.
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

    /* The response is interleaved with the meter stream. */
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

/* --- LIMITER kcontrols --- */

static int limiter_switch_info(struct snd_kcontrol *kcontrol,
                               struct snd_ctl_elem_info *uinfo)
{
    uinfo->type = SNDRV_CTL_ELEM_TYPE_BOOLEAN;
    uinfo->count = 1;
    uinfo->value.integer.min = 0;
    uinfo->value.integer.max = 1;
    return 0;
}

/*
 * The kcontrol getters return the driver's cached value.  Reading a mic
 * parameter back from the device is not done through the EP 0x83
 * type-30 response (that echoes a status, not the value); the app reads
 * values from the full state dump the device pushes after a type-20
 * request.  Decoding that dump is a separate task, so the cache is the
 * source of truth until then (it reflects the last value written).
 */

static int limiter_switch_get(struct snd_kcontrol *kcontrol,
                              struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);

    ucontrol->value.integer.value[0] = dev->limiter_enabled;
    return 0;
}

static int limiter_switch_put(struct snd_kcontrol *kcontrol,
                              struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);
    bool on = ucontrol->value.integer.value[0] != 0;
    int ret;

    if (dev->limiter_enabled == on)
        return 0;

    ret = zg01_param_write(dev, ZG01_PARAM_LIMITER_ENABLE,
                           ZG01_PARAM_FLAG_MIC, on ? 1 : 0);
    if (ret)
        return ret;

    dev->limiter_enabled = on;
    return 1;
}

static int limiter_value_info(struct snd_kcontrol *kcontrol,
                              struct snd_ctl_elem_info *uinfo)
{
    uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
    uinfo->count = 1;
    uinfo->value.integer.min = 0;
    uinfo->value.integer.max = 100;
    return 0;
}

static int limiter_value_get(struct snd_kcontrol *kcontrol,
                             struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);

    ucontrol->value.integer.value[0] = dev->limiter_value;
    return 0;
}

static int limiter_value_put(struct snd_kcontrol *kcontrol,
                             struct snd_ctl_elem_value *ucontrol)
{
    struct zg01_dev *dev = snd_kcontrol_chip(kcontrol);
    unsigned int value = ucontrol->value.integer.value[0];
    int ret;

    if (value > 100 || dev->limiter_value == value)
        return 0;

    ret = zg01_param_write(dev, ZG01_PARAM_LIMITER_VALUE,
                           ZG01_PARAM_FLAG_MIC, value);
    if (ret)
        return ret;

    dev->limiter_value = value;
    return 1;
}

static const struct snd_kcontrol_new zg01_limiter_controls[] = {
    {
        .iface = SNDRV_CTL_ELEM_IFACE_MIXER,
        .name = "Limiter Capture Switch",
        .info = limiter_switch_info,
        .get = limiter_switch_get,
        .put = limiter_switch_put,
    },
    {
        .iface = SNDRV_CTL_ELEM_IFACE_MIXER,
        .name = "Limiter",
        .info = limiter_value_info,
        .get = limiter_value_get,
        .put = limiter_value_put,
    },
};

int zg01_create_controls(struct zg01_dev *dev)
{
    unsigned int i;

    if (!dev || !dev->card)
        return -ENODEV;

    for (i = 0; i < ARRAY_SIZE(zg01_limiter_controls); i++) {
        struct snd_kcontrol *kctl =
            snd_ctl_new1(&zg01_limiter_controls[i], dev);
        int ret;

        if (!kctl)
            return -ENOMEM;
        ret = snd_ctl_add(dev->card, kctl);
        if (ret)
            return ret;
    }

    return 0;
}
