#include <linux/slab.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include "zg01.h"
#include <linux/workqueue.h>
#include <linux/jiffies.h>

/* Minimal delayed work handler placeholder. Detailed deferred start logic
 * can be implemented later; this avoids undefined symbol at build time. */
static void zg01_pcm_start_work(struct work_struct *work)
{
    pr_info("zg01_pcm: delayed start work executed (placeholder)\n");
}

/* Audio streaming parameters based on USB capture analysis */
/* Each URB contains 32 ISO descriptors of 240 bytes = 7680 bytes USB data */
/* Each ISO descriptor contains 6 audio frames = 192 frames per URB */
/* At S32_LE stereo: 192 frames × 8 bytes = 1536 bytes PCM data per URB (4ms @ 48kHz) */
#define PCM_BUFFER_BYTES_MAX_GAME   (1536 * 32)       /* 48KB buffer (128ms) */
#define PCM_BUFFER_BYTES_MIN_GAME   (1536 * 2)        /* 3KB min buffer (8ms) */
#define PCM_PERIOD_BYTES_MIN_GAME   (192 * 8)         /* 1536 bytes = 192 frames minimum */
#define PCM_PERIOD_BYTES_MAX_GAME   (1536 * 8)        /* 12KB period max (32ms) */

#define PCM_BUFFER_BYTES_MAX_VOICE  (48 * 32 * 64)   /* ~98KB max buffer */
#define PCM_BUFFER_BYTES_MIN_VOICE  (48 * 8)         /* ~384B min buffer */
#define PCM_PERIOD_BYTES_MIN_VOICE  (48 * 1)         /* 48 bytes min (1 uFrame) */
#define PCM_PERIOD_BYTES_MAX_VOICE  (48 * 16)        /* 768 bytes max (16 uFrames) */

/* USB endpoints from capture analysis */
#define ZG01_EP_AUDIO_OUT  0x01   /* Audio output endpoint */
#define ZG01_EP_AUDIO_IN   0x81   /* Audio input endpoint */

/* Forward declarations */
static int zg01_start_streaming(struct zg01_dev *dev, struct snd_pcm_substream *substream);
static void zg01_stop_streaming(struct zg01_dev *dev);


static int zg01_pcm_open(struct snd_pcm_substream *substream)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    struct snd_pcm_runtime *runtime = substream->runtime;
    int ret = 0;
    unsigned long now = jiffies;
    
    if (!dev) {
        pr_err("zg01_pcm: No device structure available\n");
        return -ENODEV;
    }

    if (!runtime) {
        pr_err("zg01_pcm: No runtime available for substream\n");
        return -EINVAL;
    }
    
    /* Validate stream direction matches channel capability */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        /* Game channel - playback only */
        if (substream->stream != SNDRV_PCM_STREAM_PLAYBACK) {
            pr_err("zg01_pcm: Game channel only supports playback\n");
            return -ENODEV;
        }
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        /* Voice In channel - capture only */
        if (substream->stream != SNDRV_PCM_STREAM_CAPTURE) {
            pr_err("zg01_pcm: Voice In channel only supports capture\n");
            return -ENODEV;
        }
    } else {
        /* Voice Out channel - playback only */
        if (substream->stream != SNDRV_PCM_STREAM_PLAYBACK) {
            pr_err("zg01_pcm: Voice Out channel only supports playback\n");
            return -ENODEV;
        }
    }
    
    /* Protect concurrent opens */
    mutex_lock(&dev->pcm_mutex);
    
    /* Rate limiting for audio system probing - reduce log spam */
    bool is_rapid_probe = false;
    if (time_before(now, dev->last_open_jiffies + msecs_to_jiffies(1000))) { /* 1 second window */
        dev->open_count++;
        if (dev->open_count > 2) { /* More aggressive - suppress after 2nd open */
            is_rapid_probe = true;
        }
    } else {
        dev->open_count = 1;
    }
    dev->last_open_jiffies = now;
    
    runtime->hw.info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_INTERLEAVED |
                       SNDRV_PCM_INFO_BLOCK_TRANSFER;

    runtime->hw.formats = SNDRV_PCM_FMTBIT_S32_LE;  /* 32-bit samples (device uses lower 24 bits) */
        /* Default to 48kHz; voice channel may operate at 16kHz on some devices */
        runtime->hw.rates = SNDRV_PCM_RATE_48000;
    runtime->hw.rate_min = 48000;
    runtime->hw.rate_max = 48000;
    runtime->hw.channels_min = 2;
    runtime->hw.channels_max = 2;

    /* Configure buffer sizes based on channel type */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        /* Game channel - Interface 1, Alt 1, EP 0x01 OUT (280 bytes) */
        /* Allow flexible buffer/period sizes aligned to USB packet boundaries */
        runtime->hw.buffer_bytes_max = PCM_BUFFER_BYTES_MAX_GAME;
        runtime->hw.period_bytes_min = PCM_PERIOD_BYTES_MIN_GAME;
        runtime->hw.period_bytes_max = PCM_PERIOD_BYTES_MAX_GAME;
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Opening ZG01 Game channel (Interface 1, Alt 1)\n");
        } else {
            pr_debug("zg01_pcm: Opening ZG01 Game channel (rapid probe #%u)\n", dev->open_count);
        }
        
        /* Verify we have a valid usb_interface and cur_altsetting */
        if (!dev->interface || !dev->interface->cur_altsetting) {
            pr_warn("zg01_pcm: No valid USB interface for Game channel\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* Verify we're on the correct interface */
        int current_interface = dev->interface->cur_altsetting->desc.bInterfaceNumber;
        if (current_interface != 1) {
            pr_warn("zg01_pcm: Game channel requires Interface 1, current is %d\n",
                    current_interface);
            ret = -ENODEV;
            goto unlock;
        }

        /* Ensure we have a valid usb_device before changing interface */
        if (!dev->udev) {
            pr_err("zg01_pcm: No usb_device available to set interface\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* Set Interface 1 to Alt Setting 1 to enable isochronous endpoint 0x01 OUT */
        ret = usb_set_interface(dev->udev, 1, 1);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set Interface 1 Alt 1: %d\n", ret);
            goto unlock;
        }
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Game channel configured Interface 1, Alt 1, EP 0x01 OUT (280 bytes)\n");
        }
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        /* Voice In channel - Interface 2, Alt 1, EP 0x81 IN (124 bytes) - CAPTURE ONLY */
        /* Allow flexible buffer/period sizes aligned to USB packet boundaries */
        runtime->hw.buffer_bytes_max = PCM_BUFFER_BYTES_MAX_VOICE;
        runtime->hw.period_bytes_min = PCM_PERIOD_BYTES_MIN_VOICE;
        runtime->hw.period_bytes_max = PCM_PERIOD_BYTES_MAX_VOICE;
        /* Voice channel can be 16kHz or 48kHz depending on device firmware; allow both */
        runtime->hw.rates = SNDRV_PCM_RATE_48000 | SNDRV_PCM_RATE_16000;
        runtime->hw.rate_min = 16000;
        runtime->hw.rate_max = 48000;
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Opening ZG01 Voice In channel (Interface 2, Alt 1)\n");
        } else {
            pr_debug("zg01_pcm: Opening ZG01 Voice In channel (rapid probe #%u)\n", dev->open_count);
        }
        
        /* Verify we have a valid usb_interface and cur_altsetting */
        if (!dev->interface || !dev->interface->cur_altsetting) {
            pr_warn("zg01_pcm: No valid USB interface for Voice channel\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* Verify we're on the correct interface */
        int current_interface = dev->interface->cur_altsetting->desc.bInterfaceNumber;
        if (current_interface != 2) {
            pr_warn("zg01_pcm: Voice In channel requires Interface 2, current is %d\n",
                    current_interface);
            ret = -ENODEV;
            goto unlock;
        }

        /* Ensure we have a valid usb_device before changing interface */
        if (!dev->udev) {
            pr_err("zg01_pcm: No usb_device available to set interface\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* Set Interface 2 to Alt Setting 1 to enable isochronous endpoint 0x81 IN */
        ret = usb_set_interface(dev->udev, 2, 1);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set Interface 2 Alt 1: %d\n", ret);
            goto unlock;
        }
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Voice In channel configured Interface 2, Alt 1, EP 0x81 IN (124 bytes)\n");
        }
    } else {
        /* Voice Out channel - Interface 1, Alt 1, EP 0x01 OUT - PLAYBACK ONLY */
        /* Based on USB captures, voice output uses Interface 1 Alt 1 but WITHOUT sample rate control */
        runtime->hw.buffer_bytes_max = PCM_BUFFER_BYTES_MAX_GAME;
        runtime->hw.period_bytes_min = PCM_PERIOD_BYTES_MIN_GAME;
        runtime->hw.period_bytes_max = PCM_PERIOD_BYTES_MAX_GAME;
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Opening ZG01 Voice Out channel (Interface 1, Alt 1)\n");
        } else {
            pr_debug("zg01_pcm: Opening ZG01 Voice Out channel (rapid probe #%u)\n", dev->open_count);
        }
        
        /* Verify we have a valid usb_interface and cur_altsetting */
        if (!dev->interface || !dev->interface->cur_altsetting) {
            pr_warn("zg01_pcm: No valid USB interface for Voice Out channel\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* Voice Out uses Interface 1 like Game, but different configuration */
        int current_interface = dev->interface->cur_altsetting->desc.bInterfaceNumber;
        if (current_interface != 1) {
            pr_warn("zg01_pcm: Voice Out channel requires Interface 1, current is %d\n",
                    current_interface);
            ret = -ENODEV;
            goto unlock;
        }

        /* Ensure we have a valid usb_device before changing interface */
        if (!dev->udev) {
            pr_err("zg01_pcm: No usb_device available to set interface\n");
            ret = -ENODEV;
            goto unlock;
        }

        /* According to USB capture: Interface 2 Alt 0, then Interface 1 Alt 1, then Interface 2 Alt 1 */
        ret = usb_set_interface(dev->udev, 2, 0);
        if (ret < 0) {
            pr_warn("zg01_pcm: Failed to set Interface 2 Alt 0 for Voice Out: %d\n", ret);
        }
        
        ret = usb_set_interface(dev->udev, 1, 1);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set Interface 1 Alt 1 for Voice Out: %d\n", ret);
            goto unlock;
        }
        
        ret = usb_set_interface(dev->udev, 2, 1);
        if (ret < 0) {
            pr_warn("zg01_pcm: Failed to set Interface 2 Alt 1 for Voice Out: %d\n", ret);
        }
        
        if (!is_rapid_probe) {
            pr_info("zg01_pcm: Voice Out channel configured Interface 1, Alt 1, EP 0x01 OUT (voice mode)\n");
        }
    }
    
    runtime->hw.periods_min = 2;
    runtime->hw.periods_max = 64; /* Allow more flexibility for PipeWire */
    
    /* Add constraints to ensure USB packet alignment */
    if (dev->channel_type == CHANNEL_TYPE_GAME || dev->channel_type == CHANNEL_TYPE_VOICE_OUT) {
        /* Game and Voice Out channels: period size must be multiple of 1536 bytes (192 frames = 1 URB) */
        ret = snd_pcm_hw_constraint_step(runtime, 0, SNDRV_PCM_HW_PARAM_PERIOD_BYTES, 1536);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set period step constraint: %d\n", ret);
            goto unlock;
        }
        /* Buffer size should also align to period boundaries */
        ret = snd_pcm_hw_constraint_step(runtime, 0, SNDRV_PCM_HW_PARAM_BUFFER_BYTES, 96);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set buffer step constraint: %d\n", ret);
            goto unlock;
        }
    } else {
        /* Voice In channel: period size must be multiple of 48 bytes (6 frames) */
        ret = snd_pcm_hw_constraint_step(runtime, 0, SNDRV_PCM_HW_PARAM_PERIOD_BYTES, 48);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set period step constraint: %d\n", ret);
            goto unlock;
        }
        ret = snd_pcm_hw_constraint_step(runtime, 0, SNDRV_PCM_HW_PARAM_BUFFER_BYTES, 48);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set buffer step constraint: %d\n", ret);
            goto unlock;
        }
    }
    
    /* Set up channel state */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        if (dev->game_channel_active) {
            pr_warn("zg01_pcm: Game channel already active\n");
            ret = -EBUSY;
            goto unlock;
        }
        dev->game_channel_active = true;
        dev->substream_game = substream;
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        if (dev->voice_channel_active) {
            pr_warn("zg01_pcm: Voice In channel already active\n");
            ret = -EBUSY;
            goto unlock;
        }
        dev->voice_channel_active = true;
        dev->substream_voice = substream;
    } else {
        if (dev->voice_out_channel_active) {
            pr_warn("zg01_pcm: Voice Out channel already active\n");
            ret = -EBUSY;
            goto unlock;
        }
        dev->voice_out_channel_active = true;
        dev->substream_voice_out = substream; /* Voice Out has its own substream pointer */
    }

