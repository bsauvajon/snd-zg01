#include <linux/module.h>
#include <linux/usb.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/workqueue.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include "zg01.h"
#include "zg01_pcm.h"
#include "zg01_control.h"


DEFINE_MUTEX(devices_mutex);  /* Non-static: accessed from zg01_pcm.c */

/* Global device pointers — also declared extern in zg01.h for zg01_pcm.c */
struct zg01_dev *game_dev;
struct zg01_dev *voice_in_dev;
struct zg01_dev *voice_out_dev;

/* Keep cleanup independent from period notifications that call back into ALSA. */
struct workqueue_struct *zg01_cleanup_wq;
struct workqueue_struct *zg01_period_wq;

static void zg01_card_private_free(struct snd_card *card)
{
    struct zg01_dev *dev = card->private_data;

    if (dev->udev) {
        usb_put_dev(dev->udev);
        dev->udev = NULL;
    }
}
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
    card->private_free = zg01_card_private_free;
    dev->interface = interface;

    spin_lock_init(&dev->lock);
    mutex_init(&dev->pcm_mutex);
    mutex_init(&dev->chain_mutex);

    dev->game_channel_active = false;
    dev->voice_channel_active = false;
    dev->voice_out_channel_active = false;
    dev->game_initialized = false;
    dev->voice_initialized = false;
    dev->voice_out_initialized = false;
    dev->cleanup_in_progress_game = false;
    dev->cleanup_in_progress_voice = false;
    dev->cleanup_in_progress_voice_out = false;
    atomic_set(&dev->disconnecting, 0);
    atomic_set(&dev->active_urbs_game, 0);
    atomic_set(&dev->active_urbs_voice, 0);
    atomic_set(&dev->active_urbs_voice_out, 0);
    atomic_set(&dev->voice_out_mixing, 0);
    dev->vo_mix_dev = NULL;
    dev->voice_out_piggybacking = false;
    dev->piggyback_host = NULL;

    /* Initialize embedded cleanup work structs - prevents GFP_ATOMIC allocation failures */
    INIT_WORK(&dev->cleanup_work_game, zg01_cleanup_work_game_fn);
    INIT_WORK(&dev->cleanup_work_voice, zg01_cleanup_work_voice_fn);
    INIT_WORK(&dev->cleanup_work_voice_out, zg01_cleanup_work_voice_out_fn);

    /* Initialize deferred period-elapsed work structs */
    INIT_WORK(&dev->period_work_game, zg01_period_work_game_fn);
    INIT_WORK(&dev->period_work_voice, zg01_period_work_voice_fn);
    INIT_WORK(&dev->period_work_voice_out, zg01_period_work_voice_out_fn);

    snd_card_set_dev(card, &interface->dev);
    strscpy(card->driver, "zg01_usb", sizeof(card->driver));

    if (channel_type == CHANNEL_TYPE_GAME) {
        strscpy(card->shortname, "ZG01 Game",                    sizeof(card->shortname));
        strscpy(card->longname, "Yamaha ZG01 Game Channel",     sizeof(card->longname));
        strscpy(card->mixername, "ZG01 Game",                    sizeof(card->mixername));
        snd_component_add(card, "USB0499:1513-Game");
        dev_info(&interface->dev, "ZG01: Creating Game channel\n");
    } else if (channel_type == CHANNEL_TYPE_VOICE_IN) {
        strscpy(card->shortname, "ZG01 Voice In",                      sizeof(card->shortname));
        strscpy(card->longname, "Yamaha ZG01 Voice Input Channel",    sizeof(card->longname));
        strscpy(card->mixername, "ZG01 Voice In",                      sizeof(card->mixername));
        snd_component_add(card, "USB0499:1513-VoiceIn");
        dev_info(&interface->dev, "ZG01: Creating Voice In channel\n");
    } else {
        strscpy(card->shortname, "ZG01 Voice Out",                      sizeof(card->shortname));
        strscpy(card->longname, "Yamaha ZG01 Voice Output Channel",   sizeof(card->longname));
        strscpy(card->mixername, "ZG01 Voice Out",                      sizeof(card->mixername));
        snd_component_add(card, "USB0499:1513-VoiceOut");
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
        bool need_game, need_voice_out;

        mutex_lock(&devices_mutex);
        need_game      = (game_dev == NULL);
        need_voice_out = (voice_out_dev == NULL);
        mutex_unlock(&devices_mutex);

        if (!need_game && !need_voice_out)
            return 0;

        if (need_game) {
            err = zg01_create_one_card(interface, id, CHANNEL_TYPE_GAME);
            if (err)
                return err;
        }

        if (need_voice_out) {
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

    /* Set disconnecting flag to prevent URB resubmission (USB-10) */
    atomic_set(&dev->disconnecting, 1);

    /* Stop URB cleanup first, then drain any period notifications queued by
     * those URBs before freeing the buffers and card. */
    flush_workqueue(zg01_cleanup_wq);
    flush_workqueue(zg01_period_wq);

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

    /*
     * Use asynchronous disconnect pattern to prevent blocking USB hub thread.
     * snd_card_disconnect() notifies userspace and prevents new opens.
     * snd_card_free_when_closed() defers actual free until all file descriptors close.
     * This prevents hanging the entire USB subsystem if PipeWire/apps still have the device open.
     */
    if (dev->card) {
        snd_card_disconnect(dev->card);
        snd_card_free_when_closed(dev->card);
    }

    /* USB device reference is released by ALSA when card is freed (via private_free) */
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

/*
 * Suspend one dev: quiesce its PCM, then stop and drain its URB chain.
 *
 * snd_pcm_suspend_all() moves open streams to SNDRV_PCM_STATE_SUSPENDED
 * (-ESTRPIPE) instead of letting them die mid-transfer.
 *
 * The explicit zg01_stop_streaming() + flush_work() matters: during
 * system suspend the USB core flushes all endpoints, and our ISO
 * callback exits on -ESHUTDOWN/-ENOENT WITHOUT resubmitting or queueing
 * cleanup work. Without this drain, active_urbs_* stay > 0 forever,
 * which makes the next prepare() skip the Magic Sequence (any-channel-
 * streaming early path) and the next TRIGGER_START skip URB submission
 * ("Streaming already active"). PM context may sleep, so flush is safe.
 */
static void zg01_suspend_one(struct zg01_dev *dev)
{
    if (!dev)
        return;

    if (dev->pcm.instance)
        snd_pcm_suspend_all(dev->pcm.instance);

    zg01_stop_streaming(dev);

    if (dev->channel_type == CHANNEL_TYPE_GAME)
        flush_work(&dev->cleanup_work_game);
    else if (dev->channel_type == CHANNEL_TYPE_VOICE_IN)
        flush_work(&dev->cleanup_work_voice);
    else
        flush_work(&dev->cleanup_work_voice_out);
}

/* Grab all three devs under devices_mutex; NULL entries allowed. */
static void zg01_get_all_devs(struct zg01_dev **gdev,
                              struct zg01_dev **vin,
                              struct zg01_dev **vout)
{
    mutex_lock(&devices_mutex);
    *gdev = game_dev;
    *vin = voice_in_dev;
    *vout = voice_out_dev;
    mutex_unlock(&devices_mutex);
}

static int zg01_suspend(struct usb_interface *intf, pm_message_t message)
{
    struct zg01_dev *gdev, *vin, *vout;

    /*
     * PM callbacks arrive per interface, but usb_get_intfdata() returns
     * only ONE dev per interface (probe registers two cards — Game and
     * Voice Out — on interface 1, and intfdata keeps just the last).
     * Suspend all three devs explicitly so Game is not missed.
     */
    zg01_get_all_devs(&gdev, &vin, &vout);
    /* Voice Out BEFORE Game: if the Game chain is in keepalive for
     * Voice Out, VO's stop tears the orphaned chain down and queues
     * Game cleanup work, which the Game suspend below then flushes. */
    zg01_suspend_one(vout);
    zg01_suspend_one(gdev);
    zg01_suspend_one(vin);

    return 0;
}

/* Reset per-channel streaming bookkeeping on one dev. */
static void zg01_pm_reset_dev(struct zg01_dev *dev)
{
    if (!dev)
        return;

    atomic_set(&dev->disconnecting, 0);
    dev->game_channel_active = false;
    dev->voice_channel_active = false;
    dev->voice_out_channel_active = false;
    /* Firmware restarted: force first-prepare path (Magic Sequence) on
     * the next open() of each channel. */
    dev->game_initialized = false;
    dev->voice_initialized = false;
    dev->voice_out_initialized = false;
}

static int zg01_resume(struct usb_interface *intf)
{
    struct zg01_dev *gdev, *vin, *vout;

    zg01_get_all_devs(&gdev, &vin, &vout);
    zg01_pm_reset_dev(gdev);
    zg01_pm_reset_dev(vin);
    zg01_pm_reset_dev(vout);

    return 0;
}

/*
 * reset_resume: the ZG01 firmware resets itself across suspend, so the
 * USB core would normally treat resume as unplug+replug — disconnect(),
 * then probe() building a NEW set of cards while the old ones linger in
 * snd_card_free_when_closed() as zombies. That is the source of the
 * duplicate ALSA/PipeWire devices after sleep.
 *
 * With reset_resume registered, the core keeps our interfaces bound and
 * preserves the same ALSA card objects. No disconnect, no probe, no new
 * cards — userspace sees the same nodes it had before suspend.
 *
 * The callback fires once per interface (serialized by the USB core PM
 * lock, and userspace is still frozen during device resume, so no ALSA
 * ioctls can race). Per-dev state is reset for ALL three devs; the
 * device-global reinit (vendor handshake + interfaces to alt 0) runs
 * exactly once, from interface 2's callback.
 */
static int zg01_reset_resume(struct usb_interface *intf)
{
    struct zg01_dev *gdev, *vin, *vout, *any;
    int iface_num = intf->cur_altsetting->desc.bInterfaceNumber;

    zg01_get_all_devs(&gdev, &vin, &vout);
    zg01_pm_reset_dev(gdev);
    zg01_pm_reset_dev(vin);
    zg01_pm_reset_dev(vout);

    if (iface_num != 2)
        return 0; /* device-global reinit handled from interface 2 */

    any = vin ? vin : (gdev ? gdev : vout);
    if (any) {
        zg01_init_control(any);
        usb_set_interface(any->udev, 1, 0);
        usb_set_interface(any->udev, 2, 0);
    }

    return 0;
}

static struct usb_driver zg01_driver = {
    .name          = "zg01_usb",
    .id_table      = zg01_table,
    .probe         = zg01_probe,
    .disconnect    = zg01_disconnect,
    .suspend       = zg01_suspend,
    .resume        = zg01_resume,
    .reset_resume  = zg01_reset_resume,
};

static int __init zg01_init(void)
{
    int ret;

    zg01_cleanup_wq = alloc_ordered_workqueue("zg01-cleanup", WQ_MEM_RECLAIM);
    if (!zg01_cleanup_wq)
        return -ENOMEM;

    zg01_period_wq = alloc_ordered_workqueue("zg01-period", WQ_MEM_RECLAIM);
    if (!zg01_period_wq) {
        destroy_workqueue(zg01_cleanup_wq);
        zg01_cleanup_wq = NULL;
        return -ENOMEM;
    }

    ret = usb_register(&zg01_driver);
    if (ret) {
        destroy_workqueue(zg01_period_wq);
        destroy_workqueue(zg01_cleanup_wq);
        zg01_period_wq = NULL;
        zg01_cleanup_wq = NULL;
        return ret;
    }

    return 0;
}

static void __exit zg01_exit(void)
{
    usb_deregister(&zg01_driver);
    destroy_workqueue(zg01_period_wq);
    destroy_workqueue(zg01_cleanup_wq);
}

module_init(zg01_init);
module_exit(zg01_exit);

MODULE_AUTHOR("Yamaha ZG01 Driver Contributors");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver");
MODULE_LICENSE("GPL");
