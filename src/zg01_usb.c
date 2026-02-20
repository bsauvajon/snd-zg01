#include <linux/module.h>
#include <linux/usb.h>
#include <linux/slab.h>
#include <linux/workqueue.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include "zg01.h"
#include "zg01_pcm.h"
#include "zg01_control.h"

static DEFINE_MUTEX(devices_mutex);

/* Global device pointers — also declared extern in zg01.h for zg01_pcm.c */
struct zg01_dev *game_dev;
struct zg01_dev *voice_in_dev;
struct zg01_dev *voice_out_dev;

/* Private workqueue — avoids flush_workqueue(system_wq) warning */
struct workqueue_struct *zg01_wq;

/*
 * zg01_create_one_card - allocate and register a single ALSA card for one
 * channel type.  Called by zg01_probe; devices_mutex must NOT be held on
 * entry (snd_card_new / snd_card_register may sleep).
 *
 * On success the global device pointer (game_dev / voice_in_dev /
 * voice_out_dev) is set and the card is live.  On error everything is
 * cleaned up before returning.
 */
static int zg01_create_one_card(struct usb_interface *interface,
                                const struct usb_device_id *id,
                                int channel_type)
{
    const char *card_id;
    struct snd_card *card;
    struct zg01_dev *dev;
    int err;

    /* Select ALSA card short-id */
    if (channel_type == CHANNEL_TYPE_GAME)
        card_id = "zg01game";
    else if (channel_type == CHANNEL_TYPE_VOICE_IN)
        card_id = "zg01voice";
    else
        card_id = "zg01voiceout";

    /* Create card with embedded zg01_dev structure */
    err = snd_card_new(&interface->dev, -1, card_id, THIS_MODULE,
                       sizeof(struct zg01_dev), &card);
    if (err) {
        dev_err(&interface->dev, "ZG01: Failed to create sound card: %d\n", err);
        return err;
    }

    /* Ensure module owner is set — try_module_get() needs a valid pointer */
    card->module = THIS_MODULE;

    dev = card->private_data;
    dev->card = card;
    dev->channel_type = channel_type;
    dev->udev = usb_get_dev(interface_to_usbdev(interface));
    dev->interface = interface;

    spin_lock_init(&dev->lock);
    mutex_init(&dev->pcm_mutex);

    dev->game_channel_active = false;
    dev->voice_channel_active = false;
    dev->voice_out_channel_active = false;
    dev->game_initialized = false;
    dev->voice_initialized = false;
    dev->voice_out_initialized = false;
    dev->cleanup_in_progress_game = false;
    dev->cleanup_in_progress_voice = false;
    dev->cleanup_in_progress_voice_out = false;

    snd_card_set_dev(card, &interface->dev);
    strncpy(card->driver, "zg01_usb", sizeof(card->driver));

    if (channel_type == CHANNEL_TYPE_GAME) {
        strncpy(card->shortname,  "ZG01 Game",                    sizeof(card->shortname));
        strncpy(card->longname,   "Yamaha ZG01 Game Channel",     sizeof(card->longname));
        strncpy(card->mixername,  "ZG01 Game",                    sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-Game",            sizeof(card->components));
        dev_info(&interface->dev, "ZG01: Creating Game channel\n");
    } else if (channel_type == CHANNEL_TYPE_VOICE_IN) {
        strncpy(card->shortname,  "ZG01 Voice In",                      sizeof(card->shortname));
        strncpy(card->longname,   "Yamaha ZG01 Voice Input Channel",    sizeof(card->longname));
        strncpy(card->mixername,  "ZG01 Voice In",                      sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-VoiceIn",               sizeof(card->components));
        dev_info(&interface->dev, "ZG01: Creating Voice In channel\n");
    } else {
        strncpy(card->shortname,  "ZG01 Voice Out",                      sizeof(card->shortname));
        strncpy(card->longname,   "Yamaha ZG01 Voice Output Channel",   sizeof(card->longname));
        strncpy(card->mixername,  "ZG01 Voice Out",                      sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-VoiceOut",               sizeof(card->components));
        dev_info(&interface->dev, "ZG01: Creating Voice Out channel\n");
    }

    err = zg01_init_control(dev);
    if (err) {
        dev_err(&interface->dev, "ZG01: Failed to init control: %d\n", err);
        goto err_free_card;
    }

    /* USB discovery is best-effort; failures are non-fatal */
    if (zg01_discover_usb_config(dev))
        pr_warn("zg01_usb: USB discovery failed, continuing\n");

    /* Reset streaming interfaces to alt 0 before PCM registration */
    if (usb_set_interface(dev->udev, 1, 0))
        dev_err(&interface->dev, "ZG01: Failed to set iface 1 alt 0\n");
    if (usb_set_interface(dev->udev, 2, 0))
        dev_err(&interface->dev, "ZG01: Failed to set iface 2 alt 0\n");

    err = zg01_create_pcm(dev);
    if (err) {
        dev_err(&interface->dev, "ZG01: Failed to create PCM: %d\n", err);
        goto err_free_card;
    }

    err = snd_card_register(card);
    if (err) {
        dev_err(&interface->dev, "ZG01: Failed to register card: %d\n", err);
        goto err_free_card;
    }

    /*
     * Global pointer is set only after successful registration (USB-6).
     * Protected by devices_mutex in the caller (probe / disconnect).
     */
    mutex_lock(&devices_mutex);
    if (channel_type == CHANNEL_TYPE_GAME)
        game_dev = dev;
    else if (channel_type == CHANNEL_TYPE_VOICE_IN)
        voice_in_dev = dev;
    else
        voice_out_dev = dev;
    mutex_unlock(&devices_mutex);

    /*
     * Store dev in interface data.  For interface 1 this is called twice
     * (Game then Voice Out); the second call overwrites the first.
     * Disconnect retrieves both via the global pointers under devices_mutex
     * (USB-7).
     */
    usb_set_intfdata(interface, dev);

    return 0;

err_free_card:
    usb_put_dev(dev->udev);
    snd_card_free(card);
    return err;
}

static int zg01_probe(struct usb_interface *interface,
                      const struct usb_device_id *id)
{
    int iface_num;
    int err;

    iface_num = interface->cur_altsetting->desc.bInterfaceNumber;

    if (iface_num != 1 && iface_num != 2) {
        dev_info(&interface->dev, "ZG01: Skipping interface %d\n", iface_num);
        return 0;
    }

    if (iface_num == 1) {
        /*
         * Interface 1 hosts both Game (playback) and Voice Out (playback).
         * Create them in sequence — no recursion (USB-8).
         */
        mutex_lock(&devices_mutex);
        if (game_dev && voice_out_dev) {
            /* Both already created, nothing to do */
            mutex_unlock(&devices_mutex);
            return 0;
        }
        mutex_unlock(&devices_mutex);

        if (!game_dev) {
            err = zg01_create_one_card(interface, id, CHANNEL_TYPE_GAME);
            if (err)
                return err;
        }

        if (!voice_out_dev) {
            err = zg01_create_one_card(interface, id, CHANNEL_TYPE_VOICE_OUT);
            if (err)
                return err;
        }
    } else {
        /* Interface 2 — Voice In capture */
        mutex_lock(&devices_mutex);
        if (voice_in_dev) {
            mutex_unlock(&devices_mutex);
            return 0;
        }
        mutex_unlock(&devices_mutex);

        err = zg01_create_one_card(interface, id, CHANNEL_TYPE_VOICE_IN);
        if (err)
            return err;
    }

    return 0;
}

/*
 * zg01_disconnect_one - kill URBs, free buffers, free card for a single dev.
 * May be called for Game, Voice Out, or Voice In.
 */
static void zg01_disconnect_one(struct zg01_dev *dev)
{
    struct urb **iso_urbs;
    unsigned char **iso_buffers;
    int i;

    if (!dev)
        return;

    /*
     * Flush any pending deferred cleanup work before we inline-free
     * the buffers, to avoid double-free (PCM-22).
     */
    flush_workqueue(zg01_wq);

    /* Kill + free URBs and their transfer buffers (USB-9) */
    if (dev->channel_type == CHANNEL_TYPE_GAME) {
        iso_urbs    = dev->iso_urbs_game;
        iso_buffers = dev->iso_buffers_game;
    } else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN) {
        iso_urbs    = dev->iso_urbs_voice;
        iso_buffers = dev->iso_buffers_voice;
    } else {
        iso_urbs    = dev->iso_urbs_voice_out;
        iso_buffers = dev->iso_buffers_voice_out;
    }

    for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
        if (iso_urbs[i]) {
            usb_kill_urb(iso_urbs[i]);
            usb_free_urb(iso_urbs[i]);
            iso_urbs[i] = NULL;
        }
        if (iso_buffers[i]) {
            kfree(iso_buffers[i]);
            iso_buffers[i] = NULL;
        }
    }

    /* Clear global pointer under mutex (USB-3) */
    mutex_lock(&devices_mutex);
    if (dev == game_dev)
        game_dev = NULL;
    else if (dev == voice_in_dev)
        voice_in_dev = NULL;
    else if (dev == voice_out_dev)
        voice_out_dev = NULL;
    mutex_unlock(&devices_mutex);

    /* Free the card — also frees the embedded dev structure */
    if (dev->card)
        snd_card_free(dev->card);
}

static void zg01_disconnect(struct usb_interface *interface)
{
    int iface_num = interface->cur_altsetting->desc.bInterfaceNumber;
    struct zg01_dev *dev = usb_get_intfdata(interface);

    usb_set_intfdata(interface, NULL);

    if (iface_num == 1) {
        /*
         * Interface 1 owns Game and Voice Out.  usb_set_intfdata only stores
         * one pointer (the last one written by zg01_create_one_card, which is
         * Voice Out).  Retrieve both via the global pointers (USB-7).
         */
        struct zg01_dev *gdev;
        struct zg01_dev *vodev;

        mutex_lock(&devices_mutex);
        gdev  = game_dev;
        vodev = voice_out_dev;
        mutex_unlock(&devices_mutex);

        zg01_disconnect_one(gdev);
        zg01_disconnect_one(vodev);
    } else {
        /* Interface 2 — Voice In; intfdata is the correct dev */
        zg01_disconnect_one(dev);
    }

    dev_info(&interface->dev, "Yamaha ZG01 device disconnected\n");
}

int zg01_set_streaming_interface(struct zg01_dev *dev, int interface,
                                 int alt_setting)
{
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;

    ret = usb_set_interface(dev->udev, interface, alt_setting);
    if (ret) {
        dev_err(&dev->udev->dev,
                "ZG01: Failed to set interface %d alt %d: %d\n",
                interface, alt_setting, ret);
        return ret;
    }

    dev_dbg(&dev->udev->dev, "ZG01: Set interface %d to alt %d\n",
            interface, alt_setting);
    return 0;
}

static struct usb_device_id zg01_table[] = {
    { USB_DEVICE(VENDOR_ID_YAMAHA, PRODUCT_ID_ZG01) },
    {}
};
MODULE_DEVICE_TABLE(usb, zg01_table);

static struct usb_driver zg01_driver = {
    .name      = "zg01_usb",
    .id_table  = zg01_table,
    .probe     = zg01_probe,
    .disconnect = zg01_disconnect,
};

static int __init zg01_init(void)
{
    zg01_wq = alloc_ordered_workqueue("zg01", WQ_MEM_RECLAIM);
    if (!zg01_wq)
        return -ENOMEM;
    return usb_register(&zg01_driver);
}

static void __exit zg01_exit(void)
{
    usb_deregister(&zg01_driver);
    destroy_workqueue(zg01_wq);
}

module_init(zg01_init);
module_exit(zg01_exit);

MODULE_AUTHOR("Yamaha ZG01 Driver Contributors");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver");
MODULE_LICENSE("GPL");