unlock:
    mutex_unlock(&dev->pcm_mutex);
    return ret;
}

static int zg01_pcm_close(struct snd_pcm_substream *substream)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    
    if (!dev) {
        return 0;
    }
    
    /* Stop continuous streaming */
    zg01_stop_streaming(dev);
    
    mutex_lock(&dev->pcm_mutex);
    
    /* Clear channel state */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        dev->game_channel_active = false;
        /* Don't reset game_initialized - keep device initialized across opens */
        dev->substream_game = NULL;
        /* Reduce logging for rapid probe cycles */
        if (dev->open_count <= 2) {
            pr_info("zg01_pcm: Game channel closed\n");
        } else {
            pr_debug("zg01_pcm: Game channel closed (rapid probe)\n");
        }
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        dev->voice_channel_active = false;
        /* Don't reset voice_initialized - keep device initialized across opens */
        dev->substream_voice = NULL;
        /* Reduce logging for rapid probe cycles */
        if (dev->open_count <= 2) {
            pr_info("zg01_pcm: Voice In channel closed\n");
        } else {
            pr_debug("zg01_pcm: Voice In channel closed (rapid probe)\n");
        }
    } else {
        dev->voice_out_channel_active = false;
        /* Don't reset voice_out_initialized - keep device initialized across opens */
        dev->substream_voice_out = NULL; /* Voice Out uses dedicated substream pointer */
        /* Reduce logging for rapid probe cycles */
        if (dev->open_count <= 2) {
            pr_info("zg01_pcm: Voice Out channel closed\n");
        } else {
            pr_debug("zg01_pcm: Voice Out channel closed (rapid probe)\n");
        }
    }
    
    mutex_unlock(&dev->pcm_mutex);
    return 0;
}

static int zg01_pcm_hw_params(struct snd_pcm_substream *substream,
                              struct snd_pcm_hw_params *hw_params)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    unsigned int rate, channels, format;
    
    if (!dev || !hw_params) {
        pr_err("zg01_pcm: Invalid parameters in hw_params\n");
        return -EINVAL;
    }
    
    rate = params_rate(hw_params);
    channels = params_channels(hw_params);
    format = params_format(hw_params);
    
    pr_info("zg01_pcm: hw_params - rate:%u, channels:%u, format:%u, period_size:%u, periods:%u, buffer_size:%u\n",
            rate, channels, format, 
            params_period_size(hw_params),
            params_periods(hw_params),
            params_buffer_size(hw_params));
    
    /* Validate parameters: device supports 48000 and may report 16000 on some firmwares */
    if (rate != 48000 && rate != 16000) {
        pr_warn("zg01_pcm: Unsupported sample rate: %u\n", rate);
        return -EINVAL;
    }

    /* Attempt to read device-reported sampling frequency (GET_CUR) and enforce it.
     * If we cannot read the device, fall back to accepting the requested rate.
     */
    if (dev && dev->udev) {
        unsigned char cur_rate_buf[4];
        int rc = usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                                 0x01, /* GET_CUR */
                                 USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                                 0x0100, /* SAMPLING_FREQ_CONTROL */
                                 0x0100, /* Index: Entity 1, Intf 0 */
                                 cur_rate_buf, 4, 500);
        if (rc == 4) {
            unsigned int dev_rate = cur_rate_buf[0] | (cur_rate_buf[1] << 8) |
                                    (cur_rate_buf[2] << 16) | (cur_rate_buf[3] << 24);
            pr_info("zg01_pcm: Device-reported sampling rate via GET_CUR: %u\n", dev_rate);
            dev->current_rate = (int)dev_rate;
            if ((unsigned int)dev->current_rate != rate) {
                pr_warn("zg01_pcm: Requested rate %u does not match device rate %u; rejecting hw_params\n",
                        rate, dev->current_rate);
                return -EINVAL;
            }
        } else {
            pr_warn("zg01_pcm: Could not read device sampling rate (rc=%d); accepting requested rate %u\n", rc, rate);
            dev->current_rate = rate;
        }
    } else {
        /* No usb_device available; accept requested rate and store it */
        dev->current_rate = rate;
    }
    dev->rate_residual = 0;
    
    if (channels != 2) {
        pr_warn("zg01_pcm: Unsupported channel count: %u\n", channels);
        return -EINVAL;
    }
    
    if (format != SNDRV_PCM_FORMAT_S32_LE) {
        pr_warn("zg01_pcm: Unsupported format: %u\n", format);
        return -EINVAL;
    }
    
    /* Reduce logging for rapid probe cycles - hw_params is often called multiple times */
    if (dev->open_count <= 1) { /* Even more aggressive - only log first open */
        pr_info("zg01_pcm: hw_params - rate:%u, channels:%u, format:%u\n",
                rate, channels, format);
    } else {
        pr_debug("zg01_pcm: hw_params - rate:%u, channels:%u, format:%u (rapid probe)\n", 
                rate, channels, format);
    }
    
    return 0;
}

static int zg01_pcm_hw_free(struct snd_pcm_substream *substream)
{
    return 0;
}

/* Helper to set sample rate via UAC2 Clock Source Control + Extended Vendor Magic */
static int zg01_set_rate(struct zg01_dev *dev, int rate)
{
    unsigned char *data;
    unsigned char *large_data;
    int ret = 0;
    
    if (!dev || !dev->udev) {
        pr_err("zg01_pcm: zg01_set_rate called with invalid dev or missing udev\n");
        return -ENODEV;
    }

    /* Allocate DMA-capable buffers - USB control messages need DMA-safe memory */
    data = kmalloc(4, GFP_KERNEL);
    large_data = kmalloc(72, GFP_KERNEL);
    if (!data || !large_data) {
        pr_err("zg01_pcm: Failed to allocate control message buffers\n");
        ret = -ENOMEM;
        goto cleanup;
    }
    
    pr_info("zg01_pcm: Starting extended Magic Sequence for %d Hz\n", rate);
    
    /* 1. Early Vendor Reads (Initialization/State discovery) */
    /* Many Yamaha devices require these reads to move out of standby */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x07, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 3, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x04, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 1, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0a, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 4, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0c, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x8000, 0x0000, large_data, 72, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0c, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 72, 1000);

    /* 2. Set Interfaces 1 and 2 to Alt 0 */
    pr_info("zg01_pcm: Resetting interfaces to Alt 0\n");
    if (dev->udev) {
        usb_set_interface(dev->udev, 1, 0);
        usb_set_interface(dev->udev, 2, 0);
    } else {
        pr_warn("zg01_pcm: Skipping interface reset; missing udev\n");
    }

    /* 3. Set UAC2 Rate on Clock Source 1 */
    data[0] = rate & 0xff;
    data[1] = (rate >> 8) & 0xff;
    data[2] = (rate >> 16) & 0xff;
    data[3] = (rate >> 24) & 0xff;
    
    /* Perform SET_CUR and then verify by reading GET_CUR. Retry if necessary. */
    {
        int attempts = 3;
        int attempt;
        int verify_ret = -EIO;
        for (attempt = 1; attempt <= attempts; attempt++) {
            ret = usb_control_msg(dev->udev, usb_sndctrlpipe(dev->udev, 0),
                                  0x01, /* SET_CUR (class) */
                                  USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                                  0x0100, /* SAMPLING_FREQ_CONTROL */
                                  0x0100, /* Index: Entity 1, Intf 0 */
                                  data, 4, 1000);

            if (ret < 0) {
                pr_err("zg01_pcm: Attempt %d: Failed to set UAC2 rate: %d\n", attempt, ret);
            } else {
                pr_info("zg01_pcm: Attempt %d: UAC2 Set Rate sent\n", attempt);
            }

            /* Read back the current sampling frequency (GET_CUR) */
            verify_ret = usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                                         0x01, /* GET_CUR (class) */
                                         USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                                         0x0100, /* SAMPLING_FREQ_CONTROL */
                                         0x0100, /* Index: Entity 1, Intf 0 */
                                         large_data, 4, 1000);

            if (verify_ret == 4) {
                unsigned int ret_rate = large_data[0] | (large_data[1] << 8) |
                                         (large_data[2] << 16) | (large_data[3] << 24);
                pr_info("zg01_pcm: GET_CUR reported rate: %u (requested %d)\n", ret_rate, rate);
                /* Treat the device-reported rate as authoritative */
                dev->current_rate = (int)ret_rate;
                if ((int)ret_rate == rate) {
                    pr_info("zg01_pcm: Verified device rate %u Hz\n", ret_rate);
                } else {
                    pr_warn("zg01_pcm: Device reported different rate (%u) than requested (%d); using device rate\n",
                            ret_rate, rate);
                }
                ret = 0;
                break;
            } else {
                pr_warn("zg01_pcm: Failed to read back sampling freq (rc=%d)\n", verify_ret);
                ret = (verify_ret < 0) ? verify_ret : -EIO;
            }

            /* Try some vendor handshakes if first attempt failed */
            if (attempt < attempts) {
                pr_info("zg01_pcm: Retrying rate set (attempt %d/%d)\n", attempt + 1, attempts);
                /* Small pause to let device settle */
                msleep(150);
            }
        }
    }
    
    /* 4. Complete Handshake/Commit */
    pr_info("zg01_pcm: Finalizing handshake (Vendor 0xC0/0x41)\n");
    
    /* 0xC0 Request 2 Value 2 Index 0 Len 1 */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x02, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0002, 0x0000, large_data, 1, 1000);
                    
    /* 0xC0 Request 2 Value 1 Index 0 Len 1 */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x02, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0001, 0x0000, large_data, 1, 1000);
                    
    /* 0xC0 Request 8 Value 0 Index 0 Len 1 */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x08, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 1, 1000);
                    
    /* 0x41 Request 0 Value 0 Index 0 Len 0 */
    usb_control_msg(dev->udev, usb_sndctrlpipe(dev->udev, 0),
                    0x00, /* Request 0 */
                    USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_INTERFACE,
                    0x0000, 0x0000, NULL, 0, 1000);

    /* 5. Restore Streaming Interfaces (Alt 1) */
    pr_info("zg01_pcm: Activating interfaces (Alt 1)\n");
    usb_set_interface(dev->udev, 1, 1);
    usb_set_interface(dev->udev, 2, 1);
    
    /* Give device time to stabilize after configuration */
    msleep(200);  /* Increase delay - localhost may need more time */
    pr_info("zg01_pcm: Magic Sequence complete, device should be ready\n");
    
cleanup:
    kfree(large_data);
    kfree(data);
    return ret;
}

/* Removed unused zg01_check_clock_validity function - was returning -11 on localhost
 * and device works without it. Can be re-added if needed for clock validation. */

static int zg01_pcm_prepare(struct snd_pcm_substream *substream)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    int ret = 0;
    int interface_num;
    bool is_first_prepare = false;
    
    /* Determine interface number based on channel type */
    if (dev->channel_type == CHANNEL_TYPE_GAME || dev->channel_type == CHANNEL_TYPE_VOICE_OUT) {
        interface_num = 1; /* Game and Voice Out use Interface 1 */
    } else {
        interface_num = 2; /* Voice In uses Interface 2 */
    }
    
    pr_info("zg01_pcm: prepare called - channel_type=%d, game_init=%d, voice_init=%d, voice_out_init=%d\n",
            dev->channel_type, dev->game_initialized, dev->voice_initialized, dev->voice_out_initialized);
    
    /* Check if this is the first prepare (device needs initialization) */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        /* Game channel */
        if (!dev->game_initialized) {
            is_first_prepare = true;
            dev->game_initialized = true;
        }
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        /* Voice In channel */
        if (!dev->voice_initialized) {
            is_first_prepare = true;
            dev->voice_initialized = true;
        }
    } else {
        /* Voice Out channel */
        if (!dev->voice_out_initialized) {
            is_first_prepare = true;
            dev->voice_out_initialized = true;
        }
    }
    
    /* Only do full initialization on first prepare */
    if (is_first_prepare) {
        const char *channel_name;
        if (dev->channel_type == CHANNEL_TYPE_GAME) channel_name = "Game";
        else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) channel_name = "Voice In";
        else channel_name = "Voice Out";
        
        pr_info("zg01_pcm: First prepare for %s channel - running initialization\n", channel_name);

        /* Voice Out does NOT send SET_CUR control message according to USB capture */
        if (dev->channel_type != 2) {
            /* Game and Voice In: Prefer previously negotiated rate if available, otherwise request 48000 */
            if (dev->current_rate == 16000 || dev->current_rate == 48000) {
                pr_info("zg01_pcm: Using existing current_rate=%d\n", dev->current_rate);
                /* Attempt to set device to current_rate to make sure device matches runtime */
                if (zg01_set_rate(dev, dev->current_rate) < 0) {
                    pr_warn("zg01_pcm: zg01_set_rate failed for current_rate=%d, falling back to 48000\n", dev->current_rate);
                    zg01_set_rate(dev, 48000);
                    dev->current_rate = 48000;
                }
            } else {
                /* Set Rate with Magic Sequence, default to 48000 */
                if (zg01_set_rate(dev, 48000) < 0) {
                    pr_warn("zg01_pcm: zg01_set_rate(48000) failed during initialization\n");
                } else {
                    dev->current_rate = 48000;
                }
            }
        } else {
            /* Voice Out: Skip sample rate setting, just configure interfaces */
            pr_info("zg01_pcm: Voice Out - skipping sample rate control (not needed per USB capture)\n");
            /* Set the interface sequence as shown in USB capture: Interface 2 Alt 0, Interface 1 Alt 1, Interface 2 Alt 1 */
            usb_set_interface(dev->udev, 2, 0);
            usb_set_interface(dev->udev, 1, 1);
            usb_set_interface(dev->udev, 2, 1);
            dev->current_rate = 48000; /* Assume 48kHz */
        }

        /* Skip clock validity check for now - it fails with -11 on localhost */
        /* zg01_check_clock_validity(dev); */
    } else {
        pr_info("zg01_pcm: prepare called - device already initialized, skipping Magic Sequence\n");
    }
    
    /* Restore streaming interface only if not already streaming */
    int active_urbs_count = 0;
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        active_urbs_count = dev->active_urbs_game;
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        active_urbs_count = dev->active_urbs_voice;
    } else {
        active_urbs_count = dev->active_urbs_voice_out;
    }
    
    if (active_urbs_count == 0) {
        /* Only set interface if not already streaming - avoid disrupting active URBs */
        pr_debug("zg01_pcm: Switching Interface %d to Alt 1 for streaming\n", interface_num);
        ret = usb_set_interface(dev->udev, interface_num, 1);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to set Interface %d Alt 1: %d\n", interface_num, ret);
            return ret;
        }
    } else {
        pr_debug("zg01_pcm: Streaming already active, skipping interface setup\n");
    }
    
    /* Reset PCM position only if not already streaming */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        if (dev->active_urbs_game == 0) {
            dev->pcm_pos_game = 0;
        }
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        if (dev->active_urbs_voice == 0) {
            dev->pcm_pos_voice = 0;
        }
    } else {
        if (dev->active_urbs_voice_out == 0) {
            dev->pcm_pos_voice_out = 0;
        }
    }
    
    return 0;
}

struct zg01_cleanup_work {
    struct work_struct work;
    struct urb *urb;
    unsigned char *buf;
    dma_addr_t dma;
    struct usb_device *udev;
    int total_size;
    
    /* For multi-URB cleanup */
    struct zg01_dev *dev;
    int channel_type; /* 0=game, 1=voice_in, 2=voice_out */
};

/* Multi-URB cleanup function that can safely sleep */
static void zg01_cleanup_multi_urb_work_fn(struct work_struct *work)
{
    struct zg01_cleanup_work *cw = container_of(work, struct zg01_cleanup_work, work);
    struct zg01_dev *dev = cw->dev;
    struct urb **iso_urbs;
    unsigned char **iso_buffers;
    dma_addr_t *iso_dmas;
    int iso_pkts, iso_pkt_size;
    int i;

    if (cw->channel_type == CHANNEL_TYPE_GAME) {
        iso_urbs = dev->iso_urbs_game;
        iso_buffers = dev->iso_buffers_game;
        iso_dmas = dev->iso_dmas_game;
        iso_pkts = ISO_PKTS_GAME;
        iso_pkt_size = ISO_PKT_SIZE_GAME;
    } else if (cw->channel_type == CHANNEL_TYPE_VOICE_IN) {
        iso_urbs = dev->iso_urbs_voice;
        iso_buffers = dev->iso_buffers_voice;
        iso_dmas = dev->iso_dmas_voice;
        iso_pkts = ISO_PKTS_VOICE;
        iso_pkt_size = ISO_PKT_SIZE_VOICE;
    } else {
        iso_urbs = dev->iso_urbs_voice_out;
        iso_buffers = dev->iso_buffers_voice_out;
        iso_dmas = dev->iso_dmas_voice_out;
        iso_pkts = ISO_PKTS_GAME;
        iso_pkt_size = ISO_PKT_SIZE_GAME;
    }

    /* Kill all URBs (can sleep here) */
    for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
        if (iso_urbs[i]) {
            usb_kill_urb(iso_urbs[i]);
        }
    }

    /* Free all resources */
    for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
        if (iso_buffers[i]) {
            kfree(iso_buffers[i]);
            iso_buffers[i] = NULL;
        }
        if (iso_urbs[i]) {
            usb_free_urb(iso_urbs[i]);
            iso_urbs[i] = NULL;
        }
    }

    /* Clear cleanup flag - new streams can now start */
    if (cw->channel_type == CHANNEL_TYPE_GAME) {
        dev->cleanup_in_progress_game = false;
    } else if (cw->channel_type == CHANNEL_TYPE_VOICE_IN) {
        dev->cleanup_in_progress_voice = false;
    } else {
        dev->cleanup_in_progress_voice_out = false;
    }

    pr_info("zg01_pcm: Multi-URB cleanup completed\n");
    kfree(cw);
}

static void zg01_iso_callback(struct urb *urb)
{
    struct zg01_dev *dev = urb->context;
    struct snd_pcm_substream *substream;
    struct snd_pcm_runtime *runtime;
    unsigned char *pcm_buf;
    unsigned int period_size;
    unsigned long flags;
    unsigned int *pcm_pos;
    int i;
    int resubmit_ret;
    bool is_game_channel = false;
    bool is_voice_out_channel = false;
    bool found_urb = false;

    /* Early exit for shutdown or critical errors */
    if (urb->status == -ESHUTDOWN || urb->status == -ENOENT || urb->status == -ECONNRESET) {
        pr_debug("zg01_pcm: URB stopped: %d\n", urb->status);
        return;
    }

    if (urb->status && urb->status != -EXDEV) {
        pr_warn("zg01_pcm: URB error: %d\n", urb->status);
        /* Still try to resubmit for recoverable errors */
    }

    /* Use spinlock to safely check if this URB is still active */
    spin_lock_irqsave(&dev->lock, flags);
    
    /* Determine which channel this callback is for */
    for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
        if (urb == dev->iso_urbs_game[i]) {
            substream = dev->substream_game;
            pcm_pos = &dev->pcm_pos_game;
            is_game_channel = true;
            found_urb = true;
            break;
        }
        if (urb == dev->iso_urbs_voice[i]) {
            substream = dev->substream_voice;
            pcm_pos = &dev->pcm_pos_voice;
            is_game_channel = false;
            found_urb = true;
            break;
        }
        if (urb == dev->iso_urbs_voice_out[i]) {
            substream = dev->substream_voice_out;
            pcm_pos = &dev->pcm_pos_voice_out;
            is_game_channel = false;
            is_voice_out_channel = true; /* Voice Out playback */
            found_urb = true;
            break;
        }
    }
    
    spin_unlock_irqrestore(&dev->lock, flags);
    
    if (!found_urb) {
        /* This is likely an old URB that was replaced during rapid restart */
        pr_debug("zg01_pcm: Callback for stale URB (stream restarted)\n");
        return;
    }

    /* Validate substream and runtime */
    if (!substream) {
        pr_debug("zg01_pcm: No substream in callback (stream stopped)\n");
        return;
    }

    runtime = substream->runtime;
    if (!runtime) {
        pr_debug("zg01_pcm: No runtime in callback (stream stopped)\n");
        /* Still resubmit URB to keep USB streaming alive */
        goto resubmit;
    }

    /* Check if stream is still active before processing audio data */
    if (runtime->status->state != SNDRV_PCM_STATE_RUNNING) {
        pr_debug("zg01_pcm: Stream not running, state: %d - sending silence\n", runtime->status->state);
        /* Send silence but keep URBs running */
        if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
            /* Fill output buffer with silence */
            for (i = 0; i < urb->number_of_packets; i++) {
                unsigned char *pkt_buf = urb->transfer_buffer + urb->iso_frame_desc[i].offset;
                unsigned int pkt_len = urb->iso_frame_desc[i].length;
                if (pkt_len > 0 && pkt_len <= 8192) {
                    memset(pkt_buf, 0, pkt_len);
                }
            }
        }
        goto resubmit;
    }

    if (!runtime->dma_area) {
        pr_err("zg01_pcm: No DMA area allocated\n");
        goto resubmit;
    }
    
    pcm_buf = runtime->dma_area;
    period_size = runtime->period_size;
    
    /* Process audio data based on stream direction */
    if (urb->status == 0) {
        bool period_elapsed = false;
        unsigned int bytes_per_frame = runtime->frame_bits / 8; /* Should be 8 for S32_LE stereo */
        
    if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
        /* PLAYBACK: Copy audio data FROM PCM buffer TO USB device WITH PADDING */
        unsigned int total_frames_processed = 0; /* Track frames processed in this URB */

        /* For playback, packet sizes are already set during URB initialization */
        /* Process each packet and copy audio data */
        for (i = 0; i < urb->number_of_packets; i++) {
            unsigned char *pkt_buf;
            unsigned int pkt_len = urb->iso_frame_desc[i].length;
            unsigned int pkt_offset = 0;

            if (pkt_len == 0 || pkt_len > 8192) {/* Sanity check */
                continue;
            }

            pkt_buf = urb->transfer_buffer + urb->iso_frame_desc[i].offset;

            /* Each USB packet MUST contain exactly 6 frames (240 bytes) */
            /* Frame format: 8 zeros + L(4) + R(4) + 24 zeros = 40 bytes per frame */
            const unsigned int frames_per_packet = 6;
            
            if (pkt_len == 240) {
                spin_lock_irqsave(&dev->lock, flags);

                /* Get current HW position in frames - use the pcm_pos we already determined */
                unsigned int hw_pos_frames = *pcm_pos;
                
                /* runtime->buffer_size is already in frames, not bytes */
                unsigned int buffer_size_frames = runtime->buffer_size;
                unsigned int frames_copied;

                for (frames_copied = 0; frames_copied < frames_per_packet; frames_copied++) {
                    /* Calculate position in buffer (wrapping at buffer boundary) */
                    unsigned int frame_pos = (hw_pos_frames + total_frames_processed + frames_copied) % buffer_size_frames;
                    unsigned int pcm_frame_offset = frame_pos * bytes_per_frame;
                    unsigned int buffer_size_bytes = buffer_size_frames * bytes_per_frame;
                    
                    int32_t sample_l, sample_r;
                    
                    if (pcm_frame_offset + 8 <= buffer_size_bytes) {
                        memcpy(&sample_l, pcm_buf + pcm_frame_offset, 4);
                        memcpy(&sample_r, pcm_buf + pcm_frame_offset + 4, 4);
                    } else {
                        /* Wrapped read */
                        unsigned int first_part = buffer_size_bytes - pcm_frame_offset;
                        unsigned char tmp_buf[8];
                        memcpy(tmp_buf, pcm_buf + pcm_frame_offset, first_part);
                        memcpy(tmp_buf + first_part, pcm_buf, 8 - first_part);
                        memcpy(&sample_l, tmp_buf, 4);
                        memcpy(&sample_r, tmp_buf + 4, 4);
                    }
                    
                    /* No shift needed - device expects samples in upper bits like ALSA S32_LE */
                    
                    /* Check if the channel is active */
                    bool is_active;
                    if (is_game_channel) {
                        is_active = dev->game_channel_active;
                    } else if (is_voice_out_channel) {
                        is_active = dev->voice_out_channel_active;
                    } else {
                        is_active = dev->voice_channel_active;
                    }

                    /* 40-byte frame format: 8 zeros + L + R + 24 zeros */
                    memset(pkt_buf + pkt_offset, 0, 8);
                    pkt_offset += 8;
                    
                    if (is_active) {
                        /* Copy Left Channel */
                        memcpy(pkt_buf + pkt_offset, &sample_l, 4);
                    } else {
                        memset(pkt_buf + pkt_offset, 0, 4);
                    }
                    pkt_offset += 4;
                    
                    if (is_active) {
                        /* Copy Right Channel */
                         memcpy(pkt_buf + pkt_offset, &sample_r, 4);
                    } else {
                         memset(pkt_buf + pkt_offset, 0, 4);
                    }
                    pkt_offset += 4;
                    
                    /* 24 bytes of zeros after samples */
                    memset(pkt_buf + pkt_offset, 0, 24);
                    pkt_offset += 24;
                }
                
                /* Fill any remaining packet space with silence (shouldn't happen with exact calc) */
                if (pkt_offset < pkt_len) {
                    memset(pkt_buf + pkt_offset, 0, pkt_len - pkt_offset);
                }

                total_frames_processed += frames_copied;

                spin_unlock_irqrestore(&dev->lock, flags);
            }
        }
            
            /* Update global position once per URB for all processed frames */
            if (total_frames_processed > 0) {
                spin_lock_irqsave(&dev->lock, flags);
                *pcm_pos += total_frames_processed;
                if (period_size > 0 && (*pcm_pos % period_size) == 0)
                    period_elapsed = true;
                spin_unlock_irqrestore(&dev->lock, flags);
            }
        } else {
            /* CAPTURE: Copy audio data FROM USB device TO PCM buffer */
            for (i = 0; i < urb->number_of_packets; i++) {
                unsigned char *pkt_buf;
                unsigned int pkt_len;

                pkt_len = urb->iso_frame_desc[i].actual_length;
                if (pkt_len != 108) /* Voice channel expects 108 bytes per packet */
                    continue;

                pkt_buf = urb->transfer_buffer + urb->iso_frame_desc[i].offset;
                
                /* Voice channel packet format (108 bytes):
                 * - Bytes 0-7: Header (counter + size marker)
                 * - Bytes 8-103: 6 frames × 16 bytes each
                 *   Each frame: 4 bytes L + 4 bytes R + 8 bytes padding
                 * - Bytes 104-107: Trailer (counter repeat)
                 */
                {
                    const unsigned int header_size = 8;
                    const unsigned int usb_frame_size = 16;
                    const unsigned int frames_per_packet = 6;
                    unsigned int buffer_bytes = runtime->buffer_size * bytes_per_frame;

                    spin_lock_irqsave(&dev->lock, flags);

                    unsigned int write_frame = (*pcm_pos) % runtime->buffer_size;
                    unsigned int write_byte_pos = write_frame * bytes_per_frame;
                    unsigned int frames_written = 0;

                    for (int f = 0; f < frames_per_packet; f++) {
                        unsigned char *usb_frame = pkt_buf + header_size + (f * usb_frame_size);
                        int32_t sample_l, sample_r;
                        
                        /* Extract samples (already in S32_LE format, no shift needed for voice) */
                        memcpy(&sample_l, usb_frame, 4);      /* Left at offset 0 */
                        memcpy(&sample_r, usb_frame + 4, 4);  /* Right at offset 4 */

                        /* Write to DMA buffer */
                        if (write_byte_pos + bytes_per_frame <= buffer_bytes) {
                            memcpy(pcm_buf + write_byte_pos, &sample_l, 4);
                            memcpy(pcm_buf + write_byte_pos + 4, &sample_r, 4);
                        } else {
                            /* Handle wraparound */
                            unsigned int first_part = buffer_bytes - write_byte_pos;
                            if (first_part >= 4) {
                                memcpy(pcm_buf + write_byte_pos, &sample_l, 4);
                                if (first_part >= 8) {
                                    memcpy(pcm_buf + write_byte_pos + 4, &sample_r, 4);
                                } else {
                                    memcpy(pcm_buf + write_byte_pos + 4, &sample_r, first_part - 4);
                                    memcpy(pcm_buf, ((unsigned char*)&sample_r) + (first_part - 4), 8 - first_part);
                                }
                            } else {
                                memcpy(pcm_buf + write_byte_pos, &sample_l, first_part);
                                memcpy(pcm_buf, ((unsigned char*)&sample_l) + first_part, 4 - first_part);
                                memcpy(pcm_buf + (4 - first_part), &sample_r, 4);
                            }
                        }

                        frames_written++;
                        write_frame = (write_frame + 1) % runtime->buffer_size;
                        write_byte_pos = write_frame * bytes_per_frame;
                    }

                    *pcm_pos += frames_written;
                    if (period_size > 0 && ((*pcm_pos % period_size) == 0))
                        period_elapsed = true;

                    spin_unlock_irqrestore(&dev->lock, flags);
                }
            }
        }
        
        /* Call period_elapsed outside of spinlock */
        if (period_elapsed) {
            snd_pcm_period_elapsed(substream);
        }
    }

resubmit:
    /* Always resubmit URB to keep USB streaming active */
    /* Reset all frame descriptors for next transfer */
    for (i = 0; i < urb->number_of_packets; i++) {
        urb->iso_frame_desc[i].status = 0;
        urb->iso_frame_desc[i].actual_length = 0;
    }
    
    resubmit_ret = usb_submit_urb(urb, GFP_ATOMIC);
    if (resubmit_ret < 0) {
        pr_warn("zg01_pcm: Failed to resubmit URB: %d\n", resubmit_ret);
        /* Only notify ALSA if stream was running */
        if (substream && runtime && runtime->status->state == SNDRV_PCM_STATE_RUNNING) {
            pr_info("zg01_pcm: Stopping stream due to URB resubmission failure\n");
            snd_pcm_stop_xrun(substream);
        }
    }
}

void zg01_pcm_start_work_fn(struct work_struct *work);

/* Helper function to start streaming with multiple URBs */
static int zg01_start_streaming(struct zg01_dev *dev, struct snd_pcm_substream *substream)
{
    int ret = 0;
    int iso_pkts, iso_pkt_size;
    unsigned int endpoint;
    struct urb **iso_urbs;
    unsigned char **iso_buffers;
    dma_addr_t *iso_dmas;
    int *active_urbs;
    int urb_idx, i, j;
    bool is_game_channel = (dev->channel_type == CHANNEL_TYPE_GAME);
    bool is_voice_in_channel = (dev->channel_type == CHANNEL_TYPE_VOICE_IN);

    /* Select parameters based on channel type */
    if (is_game_channel) {
        /* Double-check cleanup is complete */
        if (dev->cleanup_in_progress_game) {
            pr_warn("zg01_pcm: Game cleanup still in progress, aborting start\n");
            return -EBUSY;
        }
        
        iso_pkts = ISO_PKTS_GAME;
        iso_pkt_size = ISO_PKT_SIZE_GAME;
        endpoint = ZG01_EP_GAME_OUT;
        iso_urbs = dev->iso_urbs_game;
        iso_buffers = dev->iso_buffers_game;
        iso_dmas = dev->iso_dmas_game;
        active_urbs = &dev->active_urbs_game;
        dev->substream_game = substream;
        pr_info("zg01_pcm: Starting Game channel (EP 0x%02x, %d URBs, %d bytes each)\n", 
                endpoint, MAX_URBS_PER_CHANNEL, iso_pkt_size);
    } else if (is_voice_in_channel) {
        /* Double-check cleanup is complete */
        if (dev->cleanup_in_progress_voice) {
            pr_warn("zg01_pcm: Voice In cleanup still in progress, aborting start\n");
            return -EBUSY;
        }
        
        iso_pkts = ISO_PKTS_VOICE;
        iso_pkt_size = ISO_PKT_SIZE_VOICE;
        endpoint = ZG01_EP_VOICE_IN;
        iso_urbs = dev->iso_urbs_voice;
        iso_buffers = dev->iso_buffers_voice;
        iso_dmas = dev->iso_dmas_voice;
        active_urbs = &dev->active_urbs_voice;
        dev->substream_voice = substream;
        
        /* Voice In channel only supports capture */
        if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
            pr_warn("zg01_pcm: Voice In channel only supports capture (IN endpoint)\n");
            return -ENODEV;
        }
        pr_info("zg01_pcm: Starting Voice In channel (EP 0x%02x, %d URBs, %d bytes each)\n", 
                endpoint, MAX_URBS_PER_CHANNEL, iso_pkt_size);
    } else {
        /* Voice Out channel - uses same parameters as game */
        /* Double-check cleanup is complete */
        if (dev->cleanup_in_progress_voice_out) {
            pr_warn("zg01_pcm: Voice Out cleanup still in progress, aborting start\n");
            return -EBUSY;
        }
        
        iso_pkts = ISO_PKTS_GAME;
        iso_pkt_size = 240; /* Voice Out uses 240-byte packets */
        endpoint = ZG01_EP_GAME_OUT; /* Same endpoint as game */
        iso_urbs = dev->iso_urbs_voice_out;
        iso_buffers = dev->iso_buffers_voice_out;
        iso_dmas = dev->iso_dmas_voice_out;
        active_urbs = &dev->active_urbs_voice_out;
        dev->substream_voice_out = substream; /* CRITICAL: Voice Out needs its own substream */
        pr_info("zg01_pcm: Starting Voice Out channel (EP 0x%02x, %d URBs, %d bytes each)\n", 
                endpoint, MAX_URBS_PER_CHANNEL, iso_pkt_size);
    }

    /* Check if streaming is already active */
    if (*active_urbs > 0) {
        pr_info("zg01_pcm: Streaming already active (%d URBs), skipping start\n", *active_urbs);
        return 0;  /* Return success since streaming is already running */
    }

    *active_urbs = 0;

    /* Allocate and prepare multiple URBs for smooth streaming */
    for (urb_idx = 0; urb_idx < MAX_URBS_PER_CHANNEL; urb_idx++) {
        /* Allocate URB */
        iso_urbs[urb_idx] = usb_alloc_urb(iso_pkts, GFP_KERNEL);
        if (!iso_urbs[urb_idx]) {
            ret = -ENOMEM;
            goto cleanup_urbs;
        }

        /* Allocate coherent buffer - try GFP_KERNEL first for xHCI compatibility */
        iso_buffers[urb_idx] = kmalloc(iso_pkts * iso_pkt_size, GFP_KERNEL | GFP_DMA);
        if (!iso_buffers[urb_idx]) {
            usb_free_urb(iso_urbs[urb_idx]);
            iso_urbs[urb_idx] = NULL;
            ret = -ENOMEM;
            goto cleanup_urbs;
        }
        /* For kmalloc'd memory, we don't have a separate DMA address */
        iso_dmas[urb_idx] = 0;

        /* Configure URB */
        iso_urbs[urb_idx]->dev = dev->udev;
        if (endpoint & USB_DIR_IN) {
            iso_urbs[urb_idx]->pipe = usb_rcvisocpipe(dev->udev, endpoint & 0x0F);
        } else {
            iso_urbs[urb_idx]->pipe = usb_sndisocpipe(dev->udev, endpoint & 0x0F);
        }
        iso_urbs[urb_idx]->transfer_buffer = iso_buffers[urb_idx];
        iso_urbs[urb_idx]->transfer_buffer_length = iso_pkts * iso_pkt_size;
        iso_urbs[urb_idx]->complete = zg01_iso_callback;
        iso_urbs[urb_idx]->context = dev;
        iso_urbs[urb_idx]->interval = 1;  /* Back to 1ms interval with 1 packet per URB */
        iso_urbs[urb_idx]->start_frame = -1;
        iso_urbs[urb_idx]->number_of_packets = iso_pkts;
        iso_urbs[urb_idx]->transfer_flags = URB_ISO_ASAP;

        /* Setup isochronous frame descriptors */
        for (i = 0; i < iso_pkts; i++) {
            iso_urbs[urb_idx]->iso_frame_desc[i].offset = i * iso_pkt_size;
            iso_urbs[urb_idx]->iso_frame_desc[i].length = iso_pkt_size;
        }

        /* For playback, pre-fill with silence */
        if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
            memset(iso_buffers[urb_idx], 0, iso_pkts * iso_pkt_size);
        }
    }

    /* Submit all URBs */
    for (urb_idx = 0; urb_idx < MAX_URBS_PER_CHANNEL; urb_idx++) {
        ret = usb_submit_urb(iso_urbs[urb_idx], GFP_KERNEL);
        if (ret) {
            pr_err("zg01_pcm: Failed to submit URB %d: %d (EAGAIN=%d, ENODEV=%d, ENOMEM=%d)\n",
                   urb_idx, ret, -EAGAIN, -ENODEV, -ENOMEM);
            pr_err("zg01_pcm: URB details - EP: 0x%02x, interval: %d, num_packets: %d\n",
                   usb_pipeendpoint(iso_urbs[urb_idx]->pipe),
                   iso_urbs[urb_idx]->interval,
                   iso_urbs[urb_idx]->number_of_packets);
            goto cleanup_submitted_urbs;
        }
         /* Log successful submission for debugging (limited info) */
         pr_info("zg01_pcm: Submitted URB %d -> EP: 0x%02x, num_pkts: %d, pkt_size: %d\n",
              urb_idx, usb_pipeendpoint(iso_urbs[urb_idx]->pipe), iso_urbs[urb_idx]->number_of_packets, iso_pkt_size);
        (*active_urbs)++;
    }

    pr_info("zg01_pcm: Successfully started streaming with %d URBs\n", *active_urbs);
    return 0;

cleanup_submitted_urbs:
    /* Kill any already submitted URBs */
    for (j = 0; j < urb_idx; j++) {
        if (iso_urbs[j]) {
            usb_kill_urb(iso_urbs[j]);
        }
    }

cleanup_urbs:
    /* Clean up only the URBs we actually allocated (up to urb_idx) */
    for (j = 0; j <= urb_idx && j < MAX_URBS_PER_CHANNEL; j++) {
        if (iso_buffers[j]) {
            kfree(iso_buffers[j]);
            iso_buffers[j] = NULL;
        }
        if (iso_urbs[j]) {
            usb_free_urb(iso_urbs[j]);
            iso_urbs[j] = NULL;
        }
    }
    *active_urbs = 0;
    return ret;
}

/* Helper function to stop streaming and clean up URBs */
static void zg01_stop_streaming(struct zg01_dev *dev)
{
    struct urb **iso_urbs;
    unsigned char **iso_buffers;
    dma_addr_t *iso_dmas;
    int *active_urbs;
    bool *cleanup_in_progress;
    int i;
    bool is_game_channel = (dev->channel_type == CHANNEL_TYPE_GAME);
    bool is_voice_in_channel = (dev->channel_type == CHANNEL_TYPE_VOICE_IN);
    unsigned long flags;

    if (is_game_channel) {
        iso_urbs = dev->iso_urbs_game;
        iso_buffers = dev->iso_buffers_game;
        iso_dmas = dev->iso_dmas_game;
        active_urbs = &dev->active_urbs_game;
        cleanup_in_progress = &dev->cleanup_in_progress_game;
        pr_info("zg01_pcm: Stopping Game channel\n");
    } else if (is_voice_in_channel) {
        iso_urbs = dev->iso_urbs_voice;
        iso_buffers = dev->iso_buffers_voice;
        iso_dmas = dev->iso_dmas_voice;
        active_urbs = &dev->active_urbs_voice;
        cleanup_in_progress = &dev->cleanup_in_progress_voice;
        pr_info("zg01_pcm: Stopping Voice In channel\n");
    } else {
        iso_urbs = dev->iso_urbs_voice_out;
        iso_buffers = dev->iso_buffers_voice_out;
        iso_dmas = dev->iso_dmas_voice_out;
        active_urbs = &dev->active_urbs_voice_out;
        cleanup_in_progress = &dev->cleanup_in_progress_voice_out;
        pr_info("zg01_pcm: Stopping Voice Out channel\n");
    }

    spin_lock_irqsave(&dev->lock, flags);
    *cleanup_in_progress = true;
    spin_unlock_irqrestore(&dev->lock, flags);

    /* First unlink all URBs (non-blocking) */
    for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
        if (iso_urbs[i]) {
            usb_unlink_urb(iso_urbs[i]);
        }
    }

    /* Create cleanup work for deferred cleanup (can sleep) */
    struct zg01_cleanup_work *cw = kzalloc(sizeof(*cw), GFP_ATOMIC);
    if (cw) {
        INIT_WORK(&cw->work, zg01_cleanup_multi_urb_work_fn);
        cw->dev = dev;
        cw->channel_type = dev->channel_type;
        
        if (!queue_work(system_wq, &cw->work)) {
            pr_warn("zg01_pcm: Failed to queue multi-URB cleanup work\n");
            kfree(cw);
            /* Fallback - just unlink, don't try to kill/free in atomic context */
        }
    }
    
    *active_urbs = 0;
    pr_info("zg01_pcm: URBs unlinked, cleanup deferred\n");
}

static int zg01_pcm_trigger(struct snd_pcm_substream *substream, int cmd)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    int ret = 0;

    if (!dev) {
        pr_err("zg01_pcm: No device structure available in trigger\n");
        return -ENODEV;
    }

    switch (cmd) {
    case SNDRV_PCM_TRIGGER_START:
        /* Start streaming and mark channel as active */
        ret = zg01_start_streaming(dev, substream);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to start streaming in trigger: %d\n", ret);
            return ret;
        }
        
        if (dev->channel_type == CHANNEL_TYPE_GAME) {
            dev->game_channel_active = true;
            pr_info("zg01_pcm: Trigger START - Game channel playing\n");
        } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
            dev->voice_channel_active = true;
            pr_info("zg01_pcm: Trigger START - Voice In channel playing\n");
        } else {
            dev->voice_out_channel_active = true;
            pr_info("zg01_pcm: Trigger START - Voice Out channel playing\n");
        }
        break;

    case SNDRV_PCM_TRIGGER_STOP:
        /* Mark channel as inactive (stream continues with silence) */
        if (dev->channel_type == CHANNEL_TYPE_GAME) {
            dev->game_channel_active = false;
            pr_info("zg01_pcm: Trigger STOP - Game channel muted\n");
        } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
            dev->voice_channel_active = false;
            pr_info("zg01_pcm: Trigger STOP - Voice In channel muted\n");
        } else {
            dev->voice_out_channel_active = false;
            pr_info("zg01_pcm: Trigger STOP - Voice Out channel muted\n");
        }
        /* Don't stop URBs - keep USB streaming active to avoid restart overhead */
        break;

    default:
        return -EINVAL;
    }

    return 0;
}


static snd_pcm_uframes_t zg01_pcm_pointer(struct snd_pcm_substream *substream)
{
    struct zg01_dev *dev = snd_pcm_substream_chip(substream);
    struct snd_pcm_runtime *runtime = substream->runtime;
    unsigned long pos;

    if (!dev) {
        pr_err("zg01_pcm: No device structure available in pointer\n");
        return 0;
    }

    spin_lock(&dev->lock);
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        pos = dev->pcm_pos_game;
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        pos = dev->pcm_pos_voice;
    } else {
        pos = dev->pcm_pos_voice_out;
    }
    spin_unlock(&dev->lock);

    /* pos is in frames, return position within buffer (also in frames) */
    return pos % runtime->buffer_size;
}

static int zg01_pcm_ioctl(struct snd_pcm_substream *substream,
                          unsigned int cmd, void *arg)
{
    /* Allow all standard ALSA ioctl operations for now */
    pr_debug("zg01_pcm: ioctl 0x%x\n", cmd);
    return snd_pcm_lib_ioctl(substream, cmd, arg);
}

static struct snd_pcm_ops zg01_pcm_ops = {
    .open = zg01_pcm_open,
    .close = zg01_pcm_close,
    .ioctl = zg01_pcm_ioctl,
    .hw_params = zg01_pcm_hw_params,
    .hw_free = zg01_pcm_hw_free,
    .prepare = zg01_pcm_prepare,
    .trigger = zg01_pcm_trigger,
    .pointer = zg01_pcm_pointer,
};

int zg01_create_pcm(struct zg01_dev *dev)
{
    struct zg01_pcm *pcm;
    int ret;
    const char *channel_name;
    int buffer_size;

    // Make sure alternate setting 1 exists and determine channel type
    if (!dev) {
        pr_err("zg01_pcm: zg01_create_pcm called with NULL dev\n");
        return -ENODEV;
    }
    if (!dev->interface || !dev->interface->cur_altsetting) {
        pr_warn("zg01_pcm: No valid USB interface available when creating PCM\n");
        return 0;
    }
    int iface_num = dev->interface->cur_altsetting->desc.bInterfaceNumber;
    if (iface_num == 0 || iface_num > 2 || dev->interface->num_altsetting < 2) {
        return 0;
    }

    /* Channel type should already be set by probe function, but set defaults if not */
    if (dev->channel_type < 0) {
        /* Set channel type based on interface number (legacy fallback) */
        if (iface_num == 1) {
            dev->channel_type = CHANNEL_TYPE_GAME; /* Game channel */
        } else {
            dev->channel_type = CHANNEL_TYPE_VOICE_IN; /* Voice In channel */  
        }
    }
    
    /* Set channel-specific parameters based on channel type */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        channel_name = "Yamaha ZG01 Game PCM";
        buffer_size = PCM_BUFFER_BYTES_MAX_GAME;
        pr_info("zg01_pcm: Creating Game channel (interface %d, type %d)\n", iface_num, dev->channel_type);
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        channel_name = "Yamaha ZG01 Voice In PCM";
        buffer_size = PCM_BUFFER_BYTES_MAX_VOICE;
        pr_info("zg01_pcm: Creating Voice In channel (interface %d, type %d)\n", iface_num, dev->channel_type);
    } else {
        channel_name = "Yamaha ZG01 Voice Out PCM";
        buffer_size = PCM_BUFFER_BYTES_MAX_GAME;
        pr_info("zg01_pcm: Creating Voice Out channel (interface %d, type %d)\n", iface_num, dev->channel_type);
    }

    if (!dev->udev) {
        pr_warn("zg01_pcm: No usb_device available; skipping usb_set_interface\n");
    } else {
        ret = usb_set_interface(dev->udev, iface_num, 1);
        if (ret < 0)
            pr_err("zg01_pcm: Failed to set interface: %d\n", ret);
    }

    pcm = &dev->pcm;
    pcm->zg01 = dev;

    /* Create PCM device with appropriate stream directions */
    if (dev->channel_type == CHANNEL_TYPE_GAME || dev->channel_type == CHANNEL_TYPE_VOICE_OUT) {
        /* Game channel and Voice Out - playback only */
        const char *pcm_name = (dev->channel_type == CHANNEL_TYPE_GAME) ? "ZG01 Game" : "ZG01 Voice Out";
        ret = snd_pcm_new(dev->card, pcm_name, 0, 1, 0, &pcm->instance);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to create playback PCM device (type %d): %d\n", dev->channel_type, ret);
            return ret;
        }
        snd_pcm_set_ops(pcm->instance, SNDRV_PCM_STREAM_PLAYBACK, &zg01_pcm_ops);
        if (dev->channel_type == CHANNEL_TYPE_GAME) {
            pr_info("zg01_pcm: Created Game channel (playback only)\n");
        } else {
            pr_info("zg01_pcm: Created Voice Out channel (playback only)\n");
            buffer_size = PCM_BUFFER_BYTES_MAX_GAME; /* Voice out uses same buffer size as game */
        }
    } else {
        /* Voice In channel - capture only */
        ret = snd_pcm_new(dev->card, "ZG01 Voice In", 0, 0, 1, &pcm->instance);
        if (ret < 0) {
            pr_err("zg01_pcm: Failed to create Voice In PCM device: %d\n", ret);
            return ret;
        }
        snd_pcm_set_ops(pcm->instance, SNDRV_PCM_STREAM_CAPTURE, &zg01_pcm_ops);
        pr_info("zg01_pcm: Created Voice In channel (capture only)\n");
    }

    pcm->instance->private_data = dev;  /* Set to main device structure */
    pcm->instance->private_free = NULL;
    strscpy(pcm->instance->name, channel_name, sizeof(pcm->instance->name));

    snd_pcm_set_managed_buffer_all(pcm->instance,
                                  SNDRV_DMA_TYPE_CONTINUOUS, NULL,
                                  buffer_size, buffer_size);

    /* Initialize deferred start work and pending flags */
    INIT_DELAYED_WORK(&dev->start_work_game, zg01_pcm_start_work);
    INIT_DELAYED_WORK(&dev->start_work_voice, zg01_pcm_start_work);
    INIT_DELAYED_WORK(&dev->start_work_voice_out, zg01_pcm_start_work);
    dev->start_pending_game = false;
    dev->start_pending_voice = false;
    dev->start_pending_voice_out = false;

    return 0;
}

EXPORT_SYMBOL_GPL(zg01_create_pcm);

MODULE_AUTHOR("Your Name");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver - PCM Interface");
MODULE_LICENSE("GPL");
